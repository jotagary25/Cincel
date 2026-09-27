//! Redaction for anything captured from a login flow before it can reach
//! `cincel.log` (`docs/specs/06-etapa4-conexiones-y-cincel.md` §8: "nunca se
//! escriben tokens, enlaces de login ni códigos").
//!
//! [`redact`] is the helper the pty output of the login capture
//! (`docs/specs/06-etapa4-conexiones-y-cincel.md` §4, F2 step 3) runs every
//! line through before it is logged or shown: `cincel-connections`' output
//! scanner (`redact_line`) applies it to every `LoginEvent::Output` line, and
//! the "Ver detalles técnicos" section of the connect modal only ever shows
//! those already-redacted lines.

use std::sync::LazyLock;

use regex::Regex;

/// `code=`, `state=` and `token=` query parameters (case-insensitive, as
/// providers spell them differently): the value up to the next `&` or
/// whitespace is masked.
static QUERY_PARAM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(code|state|token)=[^\s&]+").unwrap());

/// `Authorization: Bearer <token>` headers and any other `Bearer …` mention.
static BEARER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)Bearer\s+\S+").unwrap());

/// One-time codes shaped like `XXXX-XXXXX` (four alphanumeric characters, a
/// dash, five more), the form Claude, Codex and Gemini's device-code logins
/// print for the user to paste back.
static ONE_TIME_CODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[A-Za-z0-9]{4}-[A-Za-z0-9]{5}\b").unwrap());

/// Masks anything in `text` that looks like a login secret: `code=`/`state=`/
/// `token=` query parameters, `Bearer …` headers and `XXXX-XXXXX` one-time
/// codes. Idempotent and safe to run on text that has neither: it is
/// returned unchanged.
#[must_use]
pub fn redact(text: &str) -> String {
    let text = QUERY_PARAM.replace_all(text, |caps: &regex::Captures<'_>| {
        format!("{}=[REDACTED]", &caps[1])
    });
    let text = BEARER.replace_all(&text, "Bearer [REDACTED]");
    ONE_TIME_CODE.replace_all(&text, "[REDACTED]").into_owned()
}

#[cfg(test)]
mod tests {
    use super::redact;

    #[test]
    fn masks_query_parameters_case_insensitively() {
        assert_eq!(
            redact("https://x.test/callback?CODE=abc123&state=xyz-789&other=fine"),
            "https://x.test/callback?CODE=[REDACTED]&state=[REDACTED]&other=fine"
        );
    }

    #[test]
    fn masks_a_token_param() {
        assert_eq!(
            redact("...&token=sk-verysecret123 trailing text"),
            "...&token=[REDACTED] trailing text"
        );
    }

    #[test]
    fn masks_bearer_headers() {
        assert_eq!(
            redact("Authorization: Bearer sk-abcdef123456"),
            "Authorization: Bearer [REDACTED]"
        );
    }

    #[test]
    fn masks_one_time_codes() {
        assert_eq!(
            redact("Pegá este código: ABCD-EFGHI y confirmá"),
            "Pegá este código: [REDACTED] y confirmá"
        );
    }

    #[test]
    fn leaves_ordinary_text_untouched() {
        let text = "Abrí este enlace y aprobá el acceso en el navegador.";
        assert_eq!(redact(text), text);
    }

    #[test]
    fn does_not_confuse_a_short_word_with_a_one_time_code() {
        // Four letters, a dash, four letters: one short of the real shape.
        let text = "El id es ABCD-EFGH, no un código de un solo uso.";
        assert_eq!(redact(text), text);
    }

    #[test]
    fn redacts_every_secret_on_the_same_line() {
        let line = "GET /cb?code=one&state=two Bearer three ABCD-EFGHI";
        assert_eq!(
            redact(line),
            "GET /cb?code=[REDACTED]&state=[REDACTED] Bearer [REDACTED] [REDACTED]"
        );
    }
}
