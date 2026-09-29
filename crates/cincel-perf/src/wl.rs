//! Wayland client of the bench (D1): window detection
//! (`wlr-foreign-toplevel-management` or `ext-foreign-toplevel-list`),
//! damage-driven frame capture (`wlr-screencopy`, `copy_with_damage`) and
//! input injection (`zwp_virtual_keyboard_v1`, `zwlr_virtual_pointer_v1`).
//!
//! Captures only ever target the compositor the bench connects to, which is
//! the invisible desktop of the container.

use std::collections::HashMap;
use std::fs::File;
use std::io::Write as _;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rustix::event::{PollFd, PollFlags, poll};
use rustix::fs::MemfdFlags;
use wayland_client::backend::ObjectId;
use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer::WlBuffer, wl_output, wl_output::WlOutput, wl_pointer, wl_registry::WlRegistry,
    wl_seat::WlSeat, wl_shm, wl_shm::WlShm, wl_shm_pool::WlShmPool,
};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop, event_created_child,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
    ext_foreign_toplevel_handle_v1::{self, ExtForeignToplevelHandleV1},
    ext_foreign_toplevel_list_v1::{self, ExtForeignToplevelListV1},
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

use crate::clock::now_ns;
use crate::frames::{ContentDetector, Frame, Image, Rect};
use crate::keys::{KEYMAP, Stroke};

/// Which window-detection protocol to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToplevelProtocol {
    Wlr,
    Ext,
}

/// A window announced by the compositor.
#[derive(Debug, Clone, Default)]
pub struct Toplevel {
    pub app_id: String,
    pub title: String,
    /// Client time of its first `done` event.
    pub done_ns: Option<u64>,
    /// Created after [`Session::arm_toplevels`].
    pub new: bool,
    pub closed: bool,
}

/// Where the frame timestamps come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockSource {
    /// `ready` of the screencopy frame (compositor, `CLOCK_MONOTONIC`).
    Compositor,
    /// The compositor clock did not match ours: time of reception.
    Client,
}

impl ClockSource {
    pub fn name(self) -> &'static str {
        match self {
            ClockSource::Compositor => "compositor",
            ClockSource::Client => "client",
        }
    }
}

struct ShmBuffer {
    file: File,
    pool: WlShmPool,
    buffer: WlBuffer,
    width: u32,
    height: u32,
    stride: u32,
    format: u32,
}

#[derive(Default)]
struct Slot {
    frame: Option<ZwlrScreencopyFrameV1>,
    buffer: Option<ShmBuffer>,
    pending: Option<(u32, u32, u32, u32)>,
    damage: Vec<Rect>,
    /// One-shot `copy` (no damage wait) of the empty desktop.
    plain: bool,
}

/// State of the content analysis done while frames arrive.
struct ContentProbe {
    detector: ContentDetector,
    first_frame_ns: Option<u64>,
    first_content_ns: Option<u64>,
    last_fraction: f64,
    last_colors: usize,
}

pub struct State {
    qh: QueueHandle<State>,
    output: Option<WlOutput>,
    output_size: Option<(i32, i32)>,
    shm: Option<WlShm>,
    screencopy: Option<ZwlrScreencopyManagerV1>,
    screencopy_version: u32,
    slots: Vec<Slot>,
    capturing: bool,
    pub frames: Vec<Frame>,
    pub capture_failures: u32,
    clock: Option<ClockSource>,
    content: Option<ContentProbe>,
    pixels: Vec<u8>,
    toplevels: HashMap<ObjectId, Toplevel>,
    armed: bool,
    desktop_color: Option<[u8; 3]>,
    desktop_done: bool,
}

/// One connection to the compositor.
pub struct Session {
    queue: EventQueue<State>,
    pub state: State,
    globals: GlobalList,
    wlr_toplevels: Option<ZwlrForeignToplevelManagerV1>,
    ext_toplevels: Option<ExtForeignToplevelListV1>,
    seat: Option<WlSeat>,
    keyboard: Option<ZwpVirtualKeyboardV1>,
    pointer: Option<ZwlrVirtualPointerV1>,
    modifiers: u32,
}

/// Interfaces a probe reports on.
pub const PROBED: &[&str] = &[
    "zwlr_screencopy_manager_v1",
    "ext_image_copy_capture_manager_v1",
    "zwlr_foreign_toplevel_manager_v1",
    "ext_foreign_toplevel_list_v1",
    "zwp_virtual_keyboard_manager_v1",
    "zwlr_virtual_pointer_manager_v1",
];

impl Session {
    /// Connects to `$WAYLAND_DISPLAY` and binds the output, shm and seat.
    pub fn connect() -> Result<Self> {
        let conn = Connection::connect_to_env()
            .context("no se pudo conectar al compositor Wayland ($WAYLAND_DISPLAY)")?;
        let (globals, queue) =
            registry_queue_init::<State>(&conn).context("no se pudo leer el registro Wayland")?;
        let qh = queue.handle();
        let output: Option<WlOutput> = globals.bind(&qh, 1..=4, ()).ok();
        let shm: Option<WlShm> = globals.bind(&qh, 1..=1, ()).ok();
        let seat: Option<WlSeat> = globals.bind(&qh, 1..=7, ()).ok();
        let state = State {
            qh,
            output,
            output_size: None,
            shm,
            screencopy: None,
            screencopy_version: 0,
            slots: Vec::new(),
            capturing: false,
            frames: Vec::new(),
            capture_failures: 0,
            clock: None,
            content: None,
            pixels: Vec::new(),
            toplevels: HashMap::new(),
            armed: false,
            desktop_color: None,
            desktop_done: false,
        };
        let mut session = Session {
            queue,
            state,
            globals,
            wlr_toplevels: None,
            ext_toplevels: None,
            seat,
            keyboard: None,
            pointer: None,
            modifiers: 0,
        };
        session.roundtrip()?;
        Ok(session)
    }

    /// Interface names and versions the compositor advertises.
    pub fn globals(&self) -> Vec<(String, u32)> {
        self.globals.contents().with_list(|list| {
            list.iter()
                .map(|global| (global.interface.clone(), global.version))
                .collect()
        })
    }

    pub fn roundtrip(&mut self) -> Result<()> {
        self.queue
            .roundtrip(&mut self.state)
            .context("se cortó la conexión con el compositor")?;
        Ok(())
    }

    /// Dispatches events for up to `timeout` (returns earlier if something
    /// arrived).
    pub fn dispatch_for(&mut self, timeout: Duration) -> Result<()> {
        self.queue.flush().context("flush Wayland")?;
        self.queue.dispatch_pending(&mut self.state)?;
        if let Some(guard) = self.queue.prepare_read() {
            let fd = guard.connection_fd();
            let mut fds = [PollFd::new(&fd, PollFlags::IN)];
            let spec = rustix::time::Timespec {
                tv_sec: timeout.as_secs() as i64,
                tv_nsec: i64::from(timeout.subsec_nanos()),
            };
            let ready = match poll(&mut fds, Some(&spec)) {
                Ok(ready) => ready,
                Err(rustix::io::Errno::INTR) => 0,
                Err(error) => return Err(error.into()),
            };
            if ready > 0 {
                guard.read().context("leyendo eventos Wayland")?;
            }
        }
        self.queue.dispatch_pending(&mut self.state)?;
        Ok(())
    }

    /// Dispatches until `deadline_ns` (monotonic).
    pub fn dispatch_until(&mut self, deadline_ns: u64) -> Result<()> {
        loop {
            let now = now_ns();
            if now >= deadline_ns {
                return Ok(());
            }
            self.dispatch_for(Duration::from_nanos((deadline_ns - now).min(50_000_000)))?;
        }
    }

    /// Dispatches until `done` holds or `deadline_ns` passes; returns whether
    /// it held.
    pub fn dispatch_while(
        &mut self,
        deadline_ns: u64,
        mut done: impl FnMut(&State) -> bool,
    ) -> Result<bool> {
        loop {
            if done(&self.state) {
                return Ok(true);
            }
            let now = now_ns();
            if now >= deadline_ns {
                return Ok(false);
            }
            self.dispatch_for(Duration::from_nanos((deadline_ns - now).min(20_000_000)))?;
        }
    }

    /// The output rectangle (the whole invisible desktop: one window fills it).
    pub fn output_region(&self) -> Rect {
        let (width, height) = self
            .state
            .output_size
            .or_else(|| {
                self.state
                    .slots
                    .iter()
                    .find_map(|slot| slot.buffer.as_ref())
                    .map(|b| (b.width as i32, b.height as i32))
            })
            .unwrap_or((1920, 1080));
        Rect {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    // ------------------------------------------------------------ windows

    /// Subscribes to window announcements with `protocol`.
    pub fn watch_toplevels(&mut self, protocol: ToplevelProtocol) -> Result<()> {
        let qh = self.state.qh.clone();
        match protocol {
            ToplevelProtocol::Wlr => {
                let manager: ZwlrForeignToplevelManagerV1 = self
                    .globals
                    .bind(&qh, 1..=3, ())
                    .context("el compositor no ofrece zwlr_foreign_toplevel_manager_v1")?;
                self.wlr_toplevels = Some(manager);
            }
            ToplevelProtocol::Ext => {
                let list: ExtForeignToplevelListV1 = self
                    .globals
                    .bind(&qh, 1..=1, ())
                    .context("el compositor no ofrece ext_foreign_toplevel_list_v1")?;
                self.ext_toplevels = Some(list);
            }
        }
        self.roundtrip()
    }

    /// From now on, windows announced are marked as new.
    pub fn arm_toplevels(&mut self) {
        self.state.armed = true;
    }

    pub fn toplevels(&self) -> impl Iterator<Item = &Toplevel> {
        self.state.toplevels.values()
    }

    // ------------------------------------------------------------ capture

    /// Starts damage-driven capture of the output with two frames in flight,
    /// so a frame produced while the other is being re-armed is not missed.
    pub fn start_capture(&mut self) -> Result<()> {
        let qh = self.state.qh.clone();
        let (manager, version): (ZwlrScreencopyManagerV1, u32) = {
            let manager: ZwlrScreencopyManagerV1 = self
                .globals
                .bind(&qh, 2..=3, ())
                .context("el compositor no ofrece zwlr_screencopy_manager_v1 (v2 o más)")?;
            let version = manager.version();
            (manager, version)
        };
        if self.state.output.is_none() || self.state.shm.is_none() {
            bail!("el compositor no ofrece wl_output o wl_shm");
        }
        self.state.screencopy = Some(manager);
        self.state.screencopy_version = version;
        self.state.slots = (0..2).map(|_| Slot::default()).collect();
        self.state.capturing = true;
        for index in 0..self.state.slots.len() {
            self.state.arm(index);
        }
        self.queue.flush()?;
        Ok(())
    }

    /// Captures the empty desktop once (plain `copy`, no damage wait) and
    /// remembers its dominant color, so frames where the new window is not
    /// drawn yet can be told apart. Call after [`Session::start_capture`].
    pub fn snapshot_desktop(&mut self) -> Result<Option<[u8; 3]>> {
        let index = self.state.slots.len();
        self.state.slots.push(Slot {
            plain: true,
            ..Slot::default()
        });
        self.state.arm(index);
        let deadline = now_ns() + 2_000_000_000;
        self.dispatch_while(deadline, |state| state.desktop_done)?;
        Ok(self.state.desktop_color)
    }

    /// Analyzes the pixels of every frame from now on until one has content.
    pub fn start_content_probe(&mut self) {
        self.state.content = Some(ContentProbe {
            detector: ContentDetector::new(self.state.desktop_color),
            first_frame_ns: None,
            first_content_ns: None,
            last_fraction: 0.0,
            last_colors: 0,
        });
    }

    /// `(first frame, first frame with content, last content fraction)`.
    pub fn content_result(&self) -> (Option<u64>, Option<u64>, f64) {
        self.state
            .content
            .as_ref()
            .map(|probe| {
                (
                    probe.first_frame_ns,
                    probe.first_content_ns,
                    probe.last_fraction,
                )
            })
            .unwrap_or((None, None, 0.0))
    }

    /// Distinct colors of the last analyzed frame.
    pub fn content_colors(&self) -> usize {
        self.state
            .content
            .as_ref()
            .map_or(0, |probe| probe.last_colors)
    }

    /// Color of the empty desktop, as `#rrggbb` in memory byte order.
    pub fn desktop_color(&self) -> Option<String> {
        let [a, b, c] = self.state.desktop_color?;
        Some(format!("#{a:02x}{b:02x}{c:02x}"))
    }

    /// Background color of the first mapped frame, as `#rrggbb` (bytes in
    /// memory order, which is BGR for the usual XRGB8888 buffers).
    pub fn content_background(&self) -> Option<String> {
        let [a, b, c] = self.state.content.as_ref()?.detector.background()?;
        Some(format!("#{a:02x}{b:02x}{c:02x}"))
    }

    pub fn clock(&self) -> ClockSource {
        self.state.clock.unwrap_or(ClockSource::Compositor)
    }

    // ------------------------------------------------------------ input

    /// Creates the virtual keyboard (US layout) on the seat.
    pub fn keyboard(&mut self) -> Result<()> {
        if self.keyboard.is_some() {
            return Ok(());
        }
        let qh = self.state.qh.clone();
        let seat = self
            .seat
            .clone()
            .context("el compositor no ofrece wl_seat")?;
        let manager: ZwpVirtualKeyboardManagerV1 = self
            .globals
            .bind(&qh, 1..=1, ())
            .context("el compositor no ofrece zwp_virtual_keyboard_manager_v1")?;
        let keyboard = manager.create_virtual_keyboard(&seat, &qh, ());
        let fd = rustix::fs::memfd_create("cincel-perf-keymap", MemfdFlags::CLOEXEC)?;
        let mut file = File::from(fd);
        file.write_all(KEYMAP.as_bytes())?;
        file.write_all(&[0])?;
        // Format 1 = WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1.
        keyboard.keymap(1, file.as_fd(), (KEYMAP.len() + 1) as u32);
        self.keyboard = Some(keyboard);
        self.roundtrip()
    }

    fn time_ms() -> u32 {
        (now_ns() / 1_000_000) as u32
    }

    fn set_modifiers(&mut self, mask: u32) {
        if let Some(keyboard) = &self.keyboard
            && self.modifiers != mask
        {
            keyboard.modifiers(mask, 0, 0, 0);
            self.modifiers = mask;
        }
    }

    /// Queues a full stroke (modifiers down, key down/up, modifiers up)
    /// without flushing; see [`Session::flush_now`].
    pub fn queue_stroke(&mut self, stroke: &Stroke) -> Result<()> {
        let keyboard = self.keyboard.clone().context("teclado virtual no creado")?;
        let time = Self::time_ms();
        for &modifier in &stroke.modifier_keys {
            keyboard.key(time, modifier, 1);
        }
        self.set_modifiers(stroke.modifier_mask);
        keyboard.key(time, stroke.key, 1);
        keyboard.key(time, stroke.key, 0);
        for &modifier in stroke.modifier_keys.iter().rev() {
            keyboard.key(time, modifier, 0);
        }
        self.set_modifiers(0);
        Ok(())
    }

    /// Flushes queued requests and returns the monotonic time just before
    /// the flush (the injection mark of §3.2).
    pub fn flush_now(&mut self) -> Result<u64> {
        let mark = now_ns();
        self.queue.flush().context("flush Wayland")?;
        Ok(mark)
    }

    /// Creates the virtual pointer, bound to the output when possible.
    pub fn pointer(&mut self) -> Result<()> {
        if self.pointer.is_some() {
            return Ok(());
        }
        let qh = self.state.qh.clone();
        let manager: ZwlrVirtualPointerManagerV1 = self
            .globals
            .bind(&qh, 1..=2, ())
            .context("el compositor no ofrece zwlr_virtual_pointer_manager_v1")?;
        let pointer = match (&self.state.output, manager.version() >= 2) {
            (Some(output), true) => manager.create_virtual_pointer_with_output(
                self.seat.as_ref(),
                Some(output),
                &qh,
                (),
            ),
            _ => manager.create_virtual_pointer(self.seat.as_ref(), &qh, ()),
        };
        self.pointer = Some(pointer);
        self.roundtrip()
    }

    /// Moves the pointer to `(x, y)` of the output.
    pub fn move_pointer(&mut self, x: u32, y: u32) -> Result<()> {
        let region = self.output_region();
        let pointer = self.pointer.clone().context("puntero virtual no creado")?;
        pointer.motion_absolute(
            Self::time_ms(),
            x,
            y,
            region.width.max(1) as u32,
            region.height.max(1) as u32,
        );
        pointer.frame();
        self.queue.flush()?;
        Ok(())
    }

    /// Queues one wheel notch down (`discrete` = 1, 15 units like a mouse).
    pub fn queue_wheel(&mut self) -> Result<()> {
        let pointer = self.pointer.clone().context("puntero virtual no creado")?;
        let time = Self::time_ms();
        pointer.axis_source(wl_pointer::AxisSource::Wheel);
        pointer.axis_discrete(time, wl_pointer::Axis::VerticalScroll, 15.0, 1);
        pointer.frame();
        Ok(())
    }
}

impl State {
    /// Requests a new frame for slot `index`.
    fn arm(&mut self, index: usize) {
        if !self.capturing {
            return;
        }
        let (Some(manager), Some(output)) = (&self.screencopy, &self.output) else {
            return;
        };
        let frame = manager.capture_output(0, output, &self.qh, index);
        let slot = &mut self.slots[index];
        if slot.plain && self.desktop_done {
            frame.destroy();
            return;
        }
        slot.frame = Some(frame);
        slot.pending = None;
        slot.damage.clear();
    }

    /// Makes sure slot `index` has a buffer of the announced size and asks
    /// for the copy.
    fn copy(&mut self, index: usize) {
        let Some((format, width, height, stride)) = self.slots[index].pending else {
            return;
        };
        let reuse = self.slots[index].buffer.as_ref().is_some_and(|buffer| {
            buffer.width == width
                && buffer.height == height
                && buffer.stride == stride
                && buffer.format == format
        });
        if !reuse {
            if let Some(old) = self.slots[index].buffer.take() {
                old.buffer.destroy();
                old.pool.destroy();
            }
            match self.new_buffer(format, width, height, stride) {
                Ok(buffer) => self.slots[index].buffer = Some(buffer),
                Err(_) => {
                    self.capture_failures += 1;
                    return;
                }
            }
        }
        let slot = &self.slots[index];
        if let (Some(frame), Some(buffer)) = (&slot.frame, &slot.buffer) {
            if slot.plain {
                frame.copy(&buffer.buffer);
            } else {
                frame.copy_with_damage(&buffer.buffer);
            }
        }
    }

    fn new_buffer(&self, format: u32, width: u32, height: u32, stride: u32) -> Result<ShmBuffer> {
        let shm = self.shm.as_ref().context("sin wl_shm")?;
        let size = stride as u64 * height as u64;
        let fd = rustix::fs::memfd_create("cincel-perf-frame", MemfdFlags::CLOEXEC)?;
        rustix::fs::ftruncate(&fd, size)?;
        let file = File::from(fd);
        let pool = shm.create_pool(file.as_fd(), size as i32, &self.qh, ());
        let shm_format = wl_shm::Format::try_from(format).unwrap_or(wl_shm::Format::Xrgb8888);
        let buffer = pool.create_buffer(
            0,
            width as i32,
            height as i32,
            stride as i32,
            shm_format,
            &self.qh,
            (),
        );
        Ok(ShmBuffer {
            file,
            pool,
            buffer,
            width,
            height,
            stride,
            format,
        })
    }

    fn frame_ready(&mut self, index: usize, compositor_ns: u64) {
        if self.slots[index].plain {
            self.desktop_color =
                self.read_slot(index)
                    .and_then(|(image_len, width, height, stride)| {
                        let image = Image {
                            width,
                            height,
                            stride,
                            data: &self.pixels[..image_len],
                        };
                        image.dominant_color(&Rect {
                            x: 0,
                            y: 0,
                            width: width as i32,
                            height: height as i32,
                        })
                    });
            self.desktop_done = true;
            if let Some(frame) = self.slots[index].frame.take() {
                frame.destroy();
            }
            return;
        }
        let received = now_ns();
        let clock = *self.clock.get_or_insert_with(|| {
            // The ready stamp must be CLOCK_MONOTONIC like ours; if it is off
            // by more than 5 s it is some other clock.
            if received.abs_diff(compositor_ns) < 5_000_000_000 {
                ClockSource::Compositor
            } else {
                ClockSource::Client
            }
        });
        let ts_ns = match clock {
            ClockSource::Compositor => compositor_ns,
            ClockSource::Client => received,
        };
        let damage = std::mem::take(&mut self.slots[index].damage);
        self.frames.push(Frame { ts_ns, damage });

        if self
            .content
            .as_ref()
            .is_some_and(|probe| probe.first_content_ns.is_none())
        {
            self.analyze(index, ts_ns);
        }
        if let Some(frame) = self.slots[index].frame.take() {
            frame.destroy();
        }
        self.arm(index);
    }

    /// Copies the pixels of slot `index` into `self.pixels`; returns
    /// `(len, width, height, stride)`.
    fn read_slot(&mut self, index: usize) -> Option<(usize, u32, u32, u32)> {
        let buffer = self.slots[index].buffer.as_ref()?;
        let len = (buffer.stride * buffer.height) as usize;
        self.pixels.resize(len, 0);
        buffer.file.read_exact_at(&mut self.pixels, 0).ok()?;
        Some((len, buffer.width, buffer.height, buffer.stride))
    }

    fn analyze(&mut self, index: usize, ts_ns: u64) {
        let Some((len, width, height, stride)) = self.read_slot(index) else {
            return;
        };
        let image = Image {
            width,
            height,
            stride,
            data: &self.pixels[..len],
        };
        let region = Rect {
            x: 0,
            y: 0,
            width: width as i32,
            height: height as i32,
        };
        let Some(probe) = self.content.as_mut() else {
            return;
        };
        let verdict = probe.detector.feed(&image, &region);
        if !verdict.drawn {
            return;
        }
        probe.first_frame_ns.get_or_insert(ts_ns);
        probe.last_fraction = verdict.fraction;
        probe.last_colors = verdict.colors;
        if verdict.content {
            probe.first_content_ns = Some(ts_ns);
        }
    }

    fn toplevel_event(&mut self, id: ObjectId, event: ToplevelChange) {
        let armed = self.armed;
        let entry = self.toplevels.entry(id).or_insert_with(|| Toplevel {
            new: armed,
            ..Toplevel::default()
        });
        match event {
            ToplevelChange::AppId(app_id) => entry.app_id = app_id,
            ToplevelChange::Title(title) => entry.title = title,
            ToplevelChange::Done => {
                entry.done_ns.get_or_insert_with(now_ns);
            }
            ToplevelChange::Closed => entry.closed = true,
        }
    }
}

enum ToplevelChange {
    AppId(String),
    Title(String),
    Done,
    Closed,
}

// ---------------------------------------------------------------- dispatch

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wayland_client::protocol::wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlOutput, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Mode {
            flags: WEnum::Value(flags),
            width,
            height,
            ..
        } = event
            && flags.contains(wl_output::Mode::Current)
        {
            state.output_size = Some((width, height));
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, usize> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let index = *index;
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer {
                format,
                width,
                height,
                stride,
            } => {
                let format = match format {
                    WEnum::Value(format) => format as u32,
                    WEnum::Unknown(raw) => raw,
                };
                state.slots[index].pending = Some((format, width, height, stride));
                if state.screencopy_version < 3 {
                    state.copy(index);
                }
            }
            zwlr_screencopy_frame_v1::Event::BufferDone => state.copy(index),
            zwlr_screencopy_frame_v1::Event::Damage {
                x,
                y,
                width,
                height,
            } => state.slots[index].damage.push(Rect {
                x: x as i32,
                y: y as i32,
                width: width as i32,
                height: height as i32,
            }),
            zwlr_screencopy_frame_v1::Event::Ready {
                tv_sec_hi,
                tv_sec_lo,
                tv_nsec,
            } => {
                let seconds = (u64::from(tv_sec_hi) << 32) | u64::from(tv_sec_lo);
                state.frame_ready(index, seconds * 1_000_000_000 + u64::from(tv_nsec));
            }
            zwlr_screencopy_frame_v1::Event::Failed => {
                if state.slots[index].plain {
                    state.desktop_done = true;
                }
                state.capture_failures += 1;
                if let Some(frame) = state.slots[index].frame.take() {
                    frame.destroy();
                }
                if state.capture_failures < 100 {
                    state.arm(index);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwlrForeignToplevelManagerV1,
        _: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }

    event_created_child!(State, ZwlrForeignToplevelManagerV1, [
        zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        handle: &ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let change = match event {
            zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                ToplevelChange::AppId(app_id)
            }
            zwlr_foreign_toplevel_handle_v1::Event::Title { title } => ToplevelChange::Title(title),
            zwlr_foreign_toplevel_handle_v1::Event::Done => ToplevelChange::Done,
            zwlr_foreign_toplevel_handle_v1::Event::Closed => ToplevelChange::Closed,
            _ => return,
        };
        state.toplevel_event(handle.id(), change);
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ExtForeignToplevelListV1,
        _: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }

    event_created_child!(State, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        handle: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let change = match event {
            ext_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                ToplevelChange::AppId(app_id)
            }
            ext_foreign_toplevel_handle_v1::Event::Title { title } => ToplevelChange::Title(title),
            ext_foreign_toplevel_handle_v1::Event::Done => ToplevelChange::Done,
            ext_foreign_toplevel_handle_v1::Event::Closed => ToplevelChange::Closed,
            _ => return,
        };
        state.toplevel_event(handle.id(), change);
    }
}

delegate_noop!(State: ignore WlShm);
delegate_noop!(State: ignore WlSeat);
delegate_noop!(State: ignore WlBuffer);
delegate_noop!(State: WlShmPool);
delegate_noop!(State: ZwlrScreencopyManagerV1);
delegate_noop!(State: ZwpVirtualKeyboardManagerV1);
delegate_noop!(State: ZwpVirtualKeyboardV1);
delegate_noop!(State: ZwlrVirtualPointerManagerV1);
delegate_noop!(State: ZwlrVirtualPointerV1);
