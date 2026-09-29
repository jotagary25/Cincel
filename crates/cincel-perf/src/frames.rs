//! Frame analysis, independent of Wayland so it can be tested on synthetic
//! images: "first frame with content" and "first stable frame" (§3.2).

/// A rectangle in output pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub fn intersects(&self, other: &Rect) -> bool {
        self.width > 0
            && self.height > 0
            && other.width > 0
            && other.height > 0
            && self.x < other.x + other.width
            && other.x < self.x + self.width
            && self.y < other.y + other.height
            && other.y < self.y + self.height
    }
}

/// One captured frame: its presentation time and the damaged area.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// `CLOCK_MONOTONIC`, nanoseconds.
    pub ts_ns: u64,
    pub damage: Vec<Rect>,
}

impl Frame {
    /// Whether the damage touches `region` (an empty damage list means the
    /// compositor did not report any, which counts as the whole output).
    pub fn damages(&self, region: &Rect) -> bool {
        self.damage.is_empty() || self.damage.iter().any(|rect| rect.intersects(region))
    }
}

/// A 32-bit-per-pixel image (the byte order does not matter: the alpha or
/// padding byte, the fourth of each pixel, is ignored).
#[derive(Debug, Clone, Copy)]
pub struct Image<'a> {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub data: &'a [u8],
}

/// Per-channel tolerance when comparing a pixel with the background: only
/// rounding noise is ignored. Panels a few shades off the background (dark
/// themes use #181818 next to #1f1f1f) do count as different; the color
/// rule below keeps such flat skeletons from counting as content.
pub const CHANNEL_TOLERANCE: u8 = 3;
/// Fraction of pixels that must differ from the background (§3.2).
pub const CONTENT_FRACTION: f64 = 0.02;
/// Sampling step, in pixels, in both axes (a 1920x1080 frame gives ~130 000
/// samples, far more than the 2 % threshold needs).
pub const SAMPLE_STEP: u32 = 4;
/// Distinct colors a frame needs to count as content: text and icons are
/// antialiased and bring dozens of shades, while the flat placeholders some
/// apps show first (a blank rectangle while Electron resizes) bring a few.
pub const CONTENT_COLORS: usize = 16;

impl Image<'_> {
    fn pixel(&self, x: u32, y: u32) -> Option<[u8; 3]> {
        let offset = (y * self.stride + x * 4) as usize;
        let bytes = self.data.get(offset..offset + 3)?;
        Some([bytes[0], bytes[1], bytes[2]])
    }

    fn samples<'s>(&'s self, region: &'s Rect) -> impl Iterator<Item = [u8; 3]> + 's {
        let x0 = region.x.max(0) as u32;
        let y0 = region.y.max(0) as u32;
        let x1 = ((region.x + region.width).max(0) as u32).min(self.width);
        let y1 = ((region.y + region.height).max(0) as u32).min(self.height);
        (y0..y1)
            .step_by(SAMPLE_STEP as usize)
            .flat_map(move |y| (x0..x1).step_by(SAMPLE_STEP as usize).map(move |x| (x, y)))
            .filter_map(|(x, y)| self.pixel(x, y))
    }

    /// The most frequent color of `region` (the background of a frame).
    pub fn dominant_color(&self, region: &Rect) -> Option<[u8; 3]> {
        let mut counts = std::collections::HashMap::new();
        for color in self.samples(region) {
            *counts.entry(color).or_insert(0u32) += 1;
        }
        counts
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
            .map(|(color, _)| color)
    }

    /// Number of distinct colors among the sampled pixels of `region`.
    pub fn distinct_colors(&self, region: &Rect) -> usize {
        self.samples(region)
            .collect::<std::collections::HashSet<_>>()
            .len()
    }

    /// Fraction of sampled pixels of `region` that differ from `background`.
    pub fn content_fraction(&self, region: &Rect, background: [u8; 3]) -> f64 {
        let mut total = 0u64;
        let mut different = 0u64;
        for color in self.samples(region) {
            total += 1;
            if color
                .iter()
                .zip(background)
                .any(|(a, b)| a.abs_diff(b) > CHANNEL_TOLERANCE)
            {
                different += 1;
            }
        }
        if total == 0 {
            0.0
        } else {
            different as f64 / total as f64
        }
    }
}

/// Finds the first frame with content (§3.2). Frames that still show the
/// empty desktop (the window is mapped but not drawn yet) are skipped; the
/// background is the dominant color of the first frame that shows the
/// window, and a frame (that one included) has content when at least
/// [`CONTENT_FRACTION`] of the window differs from that background and it has
/// at least [`CONTENT_COLORS`] distinct colors. Feed frames in order.
#[derive(Debug, Default)]
pub struct ContentDetector {
    desktop: Option<[u8; 3]>,
    background: Option<[u8; 3]>,
}

/// What [`ContentDetector::feed`] saw in a frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Verdict {
    /// The frame shows the window (not only the empty desktop).
    pub drawn: bool,
    /// Fraction of the window that differs from the background.
    pub fraction: f64,
    pub colors: usize,
    pub content: bool,
}

impl ContentDetector {
    /// `desktop`: color of the empty desktop before the app started, if known.
    pub fn new(desktop: Option<[u8; 3]>) -> Self {
        ContentDetector {
            desktop,
            background: None,
        }
    }

    pub fn background(&self) -> Option<[u8; 3]> {
        self.background
    }

    pub fn feed(&mut self, image: &Image<'_>, region: &Rect) -> Verdict {
        let not_drawn = Verdict {
            drawn: false,
            fraction: 0.0,
            colors: 0,
            content: false,
        };
        let background = match self.background {
            Some(background) => background,
            None => {
                if let Some(desktop) = self.desktop
                    && image.content_fraction(region, desktop) < CONTENT_FRACTION
                {
                    return not_drawn;
                }
                let Some(color) = image.dominant_color(region) else {
                    return not_drawn;
                };
                self.background = Some(color);
                color
            }
        };
        let fraction = image.content_fraction(region, background);
        let colors = image.distinct_colors(region);
        Verdict {
            drawn: true,
            fraction,
            colors,
            content: fraction >= CONTENT_FRACTION && colors >= CONTENT_COLORS,
        }
    }
}

/// The first stable frame after `after_ns`: the first frame (damaging
/// `region`) after which no damage touches `region` for `quiet_ns`. `end_ns`
/// is when the observation stopped; a frame whose quiet window reaches past it
/// is not yet known to be stable. Frames must be sorted by time.
pub fn stable_frame<'a>(
    frames: &'a [Frame],
    region: &Rect,
    after_ns: u64,
    quiet_ns: u64,
    end_ns: u64,
) -> Option<&'a Frame> {
    let relevant: Vec<&Frame> = frames
        .iter()
        .filter(|frame| frame.ts_ns > after_ns && frame.damages(region))
        .collect();
    for (index, frame) in relevant.iter().enumerate() {
        let next = relevant.get(index + 1).map(|next| next.ts_ns);
        let quiet_until = frame.ts_ns + quiet_ns;
        match next {
            Some(next) if next < quiet_until => continue,
            Some(_) => return Some(frame),
            None if quiet_until <= end_ns => return Some(frame),
            None => return None,
        }
    }
    None
}

/// First frame damaging `region` after `after_ns`.
pub fn first_damage_after<'a>(
    frames: &'a [Frame],
    region: &Rect,
    after_ns: u64,
) -> Option<&'a Frame> {
    frames
        .iter()
        .find(|frame| frame.ts_ns > after_ns && frame.damages(region))
}

/// Intervals, in milliseconds, between consecutive frames damaging `region`
/// inside `[from_ns, to_ns]`.
pub fn intervals_ms(frames: &[Frame], region: &Rect, from_ns: u64, to_ns: u64) -> Vec<f64> {
    let times: Vec<u64> = frames
        .iter()
        .filter(|frame| frame.ts_ns >= from_ns && frame.ts_ns <= to_ns && frame.damages(region))
        .map(|frame| frame.ts_ns)
        .collect();
    times
        .windows(2)
        .map(|pair| (pair[1] - pair[0]) as f64 / 1e6)
        .collect()
}
