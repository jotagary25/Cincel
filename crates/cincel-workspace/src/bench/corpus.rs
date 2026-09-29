//! The corpus `cincel --bench` generates for itself when the command line
//! does not name one (`docs/specs/08-etapa6-cierre-1-0.md` §3.3, §3.4).
//!
//! Everything is deterministic (the content is a function of the line or file
//! index, there is no random source) and lives in the bench's own temporary
//! directory, outside the repository, which is deleted when the process
//! ends. The shapes follow §3.4:
//!
//! * `archivos/`: `cinco-mil.rs` (5 000 lines), `un-mega.rs` (1 MiB) and
//!   `cincuenta-mil.rs` (50 000 lines, over 2 MiB, so over the inline review
//!   limits), plus a README: the project the file scenarios open.
//! * `grande/`: 50 000 small files in 500 folders (M10: "50 000 rutas"), for
//!   `finder` and `sweep`.
//! * `demo/` and `demo-turn.txt`: a small made-up calculator and the changes
//!   of one sample turn, in the `FAKE_SHELL_EDITS` format.

use std::io::Write as _;
use std::path::{Path, PathBuf};

/// How big each generated piece is. [`CorpusSizes::default`] is the real
/// bench; the tests shrink everything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CorpusSizes {
    /// Lines of `cinco-mil.rs`.
    pub five_k_lines: usize,
    /// Minimum size in bytes of `un-mega.rs`.
    pub one_mb_bytes: usize,
    /// Lines of `cincuenta-mil.rs`.
    pub fifty_k_lines: usize,
    /// Files of `grande/`.
    pub large_files: usize,
    /// Folders `grande/` spreads them over.
    pub large_folders: usize,
}

impl Default for CorpusSizes {
    fn default() -> Self {
        Self {
            five_k_lines: 5_000,
            one_mb_bytes: 1024 * 1024,
            fifty_k_lines: 50_000,
            large_files: 50_000,
            large_folders: 500,
        }
    }
}

/// The three files of `archivos/` and the folder that holds them.
#[derive(Clone, Debug)]
pub struct TextFiles {
    /// `archivos/`, opened as the project of the file scenarios.
    pub root: PathBuf,
    /// 5 000 lines (M3, M4).
    pub five_k: PathBuf,
    /// 1 MiB (M5).
    pub one_mb: PathBuf,
    /// 50 000 lines (M6).
    pub fifty_k: PathBuf,
}

impl TextFiles {
    /// The three files, smallest first.
    pub fn all(&self) -> Vec<PathBuf> {
        vec![
            self.five_k.clone(),
            self.one_mb.clone(),
            self.fifty_k.clone(),
        ]
    }
}

/// Stems the files of `grande/` rotate through, so the finder's queries
/// (`mod`, `main`, `parse`, `config`, `handler`) find something to rank.
const STEMS: [&str; 10] = [
    "main", "parser", "config", "handler", "model", "render", "server", "client", "utils", "state",
];

/// One line of made-up Rust: `fn` blocks of 24 lines whose body lines are
/// about 48 bytes long (a 50 000 line file lands near 2.4 MB, like §3.4's
/// "≈ 2 MB" and over the 2 MiB inline review limit).
pub fn code_line(index: usize) -> String {
    match index % 24 {
        0 => format!("pub fn bloque_{}(entrada: u64) -> u64 {{", index / 24),
        1 => "    let mut a = entrada;".to_string(),
        22 => "    a".to_string(),
        23 => "}".to_string(),
        _ => format!(
            "    a = a.wrapping_mul({}) ^ {}; // paso {index}",
            index % 89 + 3,
            (index * 7_919) % 65_521
        ),
    }
}

/// Writes `lines` lines of [`code_line`] to `path`.
fn write_lines(path: &Path, lines: usize) -> std::io::Result<()> {
    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    for index in 0..lines {
        writeln!(out, "{}", code_line(index))?;
    }
    out.flush()
}

/// Writes [`code_line`]s to `path` until it holds at least `bytes` bytes,
/// ending on a whole line.
fn write_bytes(path: &Path, bytes: usize) -> std::io::Result<()> {
    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut written = 0;
    let mut index = 0;
    while written < bytes {
        let line = code_line(index);
        writeln!(out, "{line}")?;
        written += line.len() + 1;
        index += 1;
    }
    out.flush()
}

/// Generates `dir/archivos/`.
pub fn text_files(dir: &Path, sizes: &CorpusSizes) -> std::io::Result<TextFiles> {
    let root = dir.join("archivos");
    std::fs::create_dir_all(&root)?;
    let files = TextFiles {
        five_k: root.join("cinco-mil.rs"),
        one_mb: root.join("un-mega.rs"),
        fifty_k: root.join("cincuenta-mil.rs"),
        root: root.clone(),
    };
    write_lines(&files.five_k, sizes.five_k_lines)?;
    write_bytes(&files.one_mb, sizes.one_mb_bytes)?;
    write_lines(&files.fifty_k, sizes.fifty_k_lines)?;
    std::fs::write(
        root.join("LEAME.md"),
        "# Corpus de medición\n\nArchivos generados por `cincel --bench`.\n",
    )?;
    Ok(files)
}

/// Generates `dir/grande/`: `files` files of about 400 bytes spread over
/// `folders` folders.
pub fn large_project(dir: &Path, files: usize, folders: usize) -> std::io::Result<PathBuf> {
    let root = dir.join("grande");
    let folders = folders.max(1);
    for folder in 0..folders {
        std::fs::create_dir_all(root.join(format!("modulo_{folder:03}")))?;
    }
    for index in 0..files {
        let folder = index % folders;
        let stem = STEMS[index % STEMS.len()];
        let path = root.join(format!("modulo_{folder:03}/{stem}_{index:05}.rs"));
        let mut content = String::with_capacity(512);
        for line in 0..8 {
            content.push_str(&code_line(index * 8 + line));
            content.push('\n');
        }
        std::fs::write(path, content)?;
    }
    std::fs::write(
        root.join("LEAME.md"),
        "# Proyecto grande\n\nGenerado por `cincel --bench`.\n",
    )?;
    Ok(root)
}

/// The demo project of §3.4 (a calculator with made-up names: eight files and
/// a README) under `dir/demo/`, and the sample turn (`dir/demo-turn.txt`,
/// three files changed and one created), which is also returned.
pub fn demo_project(dir: &Path) -> std::io::Result<(PathBuf, String)> {
    let root = dir.join("demo");
    std::fs::create_dir_all(root.join("src"))?;
    std::fs::create_dir_all(root.join("tests"))?;
    for (path, content) in DEMO_FILES {
        std::fs::write(root.join(path), content)?;
    }
    let turn = DEMO_TURN.to_string();
    std::fs::write(dir.join("demo-turn.txt"), &turn)?;
    Ok((root, turn))
}

/// Applies a `FAKE_SHELL_EDITS` list (the fake agent's format, see
/// `crates/cincel-acp/tests/fake_agent/main.rs`): `ruta=contenido`
/// overwrites, `ruta=@contenido` creates (parent folders included),
/// `ruta=` deletes and `ruta=>destino` renames; `\n` and `\t` in a content
/// stand for a newline and a tab. Returns the paths it touched, or the first
/// error.
pub fn apply_shell_edits(root: &Path, spec: &str) -> std::io::Result<Vec<PathBuf>> {
    let mut touched = Vec::new();
    for item in spec.split(';') {
        let item = item.trim();
        let Some((path, value)) = item.split_once('=') else {
            continue;
        };
        let path = root.join(path.trim());
        if value.is_empty() {
            std::fs::remove_file(&path)?;
        } else if let Some(destination) = value.strip_prefix('>') {
            let destination = root.join(destination.trim());
            std::fs::rename(&path, &destination)?;
            touched.push(destination);
        } else if let Some(content) = value.strip_prefix('@') {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, unescape(content))?;
        } else {
            std::fs::write(&path, unescape(value))?;
        }
        touched.push(path);
    }
    Ok(touched)
}

/// `\n` and `\t` of a `FAKE_SHELL_EDITS` content.
fn unescape(content: &str) -> String {
    content.replace("\\n", "\n").replace("\\t", "\t")
}

const DEMO_FILES: [(&str, &str); 9] = [
    (
        "Cargo.toml",
        "[package]\nname = \"calculadora\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    ),
    (
        "README.md",
        "# Calculadora\n\nUna calculadora de ejemplo: suma, resta, multiplica y divide.\n\n\
         ```sh\ncargo run -- \"2 + 3 * 4\"\n```\n",
    ),
    (
        "src/main.rs",
        "use calculadora::evaluar;\n\nfn main() {\n    let entrada = std::env::args().nth(1).unwrap_or_default();\n    \
         match evaluar(&entrada) {\n        Ok(valor) => println!(\"{valor}\"),\n        \
         Err(error) => eprintln!(\"error: {error}\"),\n    }\n}\n",
    ),
    (
        "src/lib.rs",
        "pub mod error;\npub mod eval;\npub mod lexer;\npub mod parser;\n\npub use eval::evaluar;\n",
    ),
    (
        "src/error.rs",
        "#[derive(Debug, PartialEq)]\npub enum Error {\n    Simbolo(char),\n    DivisionPorCero,\n    Incompleta,\n}\n\n\
         impl std::fmt::Display for Error {\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\n        \
         match self {\n            Error::Simbolo(c) => write!(f, \"símbolo inesperado: {c}\"),\n            \
         Error::DivisionPorCero => write!(f, \"división por cero\"),\n            \
         Error::Incompleta => write!(f, \"expresión incompleta\"),\n        }\n    }\n}\n",
    ),
    (
        "src/lexer.rs",
        "use crate::error::Error;\n\n#[derive(Debug, Clone, Copy, PartialEq)]\npub enum Ficha {\n    Numero(f64),\n    Mas,\n    Menos,\n    Por,\n    Entre,\n}\n\n\
         pub fn fichas(texto: &str) -> Result<Vec<Ficha>, Error> {\n    let mut salida = Vec::new();\n    \
         for c in texto.chars().filter(|c| !c.is_whitespace()) {\n        salida.push(match c {\n            \
         '+' => Ficha::Mas,\n            '-' => Ficha::Menos,\n            '*' => Ficha::Por,\n            \
         '/' => Ficha::Entre,\n            d if d.is_ascii_digit() => Ficha::Numero(f64::from(d as u8 - b'0')),\n            \
         otro => return Err(Error::Simbolo(otro)),\n        });\n    }\n    Ok(salida)\n}\n",
    ),
    (
        "src/parser.rs",
        "use crate::error::Error;\nuse crate::lexer::Ficha;\n\npub fn validar(fichas: &[Ficha]) -> Result<(), Error> {\n    \
         if fichas.is_empty() {\n        return Err(Error::Incompleta);\n    }\n    Ok(())\n}\n",
    ),
    (
        "src/eval.rs",
        "use crate::error::Error;\nuse crate::lexer::{Ficha, fichas};\n\npub fn evaluar(texto: &str) -> Result<f64, Error> {\n    \
         let fichas = fichas(texto)?;\n    crate::parser::validar(&fichas)?;\n    let mut total = 0.0;\n    \
         for ficha in fichas {\n        if let Ficha::Numero(n) = ficha {\n            total += n;\n        }\n    }\n    Ok(total)\n}\n",
    ),
    (
        "tests/basico.rs",
        "use calculadora::evaluar;\n\n#[test]\nfn suma_simple() {\n    assert_eq!(evaluar(\"2 + 3\"), Ok(5.0));\n}\n",
    ),
];

/// The sample turn: three files changed and one created. The format splits
/// items on `;`, so no content may hold one: the Rust in it is written with
/// expression bodies only.
const DEMO_TURN: &str = "src/error.rs=/// Todo lo que puede salir mal al evaluar una expresión.\\n\
#[derive(Debug, PartialEq)]\\npub enum Error {\\n    Simbolo(char),\\n    DivisionPorCero,\\n    Incompleta,\\n    \
Desborde,\\n}\\n\\nimpl std::fmt::Display for Error {\\n    \
fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\\n        match self {\\n            \
Error::Simbolo(c) => write!(f, \"símbolo inesperado: {c}\"),\\n            \
Error::DivisionPorCero => write!(f, \"no se puede dividir por cero\"),\\n            \
Error::Incompleta => write!(f, \"a la expresión le falta un número\"),\\n            \
Error::Desborde => write!(f, \"el resultado es demasiado grande\"),\\n        }\\n    }\\n}\\n\
;README.md=# Calculadora\\n\\nUna calculadora de ejemplo: suma, resta, multiplica y divide.\\n\\n\
```sh\\ncargo run -- \"2 + 3 * 4\"\\n```\\n\\nLos errores explican qué falta en la expresión.\\n\
;Cargo.toml=[package]\\nname = \"calculadora\"\\nversion = \"0.2.0\"\\nedition = \"2024\"\\n\
;src/historial.rs=@/// Las últimas operaciones evaluadas.\\n#[derive(Debug, Default)]\\npub struct Historial {\\n    \
entradas: Vec<(String, f64)>,\\n}\\n\\nimpl Historial {\\n    \
pub fn ultimas(&self) -> &[(String, f64)] {\\n        &self.entradas\\n    }\\n}\\n";
