//! Tests for `docs/specs/modulos/syntax.md`.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use asteroid_text::Buffer;

use super::*;

/// One sample per language, plus the captures it must produce.
struct Sample {
    language: &'static str,
    text: &'static str,
    required: &'static [HighlightId],
}

const SAMPLES: &[Sample] = &[
    Sample {
        language: "rust",
        text: "// un comentario\nfn main() {\n    let saludo = \"hola\";\n    println!(\"{saludo}\");\n}\n",
        required: &[
            HighlightId::Keyword,
            HighlightId::String,
            HighlightId::Comment,
        ],
    },
    Sample {
        language: "typescript",
        text: "// un comentario\nconst saludo: string = \"hola\";\nexport function f(): number { return 1; }\n",
        required: &[
            HighlightId::Keyword,
            HighlightId::String,
            HighlightId::Comment,
        ],
    },
    Sample {
        language: "tsx",
        text: "// un comentario\nconst App = () => <div className=\"caja\">hola</div>;\n",
        required: &[
            HighlightId::Keyword,
            HighlightId::String,
            HighlightId::Comment,
        ],
    },
    Sample {
        language: "javascript",
        text: "// un comentario\nconst saludo = \"hola\";\nfunction f() { return saludo; }\n",
        required: &[
            HighlightId::Keyword,
            HighlightId::String,
            HighlightId::Comment,
        ],
    },
    Sample {
        language: "json",
        text: "{\n  \"nombre\": \"asteroid\",\n  \"version\": 1\n}\n",
        required: &[HighlightId::String, HighlightId::Number],
    },
    Sample {
        language: "toml",
        text: "# un comentario\n[paquete]\nnombre = \"asteroid\"\nversion = 1\n",
        required: &[HighlightId::String, HighlightId::Comment],
    },
    Sample {
        language: "yaml",
        text: "# un comentario\nnombre: \"asteroid\"\nlista:\n  - uno\n",
        required: &[HighlightId::String, HighlightId::Comment],
    },
    Sample {
        language: "markdown",
        text: "# Titulo\n\nUn parrafo con `codigo`.\n\n```rust\nfn f() {}\n```\n",
        required: &[HighlightId::Punctuation],
    },
    Sample {
        language: "html",
        text: "<!-- un comentario -->\n<div class=\"caja\">hola</div>\n",
        required: &[HighlightId::String, HighlightId::Comment],
    },
    Sample {
        language: "css",
        text: "/* un comentario */\n.caja {\n  color: #ff0000;\n  content: \"hola\";\n}\n",
        required: &[HighlightId::String, HighlightId::Comment],
    },
    Sample {
        language: "python",
        text: "# un comentario\ndef f(x):\n    saludo = \"hola\"\n    return saludo\n",
        required: &[
            HighlightId::Keyword,
            HighlightId::String,
            HighlightId::Comment,
        ],
    },
    Sample {
        language: "go",
        text: "// un comentario\npackage main\n\nfunc main() {\n\tsaludo := \"hola\"\n\t_ = saludo\n}\n",
        required: &[
            HighlightId::Keyword,
            HighlightId::String,
            HighlightId::Comment,
        ],
    },
    Sample {
        language: "bash",
        text: "# un comentario\nif true; then\n  echo \"hola\"\nfi\n",
        required: &[
            HighlightId::Keyword,
            HighlightId::String,
            HighlightId::Comment,
        ],
    },
    Sample {
        language: "c",
        text: "// un comentario\n#include <stdio.h>\n\nint main(void) {\n  const char *s = \"hola\";\n  return 0;\n}\n",
        required: &[
            HighlightId::Keyword,
            HighlightId::String,
            HighlightId::Comment,
        ],
    },
    Sample {
        language: "cpp",
        text: "// un comentario\n#include <string>\n\nclass A {\n public:\n  std::string s = \"hola\";\n};\n",
        required: &[
            HighlightId::Keyword,
            HighlightId::String,
            HighlightId::Comment,
        ],
    },
    Sample {
        language: "java",
        text: "// un comentario\nclass A {\n  String s = \"hola\";\n}\n",
        required: &[
            HighlightId::Keyword,
            HighlightId::String,
            HighlightId::Comment,
        ],
    },
    Sample {
        language: "sql",
        text: "-- un comentario\nSELECT nombre FROM tabla WHERE nombre = 'hola';\n",
        required: &[
            HighlightId::Keyword,
            HighlightId::String,
            HighlightId::Comment,
        ],
    },
    Sample {
        language: "dockerfile",
        text: "# un comentario\nFROM alpine:3.20\nRUN echo \"hola\"\nCMD [\"sh\"]\n",
        required: &[
            HighlightId::Keyword,
            HighlightId::String,
            HighlightId::Comment,
        ],
    },
];

fn registry() -> Arc<LanguageRegistry> {
    Arc::new(LanguageRegistry::new())
}

fn parse(registry: &Arc<LanguageRegistry>, language: &str, text: &str) -> SyntaxState {
    let language = registry
        .language(language)
        .unwrap_or_else(|| panic!("language {language} is not in the registry"));
    if let Err(error) = language.grammar() {
        panic!("{}: {error}", language.name());
    }
    let buffer = Buffer::new(text);
    let mut state = SyntaxState::new(registry.clone(), language, buffer.snapshot());
    let outcome = state.reparse(&CancelFlag::new());
    assert!(
        matches!(outcome, ParseOutcome::Parsed { .. }),
        "unexpected parse outcome: {outcome:?}"
    );
    state
}

fn captures(state: &SyntaxState) -> BTreeSet<HighlightId> {
    state
        .highlights(0..state.snapshot().len())
        .into_iter()
        .map(|(_, id)| id)
        .collect()
}

// -------------------------------------------------------------- the 18 grammars

#[test]
fn every_language_compiles_its_queries() {
    let registry = registry();
    assert_eq!(registry.languages().len(), 18, "the 18 v1 languages");
    for language in registry.languages() {
        if let Err(error) = language.grammar() {
            panic!("{} failed to compile: {error}", language.name());
        }
    }
}

#[test]
fn every_language_sample_yields_its_captures() {
    let registry = registry();
    assert_eq!(SAMPLES.len(), registry.languages().len());
    for sample in SAMPLES {
        let state = parse(&registry, sample.language, sample.text);
        let found = captures(&state);
        for required in sample.required {
            assert!(
                found.contains(required),
                "{}: expected a `{required}` capture, got {found:?}",
                sample.language
            );
        }
    }
}

#[test]
fn spans_are_ascending_disjoint_and_inside_the_range() {
    let registry = registry();
    for sample in SAMPLES {
        let state = parse(&registry, sample.language, sample.text);
        let len = state.snapshot().len();
        let spans = state.highlights(0..len);
        let mut previous_end = 0;
        for (range, _) in &spans {
            assert!(
                range.start >= previous_end,
                "{}: overlapping spans around {range:?}",
                sample.language
            );
            assert!(range.start < range.end, "{}: empty span", sample.language);
            assert!(range.end <= len, "{}: span past the text", sample.language);
            previous_end = range.end;
        }
    }
}

// ---------------------------------------------------------------- the registry

#[test]
fn detects_by_extension() {
    let registry = registry();
    for (path, expected) in [
        ("src/main.rs", "rust"),
        ("app/index.ts", "typescript"),
        ("app/App.tsx", "tsx"),
        ("app/index.mjs", "javascript"),
        ("package.json", "json"),
        ("Cargo.toml", "toml"),
        ("ci/config.yml", "yaml"),
        ("README.md", "markdown"),
        ("web/index.html", "html"),
        ("web/estilo.css", "css"),
        ("tool.py", "python"),
        ("cmd/main.go", "go"),
        ("scripts/build.sh", "bash"),
        ("src/main.c", "c"),
        ("src/main.cpp", "cpp"),
        ("src/Main.java", "java"),
        ("db/schema.sql", "sql"),
        ("build/app.dockerfile", "dockerfile"),
        ("SRC/MAIN.RS", "rust"),
    ] {
        let found = registry.language_for_path(path);
        assert_eq!(
            found.as_ref().map(|language| language.name()),
            Some(expected),
            "{path}"
        );
    }
}

#[test]
fn detects_by_file_name() {
    let registry = registry();
    for name in [
        "Dockerfile",
        "dockerfile",
        "Dockerfile.dev",
        "Containerfile",
    ] {
        assert_eq!(
            registry
                .language_for_path(name)
                .as_ref()
                .map(|language| language.name()),
            Some("dockerfile"),
            "{name}"
        );
    }
    // `Makefile` is a file-name rule with no v1 grammar: plain text.
    assert!(registry.language_for_path("Makefile").is_none());
    assert!(registry.language_for_path("notas.txt").is_none());
    assert!(registry.language_for_path("sin-extension").is_none());
}

#[test]
fn detects_by_shebang() {
    let registry = registry();
    for (line, expected) in [
        ("#!/usr/bin/env python3", Some("python")),
        ("#!/usr/bin/python3.12", Some("python")),
        ("#!/bin/bash", Some("bash")),
        ("#!/bin/sh -e", Some("bash")),
        (
            "#!/usr/bin/env -S node --experimental-modules",
            Some("javascript"),
        ),
        ("#!/usr/bin/env perl", None),
        ("no es un shebang", None),
    ] {
        assert_eq!(
            registry
                .language_for_shebang(line)
                .as_ref()
                .map(|language| language.name()),
            expected,
            "{line}"
        );
    }

    // The combined lookup falls back to the first line.
    assert_eq!(
        registry
            .language_for("bin/deploy", Some("#!/usr/bin/env bash"))
            .as_ref()
            .map(|language| language.name()),
        Some("bash")
    );
}

#[test]
fn looks_languages_up_by_name_and_alias() {
    let registry = registry();
    for (name, expected) in [
        ("rust", "rust"),
        ("rs", "rust"),
        ("sh", "bash"),
        ("c++", "cpp"),
        ("yml", "yaml"),
        ("md", "markdown"),
        ("TypeScript", "typescript"),
    ] {
        assert_eq!(
            registry.language(name).as_ref().map(|l| l.name()),
            Some(expected),
            "{name}"
        );
    }
    assert!(registry.language("brainfuck").is_none());
}

// ------------------------------------------------------------------ injections

#[test]
fn markdown_code_fences_are_injected() {
    let registry = registry();
    let text = "Texto normal.\n\n```rust\nfn main() { let x = 1; }\n```\n";
    let state = parse(&registry, "markdown", text);
    let fence_start = text.find("fn main").expect("the fence body");
    let spans = state.highlights(0..text.len());
    let keyword = spans
        .iter()
        .find(|(range, id)| *id == HighlightId::Keyword && range.start >= fence_start);
    assert!(
        keyword.is_some(),
        "no Rust keyword inside the fence; got {spans:?}"
    );
}

#[test]
fn html_script_and_style_are_injected() {
    let registry = registry();
    let text =
        "<html>\n<style>.caja { color: red; }</style>\n<script>const x = 1;</script>\n</html>\n";
    let state = parse(&registry, "html", text);
    let script_start = text.find("const x").expect("the script body");
    let spans = state.highlights(0..text.len());
    assert!(
        spans
            .iter()
            .any(|(range, id)| *id == HighlightId::Keyword && range.start >= script_start),
        "no JavaScript keyword inside <script>; got {spans:?}"
    );
    let style_start = text.find(".caja").expect("the style body");
    let style_end = text.find("</style>").expect("the style end");
    assert!(
        spans
            .iter()
            .any(|(range, _)| range.start >= style_start && range.end <= style_end),
        "nothing highlighted inside <style>; got {spans:?}"
    );
}

// ------------------------------------------------------- incremental behaviour

#[test]
fn highlights_are_restricted_to_the_requested_range() {
    let registry = registry();
    let text = "fn a() {}\nfn b() {}\nfn c() {}\n";
    let state = parse(&registry, "rust", text);
    let second_line = 10..20;
    for (range, _) in state.highlights(second_line.clone()) {
        assert!(
            range.end > second_line.start && range.start < second_line.end,
            "{range:?} is outside {second_line:?}"
        );
    }
    assert!(state.highlights(0..0).is_empty());
}

#[test]
fn an_edit_is_reparsed_incrementally() {
    let registry = registry();
    let mut buffer = Buffer::new("fn main() {\n    let x = 1;\n}\n");
    let language = registry.language("rust").unwrap();
    let mut state = SyntaxState::new(registry.clone(), language, buffer.snapshot());
    state.reparse(&CancelFlag::new());
    assert_eq!(state.parsed_version(), Some(0));
    assert!(!state.is_dirty());

    buffer.record_events(true);
    let offset = buffer.text().find("let").unwrap();
    buffer.insert(offset, "struct S;\n    ");

    for event in buffer.drain_events() {
        state.apply_event(&event, buffer.snapshot());
    }
    assert!(state.is_dirty());
    assert!(matches!(
        state.reparse(&CancelFlag::new()),
        ParseOutcome::Parsed { .. }
    ));
    assert_eq!(state.parsed_version(), Some(buffer.version()));
    assert_eq!(
        state.reparse(&CancelFlag::new()),
        ParseOutcome::Unchanged,
        "a second reparse without edits is a no-op"
    );

    // The new `struct` keyword has to show up in the incremental tree.
    let spans = state.highlights(0..buffer.len_bytes());
    assert!(
        spans
            .iter()
            .any(|(range, id)| *id == HighlightId::Keyword
                && buffer.text_in(range.clone()) == "struct"),
        "the incremental tree did not pick up the new keyword: {spans:?}"
    );
}

#[test]
fn multi_line_edits_keep_the_tree_consistent() {
    let registry = registry();
    let mut buffer = Buffer::new("fn a() {}\nfn b() {}\nfn c() {}\n");
    let language = registry.language("rust").unwrap();
    let mut state = SyntaxState::new(registry.clone(), language.clone(), buffer.snapshot());
    state.reparse(&CancelFlag::new());

    buffer.record_events(true);
    buffer.replace(10..19, "// comentario\nstruct S;\n");

    for event in buffer.drain_events() {
        state.apply_event(&event, buffer.snapshot());
    }
    state.reparse(&CancelFlag::new());

    // The incremental result must match a parse from scratch.
    let incremental = state.highlights(0..buffer.len_bytes());
    let cold = parse(&registry, "rust", &buffer.text()).highlights(0..buffer.len_bytes());
    assert_eq!(incremental, cold);
}

#[test]
fn a_raised_cancel_flag_stops_the_parse() {
    let registry = registry();
    let language = registry.language("rust").unwrap();
    let buffer = Buffer::new(&"fn f() { let x = 1; }\n".repeat(2000));
    let mut state = SyntaxState::new(registry, language, buffer.snapshot());

    let cancel = CancelFlag::new();
    cancel.cancel();
    assert_eq!(state.reparse(&cancel), ParseOutcome::Cancelled);
    assert!(state.tree().is_none());

    // A fresh flag still gets a tree.
    assert!(matches!(
        state.reparse(&CancelFlag::new()),
        ParseOutcome::Parsed { .. }
    ));
}

// ----------------------------------------------------------------- the theme

#[test]
fn capture_names_map_onto_the_twelve_highlights() {
    for (capture, expected) in [
        ("keyword", Some(HighlightId::Keyword)),
        ("keyword.function", Some(HighlightId::Keyword)),
        ("function.method.builtin", Some(HighlightId::Function)),
        ("variable.parameter", Some(HighlightId::Variable)),
        ("type.builtin", Some(HighlightId::Type)),
        ("string.special.path", Some(HighlightId::String)),
        ("constant.builtin", Some(HighlightId::Constant)),
        ("punctuation.bracket", Some(HighlightId::Punctuation)),
        ("attribute", Some(HighlightId::Attribute)),
        ("property", Some(HighlightId::Property)),
        ("operator", Some(HighlightId::Operator)),
        ("number", Some(HighlightId::Number)),
        ("comment.documentation", Some(HighlightId::Comment)),
        ("text.title", None),
        ("local.scope", None),
    ] {
        assert_eq!(
            HighlightId::from_capture_name(capture),
            expected,
            "{capture}"
        );
    }
}

#[test]
fn the_default_theme_is_one_dark() {
    let theme = HighlightTheme::default();
    assert_eq!(theme, HighlightTheme::one_dark());
    assert_eq!(theme.color(HighlightId::Keyword), [0xc6, 0x78, 0xdd]);
    assert_eq!(theme.color(HighlightId::String), [0x98, 0xc3, 0x79]);
    assert_eq!(theme.color(HighlightId::Comment), [0x5c, 0x63, 0x70]);
    assert_eq!(theme.color_u32(HighlightId::Keyword), 0x00c6_78dd);

    let mut theme = theme;
    theme.set_color(HighlightId::Keyword, [1, 2, 3]);
    assert_eq!(theme.color(HighlightId::Keyword), [1, 2, 3]);

    // Every capture has a colour and a stable index.
    for (index, id) in HighlightId::ALL.iter().enumerate() {
        assert_eq!(id.index(), index);
        assert!(!id.name().is_empty());
    }
}

#[test]
fn syntax_state_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<SyntaxState>();
    assert_send::<CancelFlag>();
    fn assert_sync<T: Sync>() {}
    assert_sync::<LanguageRegistry>();
    assert_sync::<HighlightTheme>();
}

// ---------------------------------------------------------------- performance

/// Builds a Rust file of roughly 5 000 lines.
fn rust_5000_lines() -> String {
    let mut text = String::new();
    let mut n = 0;
    while text.lines().count() < 5000 {
        text.push_str(&format!(
            "/// Documenta la funcion {n}.\n\
             pub fn funcion_{n}(entrada: &str, veces: usize) -> String {{\n\
             \x20   let mut salida = String::from(\"{n}\");\n\
             \x20   for i in 0..veces {{\n\
             \x20       salida.push_str(entrada);\n\
             \x20       if i % 2 == 0 {{ salida.push('-'); }}\n\
             \x20   }}\n\
             \x20   salida\n\
             }}\n\n"
        ));
        n += 1;
    }
    text
}

/// Acceptance criterion: a 5 000-line Rust file parses cold in < 30 ms and a
/// one-character edit re-parses in < 2 ms. Measured numbers are printed, so
/// `cargo test -- --nocapture` shows the margin.
#[test]
fn rust_5000_lines_parses_cold_and_reparses_fast() {
    let registry = registry();
    let language = registry.language("rust").unwrap();
    // Compile the queries first: that is startup cost, not parse cost.
    language.grammar().expect("rust grammar");

    let text = rust_5000_lines();
    let mut buffer = Buffer::new(&text);
    buffer.record_events(true);
    let lines = buffer.line_count();
    assert!(lines >= 5000, "the sample has {lines} lines");

    let mut state = SyntaxState::new(registry.clone(), language, buffer.snapshot());
    let started = Instant::now();
    let outcome = state.reparse(&CancelFlag::new());
    let cold = started.elapsed();
    assert!(matches!(outcome, ParseOutcome::Parsed { .. }));

    // Twelve one-character edits in the middle of the file. The very first
    // one after a cold parse is the slowest (cold caches), so the median is
    // what "typing latency" actually looks like.
    let mut warm = Vec::new();
    for i in 0..12 {
        let offset = buffer.clip_offset(buffer.len_bytes() / 2 + i);
        buffer.insert(offset, "x");
        for event in buffer.drain_events() {
            state.apply_event(&event, buffer.snapshot());
        }
        let started = Instant::now();
        let outcome = state.reparse(&CancelFlag::new());
        warm.push(started.elapsed());
        assert!(matches!(outcome, ParseOutcome::Parsed { .. }));
    }
    warm.sort();
    let median = warm[warm.len() / 2];

    println!(
        "rust {lines} lines ({} KiB): cold {cold:?}; single-char reparse min {:?} median {median:?} max {:?}",
        buffer.len_bytes() / 1024,
        warm[0],
        warm[warm.len() - 1],
    );
    assert!(cold < Duration::from_millis(30), "cold parse took {cold:?}");
    assert!(
        median < Duration::from_millis(2),
        "median single-char reparse took {median:?}"
    );
}
