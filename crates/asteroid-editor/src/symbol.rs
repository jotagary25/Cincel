//! The named definition enclosing the cursor, for the breadcrumb of the
//! workspace (`docs/specs/02-visual.md` §1: "src › main.rs › fn main").
//!
//! `asteroid-syntax` has no symbol API yet, so the node kinds live here: one
//! table per language, using tree-sitter's own node-kind names. Resolving a
//! symbol is a walk from the node at the cursor up to the root, stopping at the
//! first ancestor whose kind is a definition, and reading its name child.

use asteroid_text::BufferSnapshot;
use tree_sitter::Node;

/// Node kinds that count as a named definition, per language. The key is
/// [`asteroid_syntax::Language::name`].
///
/// An empty list means "this language has nothing worth naming in a
/// breadcrumb" (a Dockerfile is a flat list of instructions).
pub fn definition_kinds(language: &str) -> &'static [&'static str] {
    match language {
        "rust" => &[
            "function_item",
            "struct_item",
            "enum_item",
            "union_item",
            "trait_item",
            "impl_item",
            "mod_item",
            "type_item",
            "macro_definition",
        ],
        "python" => &["function_definition", "class_definition"],
        "javascript" => &[
            "function_declaration",
            "generator_function_declaration",
            "class_declaration",
            "method_definition",
        ],
        "typescript" | "tsx" => &[
            "function_declaration",
            "generator_function_declaration",
            "class_declaration",
            "abstract_class_declaration",
            "method_definition",
            "interface_declaration",
            "type_alias_declaration",
            "enum_declaration",
            "module",
            "internal_module",
        ],
        "go" => &[
            "function_declaration",
            "method_declaration",
            "type_declaration",
        ],
        "c" => &[
            "function_definition",
            "struct_specifier",
            "union_specifier",
            "enum_specifier",
            "type_definition",
        ],
        "cpp" => &[
            "function_definition",
            "class_specifier",
            "struct_specifier",
            "union_specifier",
            "enum_specifier",
            "namespace_definition",
            "type_definition",
        ],
        "java" => &[
            "class_declaration",
            "interface_declaration",
            "enum_declaration",
            "record_declaration",
            "method_declaration",
            "constructor_declaration",
        ],
        "bash" => &["function_definition"],
        "css" => &["rule_set", "keyframes_statement", "media_statement"],
        "html" => &["element"],
        "json" => &["pair"],
        "markdown" => &["atx_heading", "setext_heading"],
        "sql" => &["create_table", "create_view", "create_function"],
        "toml" => &["table", "table_array_element"],
        "yaml" => &["block_mapping_pair"],
        // Dockerfile is a flat instruction list: nothing encloses the cursor.
        "dockerfile" => &[],
        _ => &[],
    }
}

/// Kinds whose "name" is simply their own first line, because the grammar has
/// no identifier child (a Markdown heading, a CSS selector).
const TEXT_IS_THE_NAME: &[&str] = &[
    "atx_heading",
    "setext_heading",
    "rule_set",
    "keyframes_statement",
    "media_statement",
];

/// Fields that hold the name of a definition, tried in order. `type` is what
/// Rust's `impl_item` uses; `key` covers JSON pairs and TOML tables.
const NAME_FIELDS: &[&str] = &["name", "type", "key", "path", "declarator"];

/// How deep the identifier search goes when no field matches (C's declarator
/// chain, Go's `type_spec`).
const MAX_NAME_DEPTH: usize = 4;

/// Longest name returned, so a pathological node cannot fill a breadcrumb.
const MAX_NAME_LEN: usize = 120;

/// The innermost named definition containing `offset`.
///
/// Returns its name and the byte range of the whole definition node, both in
/// the coordinates of `snapshot` — which must be the snapshot `tree` was parsed
/// from, not necessarily the buffer's newest one.
pub fn symbol_at(
    tree: &tree_sitter::Tree,
    snapshot: &BufferSnapshot,
    language: &str,
    offset: usize,
) -> Option<(String, std::ops::Range<usize>)> {
    let kinds = definition_kinds(language);
    if kinds.is_empty() {
        return None;
    }
    let offset = offset.min(snapshot.len());
    let mut node = tree.root_node().descendant_for_byte_range(offset, offset)?;
    loop {
        if kinds.contains(&node.kind())
            && let Some(name) = definition_name(&node, snapshot)
        {
            return Some((name, node.start_byte()..node.end_byte().min(snapshot.len())));
        }
        node = node.parent()?;
    }
}

/// The name of a definition node.
fn definition_name(node: &Node, snapshot: &BufferSnapshot) -> Option<String> {
    for field in NAME_FIELDS {
        if let Some(child) = node.child_by_field_name(field)
            && let Some(text) = node_text(&child, snapshot)
        {
            return Some(text);
        }
    }
    if let Some(child) = first_identifier(node, snapshot, MAX_NAME_DEPTH) {
        return Some(child);
    }
    if TEXT_IS_THE_NAME.contains(&node.kind()) {
        return node_text(node, snapshot);
    }
    None
}

/// First descendant that looks like a name (`identifier`, `type_identifier`,
/// `bare_key`, `tag_name`, …), breadth first so the outermost one wins.
fn first_identifier(node: &Node, snapshot: &BufferSnapshot, depth: usize) -> Option<String> {
    if depth == 0 {
        return None;
    }
    let mut cursor = node.walk();
    let children: Vec<Node> = node.named_children(&mut cursor).collect();
    for child in &children {
        if is_name_kind(child.kind())
            && let Some(text) = node_text(child, snapshot)
        {
            return Some(text);
        }
    }
    for child in &children {
        if let Some(text) = first_identifier(child, snapshot, depth - 1) {
            return Some(text);
        }
    }
    None
}

fn is_name_kind(kind: &str) -> bool {
    kind.contains("identifier") || kind.ends_with("_name") || kind.ends_with("_key")
}

/// The text of a node, first line only and length capped.
fn node_text(node: &Node, snapshot: &BufferSnapshot) -> Option<String> {
    let start = node.start_byte().min(snapshot.len());
    let end = node.end_byte().min(snapshot.len());
    if start >= end {
        return None;
    }
    let text = snapshot.text_in(start..end);
    let line = text.lines().next().unwrap_or("").trim();
    if line.is_empty() {
        return None;
    }
    let mut name = String::new();
    for ch in line.chars() {
        if name.len() + ch.len_utf8() > MAX_NAME_LEN {
            break;
        }
        name.push(ch);
    }
    Some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shipped_language_has_a_table_entry() {
        // The lists may be empty, but looking one up must never panic.
        for language in [
            "bash",
            "c",
            "cpp",
            "css",
            "dockerfile",
            "go",
            "html",
            "java",
            "javascript",
            "json",
            "markdown",
            "python",
            "rust",
            "sql",
            "toml",
            "tsx",
            "typescript",
            "yaml",
        ] {
            let _ = definition_kinds(language);
        }
        assert!(definition_kinds("rust").contains(&"impl_item"));
        assert!(definition_kinds("python").contains(&"class_definition"));
        assert!(definition_kinds("dockerfile").is_empty());
        assert!(definition_kinds("lenguaje-inventado").is_empty());
    }

    #[test]
    fn name_kinds_cover_the_usual_grammars() {
        assert!(is_name_kind("identifier"));
        assert!(is_name_kind("type_identifier"));
        assert!(is_name_kind("field_identifier"));
        assert!(is_name_kind("tag_name"));
        assert!(is_name_kind("bare_key"));
        assert!(!is_name_kind("block"));
    }
}
