//! Keymap context expressions.
//!
//! A keymap section is only active when its `context` matches the contexts
//! the focused part of the UI declares (`Editor`, `searching`,
//! `review_hunk_under_cursor`, …). The expression language is the small
//! subset of Zed's that `docs/specs/modulos/workspace.md` uses:
//!
//! ```text
//! expr    := or
//! or      := and ( "||" and )*
//! and     := unary ( "&&" unary )*
//! unary   := "!" unary | atom
//! atom    := identifier | "(" expr ")"
//! ```
//!
//! An identifier starts with a letter or `_` and continues with letters,
//! digits, `_`, `-` or `.`. An absent or empty context is [`ContextExpr::Always`].
//!
//! `||` is not used by the default keymap but is accepted, both because Zed
//! accepts it (users copy keymaps over) and because `!(a && b)` would
//! otherwise be the only way to write it.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

/// A parsed keymap context expression.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ContextExpr {
    /// No context: the section always applies.
    #[default]
    Always,
    /// A single context name.
    Identifier(String),
    /// `!expr`.
    Not(Box<ContextExpr>),
    /// `a && b`.
    And(Box<ContextExpr>, Box<ContextExpr>),
    /// `a || b`.
    Or(Box<ContextExpr>, Box<ContextExpr>),
}

impl ContextExpr {
    /// Parses an expression. An empty (or whitespace-only) string is
    /// [`ContextExpr::Always`].
    ///
    /// The error is a Spanish message ready to become a [`crate::SettingsIssue`].
    pub fn parse(source: &str) -> Result<ContextExpr, String> {
        let tokens = tokenize(source)?;
        if tokens.is_empty() {
            return Ok(ContextExpr::Always);
        }
        let mut parser = Parser { tokens, index: 0 };
        let expr = parser.expression()?;
        if parser.index < parser.tokens.len() {
            return Err(format!(
                "sobra «{}» al final de la expresión de contexto",
                parser.tokens[parser.index]
            ));
        }
        Ok(expr)
    }

    /// Whether the expression holds for the given active contexts.
    pub fn matches(&self, contexts: &[&str]) -> bool {
        match self {
            ContextExpr::Always => true,
            ContextExpr::Identifier(name) => contexts.contains(&name.as_str()),
            ContextExpr::Not(inner) => !inner.matches(contexts),
            ContextExpr::And(left, right) => left.matches(contexts) && right.matches(contexts),
            ContextExpr::Or(left, right) => left.matches(contexts) || right.matches(contexts),
        }
    }

    /// Every identifier the expression mentions, for diagnostics.
    pub fn identifiers(&self) -> Vec<&str> {
        let mut out = Vec::new();
        self.collect_identifiers(&mut out);
        out
    }

    fn collect_identifiers<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            ContextExpr::Always => {}
            ContextExpr::Identifier(name) => out.push(name),
            ContextExpr::Not(inner) => inner.collect_identifiers(out),
            ContextExpr::And(left, right) | ContextExpr::Or(left, right) => {
                left.collect_identifiers(out);
                right.collect_identifiers(out);
            }
        }
    }
}

impl fmt::Display for ContextExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContextExpr::Always => Ok(()),
            ContextExpr::Identifier(name) => f.write_str(name),
            ContextExpr::Not(inner) => match **inner {
                ContextExpr::Identifier(_) | ContextExpr::Not(_) => write!(f, "!{inner}"),
                _ => write!(f, "!({inner})"),
            },
            ContextExpr::And(left, right) => {
                write_operand(f, left, true)?;
                f.write_str(" && ")?;
                write_operand(f, right, true)
            }
            ContextExpr::Or(left, right) => {
                write_operand(f, left, false)?;
                f.write_str(" || ")?;
                write_operand(f, right, false)
            }
        }
    }
}

/// Writes one operand, parenthesizing it when it binds looser than its parent.
fn write_operand(f: &mut fmt::Formatter<'_>, expr: &ContextExpr, inside_and: bool) -> fmt::Result {
    let needs_parens = inside_and && matches!(expr, ContextExpr::Or(..));
    if needs_parens {
        write!(f, "({expr})")
    } else {
        write!(f, "{expr}")
    }
}

impl Serialize for ContextExpr {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ContextExpr {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        ContextExpr::parse(&text).map_err(D::Error::custom)
    }
}

/// A lexical token of the expression language.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Identifier(String),
    And,
    Or,
    Not,
    Open,
    Close,
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::Identifier(name) => f.write_str(name),
            Token::And => f.write_str("&&"),
            Token::Or => f.write_str("||"),
            Token::Not => f.write_str("!"),
            Token::Open => f.write_str("("),
            Token::Close => f.write_str(")"),
        }
    }
}

fn tokenize(source: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        match byte {
            b' ' | b'\t' | b'\n' | b'\r' => index += 1,
            b'(' => {
                tokens.push(Token::Open);
                index += 1;
            }
            b')' => {
                tokens.push(Token::Close);
                index += 1;
            }
            b'!' => {
                tokens.push(Token::Not);
                index += 1;
            }
            b'&' | b'|' => {
                if index + 1 < bytes.len() && bytes[index + 1] == byte {
                    tokens.push(if byte == b'&' { Token::And } else { Token::Or });
                    index += 2;
                } else {
                    return Err(format!(
                        "«{}» suelto en la expresión de contexto (¿querías «{0}{0}»?)",
                        byte as char
                    ));
                }
            }
            _ if is_identifier_start(byte) => {
                let start = index;
                index += 1;
                while index < bytes.len() && is_identifier_continue(bytes[index]) {
                    index += 1;
                }
                tokens.push(Token::Identifier(source[start..index].to_owned()));
            }
            _ => {
                return Err(format!(
                    "carácter inesperado «{}» en la expresión de contexto",
                    source[index..].chars().next().unwrap_or('?')
                ));
            }
        }
    }
    Ok(tokens)
}

fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_identifier_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')
}

struct Parser {
    tokens: Vec<Token>,
    index: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.index)
    }

    fn expression(&mut self) -> Result<ContextExpr, String> {
        let mut left = self.conjunction()?;
        while self.peek() == Some(&Token::Or) {
            self.index += 1;
            let right = self.conjunction()?;
            left = ContextExpr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn conjunction(&mut self) -> Result<ContextExpr, String> {
        let mut left = self.unary()?;
        while self.peek() == Some(&Token::And) {
            self.index += 1;
            let right = self.unary()?;
            left = ContextExpr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<ContextExpr, String> {
        if self.peek() == Some(&Token::Not) {
            self.index += 1;
            return Ok(ContextExpr::Not(Box::new(self.unary()?)));
        }
        self.atom()
    }

    fn atom(&mut self) -> Result<ContextExpr, String> {
        match self.peek().cloned() {
            Some(Token::Identifier(name)) => {
                self.index += 1;
                Ok(ContextExpr::Identifier(name))
            }
            Some(Token::Open) => {
                self.index += 1;
                let inner = self.expression()?;
                if self.peek() != Some(&Token::Close) {
                    return Err("falta un «)» en la expresión de contexto".to_owned());
                }
                self.index += 1;
                Ok(inner)
            }
            Some(token) => Err(format!("se esperaba un contexto y se encontró «{token}»")),
            None => Err("la expresión de contexto termina antes de tiempo".to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> ContextExpr {
        ContextExpr::parse(source).unwrap_or_else(|error| panic!("{source:?}: {error}"))
    }

    #[test]
    fn empty_context_always_matches() {
        assert_eq!(parse(""), ContextExpr::Always);
        assert_eq!(parse("   "), ContextExpr::Always);
        assert!(parse("").matches(&[]));
    }

    #[test]
    fn identifiers() {
        let expr = parse("Editor");
        assert_eq!(expr, ContextExpr::Identifier("Editor".to_owned()));
        assert!(expr.matches(&["Editor", "Chat"]));
        assert!(!expr.matches(&["Chat"]));
        // Dots, dashes and digits are part of a name.
        assert_eq!(
            parse("review.hunk-2"),
            ContextExpr::Identifier("review.hunk-2".to_owned())
        );
    }

    #[test]
    fn conjunction_and_negation() {
        let expr = parse("Editor && review_hunk_under_cursor");
        assert!(expr.matches(&["Editor", "review_hunk_under_cursor"]));
        assert!(!expr.matches(&["Editor"]));

        let expr = parse("Editor && !searching");
        assert!(expr.matches(&["Editor"]));
        assert!(!expr.matches(&["Editor", "searching"]));
        assert!(!expr.matches(&["searching"]));

        assert!(parse("!!Editor").matches(&["Editor"]));
    }

    #[test]
    fn parentheses_change_precedence() {
        // `!` binds tighter than `&&`.
        assert!(parse("!a && b").matches(&["b"]));
        assert!(!parse("!(a && b)").matches(&["a", "b"]));
        assert!(parse("!(a && b)").matches(&["a"]));
        // `&&` binds tighter than `||`.
        assert!(parse("a && b || c").matches(&["c"]));
        assert!(parse("a && b || c").matches(&["a", "b"]));
        assert!(!parse("a && (b || c)").matches(&["a"]));
        assert!(parse("a && (b || c)").matches(&["a", "c"]));
    }

    #[test]
    fn errors_are_specific() {
        for (source, needle) in [
            ("Editor &&", "termina antes de tiempo"),
            ("Editor & searching", "suelto"),
            ("(Editor", "falta un"),
            ("Editor searching", "sobra"),
            ("&& Editor", "se esperaba un contexto"),
            ("Editor && $", "carácter inesperado"),
            ("!", "termina antes de tiempo"),
        ] {
            let error = ContextExpr::parse(source).unwrap_err();
            assert!(error.contains(needle), "{source:?} -> {error}");
        }
    }

    #[test]
    fn display_round_trips() {
        for source in [
            "Editor",
            "Editor && review_hunk_under_cursor",
            "Editor && !searching",
            "a && b || c",
            "a && (b || c)",
            "!(a && b)",
            "!a",
        ] {
            let expr = parse(source);
            let printed = expr.to_string();
            assert_eq!(parse(&printed), expr, "{source} -> {printed}");
        }
        assert_eq!(
            parse("Editor&&searching").to_string(),
            "Editor && searching"
        );
    }

    #[test]
    fn serde_round_trip() {
        let expr = parse("Editor && !searching");
        let json = serde_json::to_string(&expr).unwrap();
        assert_eq!(json, "\"Editor && !searching\"");
        assert_eq!(serde_json::from_str::<ContextExpr>(&json).unwrap(), expr);
    }

    #[test]
    fn identifiers_are_listed() {
        assert_eq!(
            parse("Editor && !(searching || review_hunk_under_cursor)").identifiers(),
            ["Editor", "searching", "review_hunk_under_cursor"]
        );
    }
}
