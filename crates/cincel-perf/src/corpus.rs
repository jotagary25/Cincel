//! Deterministic measurement corpus (§3.4): a fixed seed, a fixed git
//! author and date, so two runs produce byte-identical trees (and the same
//! commit id).

use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

/// Seed of every generator; changing it changes the whole corpus.
pub const SEED: u64 = 0x00C1_9CE1_BE9C_0001;
pub const GIT_NAME: &str = "Cincel Bench";
pub const GIT_EMAIL: &str = "bench@example.invalid";
pub const GIT_DATE: &str = "2026-01-01T00:00:00+00:00";

/// Names of the three files of `archivos/` (also copied into `medio/bench/`).
pub const FIVE_THOUSAND: &str = "cinco-mil.rs";
pub const ONE_MEGABYTE: &str = "un-mega.rs";
pub const FIFTY_THOUSAND: &str = "cincuenta-mil.rs";

/// Sizes of the corpus; [`Scale::FULL`] is the one §3.4 describes, the tests
/// use a small one.
#[derive(Debug, Clone, Copy)]
pub struct Scale {
    pub medium_files: usize,
    pub medium_dirs: usize,
    /// Lines per text file of `medio/` (inclusive range; ~100 on average).
    pub medium_lines: (usize, usize),
    pub medium_pngs: usize,
    pub large_files: usize,
    pub large_dirs: usize,
    /// Bytes per text file of `grande/` (inclusive range).
    pub large_text_bytes: (usize, usize),
    pub large_binaries: usize,
    /// Bytes per binary of `grande/` (inclusive range).
    pub large_binary_bytes: (usize, usize),
    pub five_thousand_lines: usize,
    pub one_megabyte_bytes: usize,
    pub fifty_thousand_lines: usize,
}

impl Scale {
    pub const FULL: Scale = Scale {
        medium_files: 2_000,
        medium_dirs: 120,
        medium_lines: (20, 180),
        medium_pngs: 5,
        large_files: 20_000,
        large_dirs: 400,
        large_text_bytes: (2_000, 23_000),
        large_binaries: 50,
        large_binary_bytes: (1_000_000, 5_000_000),
        five_thousand_lines: 5_000,
        one_megabyte_bytes: 1_000_000,
        fifty_thousand_lines: 50_000,
    };
}

/// What to generate.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub scale: Scale,
    pub large: bool,
    pub demo: bool,
    pub git: bool,
}

/// Summary of one generated part (one JSON line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub name: &'static str,
    pub files: usize,
    pub bytes: u64,
    /// SHA-256 of every (relative path, content) pair, `.git` excluded.
    pub sha256: String,
    pub commit: Option<String>,
}

impl Part {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "kind": "corpus",
            "part": self.name,
            "files": self.files,
            "bytes": self.bytes,
            "sha256": self.sha256,
            "commit": self.commit,
        })
    }
}

/// SplitMix64: tiny, fast and fully determined by the seed.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `low..=high`.
    pub fn range(&mut self, low: usize, high: usize) -> usize {
        if high <= low {
            return low;
        }
        low + (self.next_u64() % (high - low + 1) as u64) as usize
    }

    pub fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.range(0, items.len() - 1)]
    }

    pub fn chance(&mut self, percent: usize) -> bool {
        self.range(0, 99) < percent
    }
}

const NOUNS: &[&str] = &[
    "buffer", "cursor", "layout", "token", "parser", "cache", "index", "range", "anchor", "row",
    "column", "span", "theme", "glyph", "window", "pane", "tab", "entry", "node", "tree", "config",
    "event", "handle", "state", "queue", "worker", "client", "session", "request", "response",
    "payload", "record", "frame", "snapshot", "segment", "hunk", "patch", "commit", "branch",
    "file", "path", "folder", "project", "search", "match", "query", "filter", "keymap", "action",
    "command", "palette", "status", "message",
];
const VERBS: &[&str] = &[
    "build",
    "parse",
    "load",
    "store",
    "apply",
    "render",
    "measure",
    "update",
    "resolve",
    "insert",
    "remove",
    "find",
    "scan",
    "merge",
    "split",
    "format",
    "encode",
    "decode",
    "flush",
    "refresh",
    "select",
    "expand",
    "collapse",
    "validate",
    "normalize",
    "compare",
];
const TYPES: &[&str] = &[
    "usize", "u32", "u64", "i64", "f32", "bool", "String", "Vec<u8>",
];
const WORDS: &[&str] = &[
    "the",
    "editor",
    "keeps",
    "every",
    "change",
    "until",
    "you",
    "decide",
    "this",
    "module",
    "handles",
    "a",
    "small",
    "part",
    "of",
    "work",
    "when",
    "file",
    "is",
    "large",
    "we",
    "only",
    "draw",
    "visible",
    "rows",
    "and",
    "cache",
    "their",
    "layout",
    "for",
    "later",
    "frames",
    "which",
    "makes",
    "scrolling",
    "cheap",
    "even",
    "with",
    "long",
    "lines",
];

fn ident(rng: &mut Rng) -> String {
    format!("{}_{}", rng.pick(VERBS), rng.pick(NOUNS))
}

fn type_name(rng: &mut Rng) -> String {
    let noun = rng.pick(NOUNS);
    let mut chars = noun.chars();
    let first = chars.next().map(|c| c.to_ascii_uppercase()).unwrap_or('X');
    let other = rng.pick(NOUNS);
    let mut other_chars = other.chars();
    let other_first = other_chars
        .next()
        .map(|c| c.to_ascii_uppercase())
        .unwrap_or('Y');
    format!(
        "{first}{}{other_first}{}",
        chars.as_str(),
        other_chars.as_str()
    )
}

fn sentence(rng: &mut Rng, words: usize) -> String {
    let mut text = String::new();
    for index in 0..words {
        if index > 0 {
            text.push(' ');
        }
        text.push_str(rng.pick(WORDS));
    }
    text
}

/// One Rust item (a block of lines, each without the newline).
fn rust_item(rng: &mut Rng) -> Vec<String> {
    let mut lines = Vec::new();
    match rng.range(0, 3) {
        0 => {
            let name = type_name(rng);
            lines.push(format!("/// {}.", sentence(rng, 6)));
            lines.push("#[derive(Debug, Clone, Default)]".to_owned());
            lines.push(format!("pub struct {name} {{"));
            for _ in 0..rng.range(2, 6) {
                lines.push(format!("    /// {}.", sentence(rng, 5)));
                lines.push(format!(
                    "    pub {}_{}: {},",
                    rng.pick(NOUNS),
                    rng.pick(NOUNS),
                    rng.pick(TYPES)
                ));
            }
            lines.push("}".to_owned());
        }
        1 => {
            let name = type_name(rng);
            lines.push(format!("impl {name} {{"));
            for _ in 0..rng.range(1, 3) {
                let function = ident(rng);
                lines.push(format!(
                    "    pub fn {function}(&mut self, {}: {}) -> Option<{}> {{",
                    rng.pick(NOUNS),
                    rng.pick(TYPES),
                    rng.pick(TYPES)
                ));
                for _ in 0..rng.range(2, 6) {
                    lines.push(format!(
                        "        let {}_{} = self.{}.len() + {}; // {}",
                        rng.pick(NOUNS),
                        rng.pick(VERBS),
                        rng.pick(NOUNS),
                        rng.range(0, 99),
                        sentence(rng, 3)
                    ));
                }
                lines.push("        None".to_owned());
                lines.push("    }".to_owned());
            }
            lines.push("}".to_owned());
        }
        2 => {
            let function = ident(rng);
            lines.push(format!("// {}", sentence(rng, 8)));
            lines.push(format!("fn {function}(items: &[u32]) -> u64 {{"));
            lines.push("    let mut total = 0u64;".to_owned());
            lines.push("    for item in items {".to_owned());
            lines.push(format!(
                "        if *item % {} == 0 && total < {}_LIMIT as u64 {{",
                rng.range(2, 9),
                rng.pick(NOUNS).to_uppercase()
            ));
            lines.push(format!(
                "            total += u64::from(*item) * {};",
                rng.range(1, 9)
            ));
            lines.push("        }".to_owned());
            lines.push("    }".to_owned());
            lines.push("    total".to_owned());
            lines.push("}".to_owned());
        }
        _ => {
            lines.push(format!(
                "const {}_LIMIT: usize = {};",
                rng.pick(NOUNS).to_uppercase(),
                rng.range(1, 10_000)
            ));
        }
    }
    lines.push(String::new());
    lines
}

fn ts_item(rng: &mut Rng) -> Vec<String> {
    let name = type_name(rng);
    let mut lines = vec![format!("export interface {name} {{")];
    for _ in 0..rng.range(2, 5) {
        let ty = rng.pick(&["string", "number", "boolean", "string[]"]);
        lines.push(format!("  {}: {ty};", rng.pick(NOUNS)));
    }
    lines.push("}".to_owned());
    lines.push(String::new());
    let function = ident(rng);
    lines.push(format!(
        "export function {function}(input: {name}): number {{"
    ));
    lines.push(format!("  const limit = {};", rng.range(1, 500)));
    lines.push("  return Object.keys(input).length * limit;".to_owned());
    lines.push("}".to_owned());
    lines.push(String::new());
    lines
}

fn py_item(rng: &mut Rng) -> Vec<String> {
    let function = ident(rng);
    let mut lines = vec![
        format!("def {function}(items, limit={}):", rng.range(1, 100)),
        format!("    \"\"\"{}.\"\"\"", sentence(rng, 7)),
        "    result = []".to_owned(),
        "    for item in items:".to_owned(),
        format!("        if len(item) > {}:", rng.range(1, 20)),
        "            result.append(item)".to_owned(),
        "    return result[:limit]".to_owned(),
    ];
    lines.push(String::new());
    lines.push(String::new());
    lines
}

fn md_item(rng: &mut Rng) -> Vec<String> {
    let mut lines = vec![format!("## {}", sentence(rng, 3)), String::new()];
    for _ in 0..rng.range(2, 5) {
        lines.push(sentence(rng, 12));
    }
    lines.push(String::new());
    lines.push(format!("- {}", sentence(rng, 5)));
    lines.push(format!("- `{}`", ident(rng)));
    lines.push(String::new());
    lines
}

/// Text of `lines` lines (exactly), made of whole items and padded with
/// comments (or blank lines for JSON-free formats).
fn text_with_lines(
    rng: &mut Rng,
    lines: usize,
    item: fn(&mut Rng) -> Vec<String>,
    pad: &str,
) -> String {
    let mut out: Vec<String> = Vec::with_capacity(lines);
    loop {
        let block = item(rng);
        if out.len() + block.len() > lines {
            break;
        }
        out.extend(block);
    }
    while out.len() < lines {
        out.push(format!("{pad}{}", sentence(rng, 6)));
    }
    let mut text = out.join("\n");
    text.push('\n');
    text
}

/// Text of at least `bytes` bytes made of whole items.
fn text_with_bytes(rng: &mut Rng, bytes: usize, item: fn(&mut Rng) -> Vec<String>) -> String {
    let mut text = String::with_capacity(bytes + 512);
    while text.len() < bytes {
        for line in item(rng) {
            text.push_str(&line);
            text.push('\n');
        }
    }
    text
}

fn json_text(rng: &mut Rng, lines: usize) -> String {
    let entries = lines.saturating_sub(2).max(1);
    let mut text = String::from("{\n");
    for index in 0..entries {
        let comma = if index + 1 < entries { "," } else { "" };
        let _ = writeln!(
            text,
            "  \"{}_{index}\": {{ \"enabled\": {}, \"size\": {} }}{comma}",
            rng.pick(NOUNS),
            rng.chance(50),
            rng.range(0, 4096)
        );
    }
    text.push_str("}\n");
    text
}

/// Rust source with exactly `lines` lines.
pub fn rust_lines(rng: &mut Rng, lines: usize) -> String {
    text_with_lines(rng, lines, rust_item, "// ")
}

/// Rust source of at least `bytes` bytes.
pub fn rust_bytes(rng: &mut Rng, bytes: usize) -> String {
    text_with_bytes(rng, bytes, rust_item)
}

/// A text file of `lines` lines in a language picked by the file index.
fn source_file(rng: &mut Rng, lines: usize) -> (&'static str, String) {
    match rng.range(0, 99) {
        0..=39 => ("rs", rust_lines(rng, lines)),
        40..=64 => ("ts", text_with_lines(rng, lines, ts_item, "// ")),
        65..=84 => ("py", text_with_lines(rng, lines, py_item, "# ")),
        85..=92 => ("md", text_with_lines(rng, lines, md_item, "")),
        _ => ("json", json_text(rng, lines)),
    }
}

/// `count` folder names (relative), two levels deep.
fn folders(rng: &mut Rng, count: usize) -> Vec<PathBuf> {
    let top = (count as f64).sqrt().ceil().max(1.0) as usize;
    let mut dirs = Vec::with_capacity(count);
    let mut index = 0;
    'outer: for t in 0..top {
        let parent = PathBuf::from(format!("{}_{t:02}", rng.pick(NOUNS)));
        dirs.push(parent.clone());
        index += 1;
        if index >= count {
            break;
        }
        for s in 0..top {
            dirs.push(parent.join(format!("{}_{s:02}", rng.pick(NOUNS))));
            index += 1;
            if index >= count {
                break 'outer;
            }
        }
    }
    dirs
}

fn write_file(root: &Path, relative: &Path, content: &[u8]) -> Result<()> {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creando {}", parent.display()))?;
    }
    fs::write(&path, content).with_context(|| format!("escribiendo {}", path.display()))
}

fn random_bytes(rng: &mut Rng, len: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(len + 8);
    while bytes.len() < len {
        bytes.extend_from_slice(&rng.next_u64().to_le_bytes());
    }
    bytes.truncate(len);
    bytes
}

// ---------------------------------------------------------------- PNG

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = Vec::with_capacity(4 + data.len());
    body.extend_from_slice(kind);
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
}

/// A valid RGB PNG (zlib "stored" blocks, no compression library needed).
pub fn png(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(((width * 3 + 1) * height) as usize);
    for y in 0..height {
        raw.push(0);
        for x in 0..width {
            raw.extend_from_slice(&pixel(x, y));
        }
    }
    let mut zlib = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
    for (index, block) in blocks.iter().enumerate() {
        zlib.push(u8::from(index + 1 == blocks.len()));
        let len = block.len() as u16;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    png_chunk(&mut out, b"IHDR", &header);
    png_chunk(&mut out, b"IDAT", &zlib);
    png_chunk(&mut out, b"IEND", &[]);
    out
}

// ---------------------------------------------------------------- parts

/// Every regular file under `root` (relative, sorted), `.git` excluded.
pub fn list_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        let dir = root.join(&relative);
        for entry in fs::read_dir(&dir).with_context(|| format!("leyendo {}", dir.display()))? {
            let entry = entry?;
            let name = entry.file_name();
            if name == ".git" {
                continue;
            }
            let path = relative.join(&name);
            let kind = entry.file_type()?;
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file() {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Files, bytes and the tree hash of `root`.
pub fn manifest(root: &Path) -> Result<(usize, u64, String)> {
    let files = list_files(root)?;
    let mut hasher = Sha256::new();
    let mut bytes = 0u64;
    for relative in &files {
        let content = fs::read(root.join(relative))?;
        bytes += content.len() as u64;
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update((content.len() as u64).to_le_bytes());
        hasher.update(&content);
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    Ok((files.len(), bytes, hex))
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(root)
        .args([
            "-c",
            "init.defaultBranch=main",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.autocrlf=false",
            "-c",
            "core.fileMode=true",
        ])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", GIT_NAME)
        .env("GIT_AUTHOR_EMAIL", GIT_EMAIL)
        .env("GIT_AUTHOR_DATE", GIT_DATE)
        .env("GIT_COMMITTER_NAME", GIT_NAME)
        .env("GIT_COMMITTER_EMAIL", GIT_EMAIL)
        .env("GIT_COMMITTER_DATE", GIT_DATE)
        .output()
        .context("no se pudo ejecutar git")?;
    if !output.status.success() {
        bail!(
            "git {} falló: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Makes `root` a git repository with one commit of everything in it.
fn commit_all(root: &Path, message: &str) -> Result<String> {
    git(root, &["init", "-q"])?;
    git(root, &["add", "-A"])?;
    git(root, &["commit", "-q", "-m", message])?;
    git(root, &["rev-parse", "HEAD"])
}

fn finish(name: &'static str, root: &Path, commit: Option<String>) -> Result<Part> {
    let (files, bytes, sha256) = manifest(root)?;
    Ok(Part {
        name,
        files,
        bytes,
        sha256,
        commit,
    })
}

/// The three big files of `archivos/`, in order.
fn big_files(scale: &Scale) -> Vec<(&'static str, String)> {
    let mut rng = Rng::new(SEED ^ 0xA5C1_1705);
    vec![
        (
            FIVE_THOUSAND,
            rust_lines(&mut rng, scale.five_thousand_lines),
        ),
        (ONE_MEGABYTE, rust_bytes(&mut rng, scale.one_megabyte_bytes)),
        (
            FIFTY_THOUSAND,
            rust_lines(&mut rng, scale.fifty_thousand_lines),
        ),
    ]
}

fn generate_files(root: &Path, scale: &Scale) -> Result<Part> {
    for (name, text) in big_files(scale) {
        write_file(root, Path::new(name), text.as_bytes())?;
    }
    finish("archivos", root, None)
}

fn generate_medium(root: &Path, options: &Options) -> Result<Part> {
    let scale = &options.scale;
    let mut rng = Rng::new(SEED);
    let dirs = folders(&mut rng, scale.medium_dirs);
    for index in 0..scale.medium_files {
        let dir = &dirs[index % dirs.len()];
        let lines = rng.range(scale.medium_lines.0, scale.medium_lines.1);
        let (extension, text) = source_file(&mut rng, lines);
        let name = format!("{}_{index:04}.{extension}", rng.pick(NOUNS));
        write_file(root, &dir.join(name), text.as_bytes())?;
    }
    for index in 0..scale.medium_pngs {
        let (r, g, b) = (
            rng.range(0, 255) as u8,
            rng.range(0, 255) as u8,
            rng.range(0, 255) as u8,
        );
        let image = png(24, 24, |x, y| {
            if (x / 6 + y / 6) % 2 == 0 {
                [r, g, b]
            } else {
                [255 - r, 255 - g, 255 - b]
            }
        });
        write_file(
            root,
            &PathBuf::from(format!("imagenes/icono_{index}.png")),
            &image,
        )?;
    }
    for (name, text) in big_files(scale) {
        write_file(root, &Path::new("bench").join(name), text.as_bytes())?;
    }
    write_file(
        root,
        Path::new("README.md"),
        b"# Proyecto mediano\n\nCorpus de medicion de Cincel (generado, no editar).\n",
    )?;
    let commit = if options.git {
        Some(commit_all(root, "Corpus de medición")?)
    } else {
        None
    };
    finish("medio", root, commit)
}

fn generate_large(root: &Path, scale: &Scale) -> Result<Part> {
    let mut rng = Rng::new(SEED ^ 0x6A0D_E000);
    let dirs = folders(&mut rng, scale.large_dirs);
    let text_files = scale.large_files.saturating_sub(scale.large_binaries);
    for index in 0..text_files {
        let dir = &dirs[index % dirs.len()];
        let bytes = rng.range(scale.large_text_bytes.0, scale.large_text_bytes.1);
        let text = if rng.chance(50) {
            rust_bytes(&mut rng, bytes)
        } else {
            text_with_bytes(&mut rng, bytes, ts_item)
        };
        let extension = if text.starts_with("export") {
            "ts"
        } else {
            "rs"
        };
        let name = format!("{}_{index:05}.{extension}", rng.pick(NOUNS));
        write_file(root, &dir.join(name), text.as_bytes())?;
    }
    for index in 0..scale.large_binaries {
        let len = rng.range(scale.large_binary_bytes.0, scale.large_binary_bytes.1);
        let data = random_bytes(&mut rng, len);
        write_file(
            root,
            &PathBuf::from(format!("binarios/blob_{index:02}.bin")),
            &data,
        )?;
    }
    finish("grande", root, None)
}

/// The demo project for the screenshots (D21): a tiny calculator. Its
/// contents avoid `;` in the changed files because the turn file uses the
/// `FAKE_SHELL_EDITS` format, whose items are separated by `;`.
pub const DEMO_FILES: &[(&str, &str)] = &[
    (
        "Cargo.toml",
        "[package]\nname = \"calculadora\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    ),
    (
        "README.md",
        "# Calculadora\n\nUna calculadora de consola pequeña.\n\nUso: `calculadora \"2 + 3 * 4\"`\n",
    ),
    (
        "src/main.rs",
        "use calculadora::evaluar;\n\nfn main() {\n    let entrada: String = std::env::args().skip(1).collect::<Vec<_>>().join(\" \");\n    match evaluar(&entrada) {\n        Ok(valor) => println!(\"{valor}\"),\n        Err(error) => eprintln!(\"error: {error}\"),\n    }\n}\n",
    ),
    (
        "src/lib.rs",
        "pub mod error;\npub mod eval;\npub mod lexer;\npub mod parser;\n\npub use eval::evaluar;\n",
    ),
    (
        "src/lexer.rs",
        "#[derive(Debug, Clone, PartialEq)]\npub enum Token {\n    Numero(f64),\n    Mas,\n    Menos,\n    Por,\n    Entre,\n}\n\npub fn separar(texto: &str) -> Vec<Token> {\n    let mut tokens = Vec::new();\n    for parte in texto.split_whitespace() {\n        let token = match parte {\n            \"+\" => Token::Mas,\n            \"-\" => Token::Menos,\n            \"*\" => Token::Por,\n            \"/\" => Token::Entre,\n            numero => Token::Numero(numero.parse().unwrap_or(0.0)),\n        };\n        tokens.push(token);\n    }\n    tokens\n}\n",
    ),
    (
        "src/parser.rs",
        "use crate::lexer::Token;\n\npub fn prioridad(token: &Token) -> u8 {\n    match token {\n        Token::Por | Token::Entre => 2,\n        Token::Mas | Token::Menos => 1,\n        Token::Numero(_) => 0,\n    }\n}\n",
    ),
    (
        "src/eval.rs",
        "use crate::error::Error;\nuse crate::lexer::{separar, Token};\n\npub fn evaluar(texto: &str) -> Result<f64, Error> {\n    let tokens = separar(texto);\n    let mut total = 0.0;\n    let mut signo = 1.0;\n    for token in tokens {\n        match token {\n            Token::Numero(valor) => total += signo * valor,\n            Token::Menos => signo = -1.0,\n            _ => signo = 1.0,\n        }\n    }\n    Ok(total)\n}\n",
    ),
    (
        "tests/basicas.rs",
        "use calculadora::evaluar;\n\n#[test]\nfn suma_simple() {\n    assert_eq!(evaluar(\"2 + 3\").unwrap(), 5.0);\n}\n",
    ),
    (
        "src/error.rs",
        "#[derive(Debug)]\npub enum Error {\n    Vacio,\n}\n\nimpl std::fmt::Display for Error {\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\n        write!(f, \"la expresión está vacía\")\n    }\n}\n",
    ),
];

/// The synthetic turn of the demo: 3 files changed and 1 created, in the
/// `FAKE_SHELL_EDITS` format (`ruta=contenido`, `ruta=@contenido` to create,
/// `\n` for newlines, items separated by `;`).
pub fn demo_turn() -> String {
    let readme = "# Calculadora\\n\\nUna calculadora de consola pequeña, con historial.\\n\\nUso: `calculadora \"2 + 3 * 4\"`\\n\\nCada resultado queda guardado en el historial.\\n";
    let error = "#[derive(Debug)]\\npub enum Error {\\n    Vacio,\\n    DivisionPorCero,\\n}\\n\\nimpl std::fmt::Display for Error {\\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\\n        match self {\\n            Error::Vacio => write!(f, \"la expresión está vacía\"),\\n            Error::DivisionPorCero => write!(f, \"no se puede dividir por cero\"),\\n        }\\n    }\\n}\\n";
    let parser = "/// Mayor número, mayor prioridad.\\npub fn prioridad(token: &crate::lexer::Token) -> u8 {\\n    match token {\\n        crate::lexer::Token::Por | crate::lexer::Token::Entre => 3,\\n        crate::lexer::Token::Mas | crate::lexer::Token::Menos => 2,\\n        crate::lexer::Token::Numero(_) => 0,\\n    }\\n}\\n";
    let history = "/// Resultados anteriores, del más viejo al más nuevo.\\n#[derive(Debug, Default)]\\npub struct Historial {\\n    pub valores: Vec<f64>,\\n}\\n\\nimpl Historial {\\n    pub fn ultimo(&self) -> Option<f64> {\\n        self.valores.last().copied()\\n    }\\n}\\n";
    format!(
        "README.md={readme};src/error.rs={error};src/parser.rs={parser};src/historial.rs=@{history}"
    )
}

fn generate_demo(root: &Path, git_enabled: bool) -> Result<Part> {
    for (name, content) in DEMO_FILES {
        write_file(root, Path::new(name), content.as_bytes())?;
    }
    let commit = if git_enabled {
        Some(commit_all(root, "Calculadora inicial")?)
    } else {
        None
    };
    finish("demo", root, commit)
}

fn reset(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_dir_all(path).with_context(|| format!("borrando {}", path.display()))?;
    }
    fs::create_dir_all(path).with_context(|| format!("creando {}", path.display()))
}

/// Generates the corpus in `dir` (its parts are replaced if they exist; the
/// rest of `dir` is left alone). Returns one summary per part.
pub fn generate(dir: &Path, options: &Options) -> Result<Vec<Part>> {
    fs::create_dir_all(dir).with_context(|| format!("creando {}", dir.display()))?;
    let mut parts = Vec::new();

    let files_dir = dir.join("archivos");
    reset(&files_dir)?;
    parts.push(generate_files(&files_dir, &options.scale)?);

    let medium = dir.join("medio");
    reset(&medium)?;
    parts.push(generate_medium(&medium, options)?);

    if options.large {
        let large = dir.join("grande");
        reset(&large)?;
        parts.push(generate_large(&large, &options.scale)?);
    }
    if options.demo {
        let demo = dir.join("demo");
        reset(&demo)?;
        parts.push(generate_demo(&demo, options.git)?);
        let mut file = fs::File::create(dir.join("demo-turn.txt"))?;
        file.write_all(demo_turn().as_bytes())?;
    }
    Ok(parts)
}
