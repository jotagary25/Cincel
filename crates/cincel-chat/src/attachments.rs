//! Image attachments of the composer, without GPUI
//! (`docs/specs/09-etapa7-conexiones-imagenes-comentarios.md` §5.3.1).
//!
//! [`prepare_image`] turns the bytes of a pasted, dropped or picked file into
//! what travels to the agent: the format is detected **by content** (never by
//! the extension), an image of at most [`MAX_IMAGE_SIDE`] px per side and
//! [`MAX_IMAGE_BYTES`] keeps its original bytes untouched, and only a bigger
//! one is decoded, reduced with Lanczos3 (EXIF orientation applied) and
//! encoded again (D5): PNG → PNG, JPEG → JPEG at [`JPEG_QUALITY`], WebP → PNG
//! with transparency or JPEG without it (`image` only writes lossless WebP,
//! which would weigh more), GIF → its first frame as PNG. The panel runs it on
//! the background executor ([`crate::ChatPanel::attach_paths`]).
//!
//! The rest of the module is the Spanish wording of every notice, so the
//! texts of §7.3 live in one place and the tests can compare them literally.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::{AnimationDecoder as _, DynamicImage, ImageDecoder as _, ImageFormat, ImageReader};
use sha2::{Digest as _, Sha256};

/// Longest side, in pixels, an image keeps; a bigger one is reduced to it.
pub const MAX_IMAGE_SIDE: u32 = 2_000;
/// Largest image, in bytes, that may travel (after reducing).
pub const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
/// Images one message may carry (D5).
pub const MAX_IMAGES_PER_MESSAGE: usize = 10;
/// Quality of a JPEG encoded again after reducing it.
pub const JPEG_QUALITY: u8 = 85;
/// The name a pasted image gets (the clipboard carries no file name).
pub const PASTED_IMAGE_NAME: &str = "Imagen pegada";
/// Extensions the "Adjuntar imagen" dialog filters on, and the ones a path
/// from the tree or the clipboard has to have to be taken for an image.
pub const IMAGE_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];

/// Title of the file dialog the workspace opens for [`crate::ChatEvent::PickImages`].
pub const PICK_DIALOG_TITLE: &str = "Adjuntar imagen";
/// Name of the dialog's filter.
pub const PICK_DIALOG_FILTER: &str = "Imágenes";
/// Tooltip of the paperclip button.
pub const ATTACH_TOOLTIP: &str = "Adjuntar imagen";
/// Tooltip of a thumbnail's `×`.
pub const REMOVE_IMAGE_TOOLTIP: &str = "Quitar imagen";
/// Tooltip of a comment tag's `×`.
pub const REMOVE_COMMENT_TOOLTIP: &str = "Quitar comentario";
/// What a sent image whose file is gone shows instead of the thumbnail.
pub const IMAGE_UNAVAILABLE: &str = "Imagen no disponible";
/// Without an active connection nothing is attached (D6).
pub const NO_CONNECTION_FOR_IMAGES: &str = "Conectá un agente para adjuntar imágenes";
/// The eleventh image of a message.
pub const TOO_MANY_IMAGES: &str = "Se pueden adjuntar hasta 10 imágenes por mensaje";

/// An image ready to travel: the bytes the agent receives and what the
/// thumbnail and its tooltip show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedImage {
    /// File name, or [`PASTED_IMAGE_NAME`].
    pub name: String,
    /// `image/png`, `image/jpeg`, `image/gif` or `image/webp`.
    pub mime_type: &'static str,
    /// What travels: the original bytes, or the reduced encoding.
    pub bytes: Arc<[u8]>,
    /// Lowercase hex SHA-256 of [`Self::bytes`] (the stored file is
    /// `<sha256>.<ext>`, D7).
    pub sha256: String,
    /// Width of what travels, in pixels (after the EXIF orientation).
    pub width: u32,
    /// Height of what travels, in pixels.
    pub height: u32,
    /// The image was bigger than [`MAX_IMAGE_SIDE`] and was reduced.
    pub resized: bool,
    /// The bytes are a new encoding, not the file's own.
    pub recoded: bool,
    /// An animated GIF was reduced: only its first frame travels.
    pub animation_dropped: bool,
}

impl PreparedImage {
    /// The stored file name, `<sha256>.<ext>` (D7).
    #[must_use]
    pub fn file_name(&self) -> String {
        format!("{}.{}", self.sha256, mime_extension(self.mime_type))
    }

    /// "nombre · 1920 × 1080 · 1,2 MB", the thumbnail's tooltip.
    #[must_use]
    pub fn summary(&self) -> String {
        image_summary(&self.name, self.width, self.height, self.bytes.len() as u64)
    }
}

/// Why an image was not attached.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageError {
    /// Not PNG, JPEG, GIF nor WebP (by content).
    NotAnImage,
    /// Still above [`MAX_IMAGE_BYTES`] after reducing it.
    TooLarge {
        /// Size of the reduced encoding.
        bytes: usize,
    },
    /// The file could not be read or decoded; the reason is in Spanish.
    Unreadable(String),
}

impl ImageError {
    /// The notice the panel emits for the file `name`.
    #[must_use]
    pub fn message(&self, name: &str) -> String {
        match self {
            ImageError::NotAnImage => format!("«{name}» no es una imagen PNG, JPEG, GIF o WebP"),
            ImageError::TooLarge { bytes } => format!(
                "«{name}» pesa {} aun reducida; el máximo es 10 MB",
                format_size(*bytes as u64)
            ),
            ImageError::Unreadable(reason) => format!("«{name}» no se pudo leer: {reason}"),
        }
    }
}

/// "«Nombre» no acepta imágenes" (the connection's label).
#[must_use]
pub fn no_images_text(connection: &str) -> String {
    format!("«{connection}» no acepta imágenes")
}

/// "«Nombre» no acepta imágenes: quitá las imágenes para enviar".
#[must_use]
pub fn blocked_send_text(connection: &str) -> String {
    format!("«{connection}» no acepta imágenes: quitá las imágenes para enviar")
}

/// "La animación de «nombre» se pierde al reducirla".
#[must_use]
pub fn animation_dropped_text(name: &str) -> String {
    format!("La animación de «{name}» se pierde al reducirla")
}

/// "Preparando 1 imagen…" / "Preparando N imágenes…".
#[must_use]
pub fn preparing_text(count: usize) -> String {
    if count == 1 {
        "Preparando 1 imagen…".to_string()
    } else {
        format!("Preparando {count} imágenes…")
    }
}

/// A size as the tooltips and notices write it: "820 B", "12,4 KB", "1,2 MB"
/// (binary units, a comma as decimal separator).
#[must_use]
pub fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.;
    let value = bytes as f64;
    if value < KB {
        return format!("{bytes} B");
    }
    let (amount, unit) = if value < KB * KB {
        (value / KB, "KB")
    } else {
        (value / (KB * KB), "MB")
    };
    format!("{amount:.1} {unit}").replace('.', ",")
}

/// "nombre · ancho × alto · tamaño".
#[must_use]
pub fn image_summary(name: &str, width: u32, height: u32, bytes: u64) -> String {
    format!("{name} · {width} × {height} · {}", format_size(bytes))
}

/// The extension of the stored file for a MIME type.
#[must_use]
pub fn mime_extension(mime_type: &str) -> &'static str {
    match mime_type {
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "png",
    }
}

/// Lowercase hex SHA-256.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

/// Whether `path` has one of [`IMAGE_EXTENSIONS`] (case-insensitive). Only
/// for routing (the tree, the clipboard): what is attached is still checked
/// by content.
#[must_use]
pub fn has_image_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            IMAGE_EXTENSIONS
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
        })
}

/// The MIME type and pixel size of an image, from its header, when it is one
/// of the four formats (used for the images a `session/load` replays).
#[must_use]
pub fn sniff_image(bytes: &[u8]) -> Option<(&'static str, u32, u32)> {
    let format = supported_format(bytes)?;
    let (width, height) = ImageReader::with_format(Cursor::new(bytes), format)
        .into_dimensions()
        .ok()?;
    Some((format_mime(format), width, height))
}

/// The paths of a clipboard text made only of `file://` URIs, one per line
/// (what a file manager offers on Wayland, where GPUI reads it as text
/// instead of `ClipboardEntry::ExternalPaths`). `None` when any line is
/// something else, so ordinary text is pasted as text.
#[must_use]
pub fn file_uri_paths(text: &str) -> Option<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let rest = line.strip_prefix("file://")?;
        // `file:///home/…` or `file://localhost/home/…`.
        let rest = rest.strip_prefix("localhost").unwrap_or(rest);
        if !rest.starts_with('/') {
            return None;
        }
        paths.push(PathBuf::from(percent_decode(rest)?));
    }
    (!paths.is_empty()).then_some(paths)
}

/// `%XX` decoding of a URI path; `None` for a malformed escape or a result
/// that is not UTF-8.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = text.get(index + 1..index + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The display name of a file: its name, or the whole path when it has none.
#[must_use]
pub fn display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map_or_else(|| path.display().to_string(), str::to_string)
}

/// Reads `path` and prepares it ([`prepare_image`]). Runs on the background
/// executor.
///
/// # Errors
///
/// [`ImageError::Unreadable`] when the file cannot be read, and whatever
/// [`prepare_image`] says about its bytes.
pub fn read_and_prepare(path: &Path) -> Result<PreparedImage, ImageError> {
    let name = display_name(path);
    if path.is_dir() {
        return Err(ImageError::Unreadable("es una carpeta".to_string()));
    }
    let bytes = std::fs::read(path).map_err(|error| ImageError::Unreadable(io_reason(&error)))?;
    prepare_image(&bytes, &name)
}

/// The reason of an I/O error, in Spanish when it is a common one.
fn io_reason(error: &std::io::Error) -> String {
    use std::io::ErrorKind;
    match error.kind() {
        ErrorKind::NotFound => "el archivo no existe".to_string(),
        ErrorKind::PermissionDenied => "no hay permiso para leerlo".to_string(),
        ErrorKind::IsADirectory => "es una carpeta".to_string(),
        _ => error.to_string(),
    }
}

/// The four formats Cincel attaches, detected by content.
fn supported_format(bytes: &[u8]) -> Option<ImageFormat> {
    match image::guess_format(bytes).ok()? {
        format @ (ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::WebP) => {
            Some(format)
        }
        _ => None,
    }
}

fn format_mime(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Gif => "image/gif",
        ImageFormat::WebP => "image/webp",
        _ => "image/png",
    }
}

/// The reason a decode failed, as the notice says it.
fn decode_reason(error: &image::ImageError) -> String {
    tracing::debug!(%error, "no se pudo decodificar una imagen adjunta");
    "el archivo está dañado o incompleto".to_string()
}

/// Turns the bytes of a file into what travels to the agent (D5).
///
/// The whole image is decoded even when its bytes are kept, so a truncated
/// or corrupt file is refused here instead of reaching the agent.
///
/// # Errors
///
/// [`ImageError::NotAnImage`] for anything but PNG, JPEG, GIF and WebP,
/// [`ImageError::Unreadable`] for a file that does not decode, and
/// [`ImageError::TooLarge`] when even the reduced encoding weighs more than
/// [`MAX_IMAGE_BYTES`].
pub fn prepare_image(bytes: &[u8], name: &str) -> Result<PreparedImage, ImageError> {
    let format = supported_format(bytes).ok_or(ImageError::NotAnImage)?;
    let mut decoder = ImageReader::with_format(Cursor::new(bytes), format)
        .into_decoder()
        .map_err(|error| ImageError::Unreadable(decode_reason(&error)))?;
    let orientation = decoder
        .orientation()
        .map_err(|error| ImageError::Unreadable(decode_reason(&error)))?;
    let mut decoded = DynamicImage::from_decoder(decoder)
        .map_err(|error| ImageError::Unreadable(decode_reason(&error)))?;
    decoded.apply_orientation(orientation);
    let (width, height) = (decoded.width(), decoded.height());

    let oversized = width.max(height) > MAX_IMAGE_SIDE;
    if !oversized && bytes.len() <= MAX_IMAGE_BYTES {
        return Ok(PreparedImage {
            name: name.to_string(),
            mime_type: format_mime(format),
            sha256: sha256_hex(bytes),
            bytes: Arc::from(bytes),
            width,
            height,
            resized: false,
            recoded: false,
            animation_dropped: false,
        });
    }

    let animation_dropped = oversized && format == ImageFormat::Gif && is_animated_gif(bytes);
    let reduced = if oversized {
        decoded.resize(
            MAX_IMAGE_SIDE,
            MAX_IMAGE_SIDE,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        decoded
    };
    let as_jpeg = match format {
        ImageFormat::Jpeg => true,
        ImageFormat::WebP => !has_transparency(&reduced),
        _ => false,
    };
    let (encoded, mime_type) = if as_jpeg {
        (encode_jpeg(&reduced)?, "image/jpeg")
    } else {
        (encode_png(&reduced)?, "image/png")
    };
    if encoded.len() > MAX_IMAGE_BYTES {
        return Err(ImageError::TooLarge {
            bytes: encoded.len(),
        });
    }
    Ok(PreparedImage {
        name: name.to_string(),
        mime_type,
        sha256: sha256_hex(&encoded),
        width: reduced.width(),
        height: reduced.height(),
        bytes: Arc::from(encoded),
        resized: oversized,
        recoded: true,
        animation_dropped,
    })
}

/// Whether a GIF has more than one frame.
fn is_animated_gif(bytes: &[u8]) -> bool {
    image::codecs::gif::GifDecoder::new(Cursor::new(bytes))
        .map(|decoder| decoder.into_frames().take(2).count() > 1)
        .unwrap_or(false)
}

/// Whether any pixel is not fully opaque.
fn has_transparency(image: &DynamicImage) -> bool {
    image.color().has_alpha() && image.to_rgba8().pixels().any(|pixel| pixel.0[3] < u8::MAX)
}

fn encode_png(image: &DynamicImage) -> Result<Vec<u8>, ImageError> {
    let mut out = Vec::new();
    image
        .write_with_encoder(image::codecs::png::PngEncoder::new(&mut out))
        .map_err(|error| ImageError::Unreadable(decode_reason(&error)))?;
    Ok(out)
}

fn encode_jpeg(image: &DynamicImage) -> Result<Vec<u8>, ImageError> {
    // JPEG has no alpha channel: an opaque image is written as RGB.
    let rgb = DynamicImage::ImageRgb8(image.to_rgb8());
    let mut out = Vec::new();
    rgb.write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
        &mut out,
        JPEG_QUALITY,
    ))
    .map_err(|error| ImageError::Unreadable(decode_reason(&error)))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage, Rgba, RgbaImage};

    fn encode(image: &DynamicImage, format: ImageFormat) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        image.write_to(&mut out, format).expect("encode");
        out.into_inner()
    }

    fn gradient(width: u32, height: u32) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_fn(width, height, |x, y| {
            Rgb([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8])
        }))
    }

    /// Pseudo-random pixels (an LCG), which no encoder can shrink.
    fn noise(width: u32, height: u32) -> DynamicImage {
        let mut state = 0x2545_f491_u32;
        DynamicImage::ImageRgb8(RgbImage::from_fn(width, height, |_, _| {
            let mut next = || {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 24) as u8
            };
            Rgb([next(), next(), next()])
        }))
    }

    #[test]
    fn the_four_formats_are_detected_by_content_and_keep_their_bytes() {
        let image = gradient(64, 48);
        for (format, mime) in [
            (ImageFormat::Png, "image/png"),
            (ImageFormat::Jpeg, "image/jpeg"),
            (ImageFormat::Gif, "image/gif"),
            (ImageFormat::WebP, "image/webp"),
        ] {
            let bytes = encode(&image, format);
            // The name lies about the format: only the content counts.
            let prepared = prepare_image(&bytes, "foto.txt").expect("se prepara");
            assert_eq!(prepared.mime_type, mime);
            assert_eq!(&*prepared.bytes, bytes.as_slice(), "{mime} sin recodificar");
            assert_eq!((prepared.width, prepared.height), (64, 48));
            assert!(!prepared.resized && !prepared.recoded);
            assert_eq!(prepared.sha256, sha256_hex(&bytes));
            assert_eq!(
                prepared.file_name(),
                format!("{}.{}", prepared.sha256, mime_extension(mime))
            );
        }
    }

    #[test]
    fn something_else_is_not_an_image() {
        assert_eq!(
            prepare_image(b"hola, no soy una imagen", "notas.png"),
            Err(ImageError::NotAnImage)
        );
        // A BMP header (`image` recognizes it, Cincel does not attach it).
        let mut bmp = b"BM".to_vec();
        bmp.extend_from_slice(&[0; 64]);
        assert_eq!(prepare_image(&bmp, "a.bmp"), Err(ImageError::NotAnImage));
        assert_eq!(
            ImageError::NotAnImage.message("notas.txt"),
            "«notas.txt» no es una imagen PNG, JPEG, GIF o WebP"
        );
    }

    #[test]
    fn a_truncated_file_is_unreadable() {
        let bytes = encode(&gradient(200, 200), ImageFormat::Png);
        let cut = &bytes[..bytes.len() / 2];
        assert!(matches!(
            prepare_image(cut, "rota.png"),
            Err(ImageError::Unreadable(_))
        ));
    }

    #[test]
    fn a_big_png_is_reduced_keeping_its_aspect_ratio() {
        let bytes = encode(&gradient(4_000, 3_000), ImageFormat::Png);
        let prepared = prepare_image(&bytes, "grande.png").expect("se prepara");
        assert_eq!((prepared.width, prepared.height), (2_000, 1_500));
        assert_eq!(prepared.mime_type, "image/png");
        assert!(prepared.resized && prepared.recoded);
        let decoded = image::load_from_memory(&prepared.bytes).expect("decodifica");
        assert_eq!((decoded.width(), decoded.height()), (2_000, 1_500));
        // Portrait too.
        let tall = encode(&gradient(1_000, 4_000), ImageFormat::Png);
        let prepared = prepare_image(&tall, "alta.png").expect("se prepara");
        assert_eq!((prepared.width, prepared.height), (500, 2_000));
    }

    #[test]
    fn a_big_jpeg_stays_jpeg_at_quality_85() {
        let bytes = encode(&gradient(2_400, 1_200), ImageFormat::Jpeg);
        let prepared = prepare_image(&bytes, "foto.jpg").expect("se prepara");
        assert_eq!(prepared.mime_type, "image/jpeg");
        assert_eq!((prepared.width, prepared.height), (2_000, 1_000));
        // Exactly what encoding the reduced image at quality 85 gives.
        let reduced = image::load_from_memory(&bytes).expect("decodifica").resize(
            2_000,
            2_000,
            image::imageops::FilterType::Lanczos3,
        );
        assert_eq!(
            &*prepared.bytes,
            encode_jpeg(&reduced).expect("jpeg").as_slice()
        );
    }

    #[test]
    fn a_big_webp_becomes_png_with_transparency_and_jpeg_without() {
        let opaque = encode(&gradient(2_100, 100), ImageFormat::WebP);
        let prepared = prepare_image(&opaque, "opaca.webp").expect("se prepara");
        assert_eq!(prepared.mime_type, "image/jpeg");
        assert_eq!(prepared.width, 2_000);

        let transparent = DynamicImage::ImageRgba8(RgbaImage::from_fn(2_100, 100, |x, _| {
            Rgba([10, 20, 30, if x < 1_000 { 0 } else { 255 }])
        }));
        let bytes = encode(&transparent, ImageFormat::WebP);
        let prepared = prepare_image(&bytes, "transparente.webp").expect("se prepara");
        assert_eq!(prepared.mime_type, "image/png");
        assert!(prepared.file_name().ends_with(".png"));
    }

    #[test]
    fn a_big_animated_gif_keeps_only_its_first_frame_as_png() {
        use image::codecs::gif::GifEncoder;
        use image::{Delay, Frame};
        let mut bytes = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut bytes);
            for shade in [0u8, 200] {
                let frame = RgbaImage::from_pixel(2_200, 40, Rgba([shade, shade, shade, 255]));
                encoder
                    .encode_frame(Frame::from_parts(
                        frame,
                        0,
                        0,
                        Delay::from_numer_denom_ms(100, 1),
                    ))
                    .expect("cuadro");
            }
        }
        let prepared = prepare_image(&bytes, "anim.gif").expect("se prepara");
        assert!(prepared.animation_dropped);
        assert_eq!(prepared.mime_type, "image/png");
        assert_eq!(prepared.width, 2_000);
        let first = image::load_from_memory(&prepared.bytes)
            .expect("png")
            .to_rgba8();
        assert_eq!(first.get_pixel(0, 0).0[0], 0, "el primer cuadro");
        assert_eq!(
            animation_dropped_text("anim.gif"),
            "La animación de «anim.gif» se pierde al reducirla"
        );

        // A small animated GIF keeps its bytes, animation included.
        let mut small = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut small);
            for shade in [0u8, 200] {
                let frame = RgbaImage::from_pixel(10, 10, Rgba([shade, shade, shade, 255]));
                encoder
                    .encode_frame(Frame::from_parts(
                        frame,
                        0,
                        0,
                        Delay::from_numer_denom_ms(100, 1),
                    ))
                    .expect("cuadro");
            }
        }
        let prepared = prepare_image(&small, "chica.gif").expect("se prepara");
        assert!(!prepared.animation_dropped && !prepared.recoded);
        assert_eq!(&*prepared.bytes, small.as_slice());
    }

    #[test]
    fn noise_that_stays_above_10_mb_after_reducing_is_refused_with_its_size() {
        let bytes = encode(&noise(2_400, 2_400), ImageFormat::Png);
        let error = prepare_image(&bytes, "ruido.png").expect_err("pesa demasiado");
        let ImageError::TooLarge { bytes: size } = error else {
            panic!("se esperaba TooLarge: {error:?}");
        };
        assert!(size > MAX_IMAGE_BYTES);
        let message = error.message("ruido.png");
        assert!(
            message.starts_with("«ruido.png» pesa ")
                && message.ends_with(" MB aun reducida; el máximo es 10 MB"),
            "{message}"
        );
        assert!(message.contains(','), "coma decimal: {message}");
    }

    #[test]
    fn sizes_and_summaries_read_in_spanish() {
        assert_eq!(format_size(820), "820 B");
        assert_eq!(format_size(12_698), "12,4 KB");
        assert_eq!(format_size(14_889_779), "14,2 MB");
        assert_eq!(
            image_summary("captura.png", 1920, 1080, 1_258_291),
            "captura.png · 1920 × 1080 · 1,2 MB"
        );
        assert_eq!(preparing_text(1), "Preparando 1 imagen…");
        assert_eq!(preparing_text(3), "Preparando 3 imágenes…");
        assert_eq!(no_images_text("Claude"), "«Claude» no acepta imágenes");
        assert_eq!(
            blocked_send_text("Claude"),
            "«Claude» no acepta imágenes: quitá las imágenes para enviar"
        );
    }

    #[test]
    fn clipboard_file_uris_become_paths_and_other_text_does_not() {
        assert_eq!(
            file_uri_paths("file:///home/ana/captura%20uno.png\nfile:///tmp/b.jpg\n"),
            Some(vec![
                PathBuf::from("/home/ana/captura uno.png"),
                PathBuf::from("/tmp/b.jpg")
            ])
        );
        assert_eq!(file_uri_paths("hola\nfile:///tmp/b.jpg"), None);
        assert_eq!(file_uri_paths("/tmp/b.jpg"), None);
        assert_eq!(file_uri_paths(""), None);
        assert!(has_image_extension(Path::new("/a/B.PNG")));
        assert!(!has_image_extension(Path::new("/a/b.txt")));
    }

    #[test]
    fn missing_files_and_folders_are_unreadable_in_spanish() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("no-esta.png");
        assert_eq!(
            read_and_prepare(&missing),
            Err(ImageError::Unreadable("el archivo no existe".to_string()))
        );
        assert_eq!(
            read_and_prepare(dir.path()),
            Err(ImageError::Unreadable("es una carpeta".to_string()))
        );
        assert_eq!(
            ImageError::Unreadable("el archivo no existe".into()).message("no-esta.png"),
            "«no-esta.png» no se pudo leer: el archivo no existe"
        );
    }

    #[test]
    fn sniffing_reads_the_header() {
        let bytes = encode(&gradient(30, 20), ImageFormat::Png);
        assert_eq!(sniff_image(&bytes), Some(("image/png", 30, 20)));
        assert_eq!(sniff_image(b"nada"), None);
    }
}
