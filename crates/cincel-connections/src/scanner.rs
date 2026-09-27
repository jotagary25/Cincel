//! Login output scanner: turns the raw bytes of a login pty into
//! [`crate::LoginEvent`]s (redacted output lines, the login URL, a one-time
//! code, the "paste the code" prompt).

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;

use crate::login::LoginEvent;

/// Any `https://` URL, up to whitespace, quotes or an escape.
static URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"https://[^\s"'<>`\x1b\x07]+"#).expect("regex"));

/// One-time device codes as printed by Codex/Claude (`ABCD-EFGHI`,
/// `ABCD-1234`): uppercase letters and digits, one dash.
static CODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[A-Z0-9]{4,5}-[A-Z0-9]{4,5}\b").expect("regex"));

/// ANSI escape sequences: CSI (`ESC [ ... final`), OSC (`ESC ] ... BEL` or
/// `ESC ] ... ESC \`), and two-byte escapes.
static ANSI: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[@-Z\\-_]")
        .expect("regex")
});

/// Hosts whose URLs are login links (the only ones reported as
/// [`LoginEvent::UrlDetected`]). `.test` is the reserved testing TLD.
const LOGIN_HOSTS: &[&str] = &[
    "claude.com",
    "claude.ai",
    "anthropic.com",
    "auth.openai.com",
    "accounts.google.com",
];

/// Prompts after which the program reads a pasted code from stdin.
const PASTE_PROMPTS: &[&str] = &[
    "paste code here",
    "paste the code",
    "paste the authorization code",
    "enter the authorization code",
    "authorization code:",
    "pegá el código",
];

/// Strip ANSI escape sequences and other control characters except tabs.
#[must_use]
pub fn strip_ansi(text: &str) -> String {
    ANSI.replace_all(text, "")
        .chars()
        .filter(|c| *c == '\t' || !c.is_control())
        .collect()
}

/// Whether `url` points at a login host.
#[must_use]
pub fn is_login_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest
        .split(['/', '?', '#', ':'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    host.ends_with(".test")
        || LOGIN_HOSTS
            .iter()
            .any(|allowed| host == *allowed || host.ends_with(&format!(".{allowed}")))
}

/// Login URLs in one clean line, with trailing punctuation removed.
#[must_use]
pub fn find_login_urls(line: &str) -> Vec<String> {
    URL.find_iter(line)
        .map(|found| {
            found
                .as_str()
                .trim_end_matches(['.', ',', ')', ']', ';'])
                .to_string()
        })
        .filter(|url| is_login_url(url))
        .collect()
}

/// A one-time code in one clean line: the whole line is the code, or the
/// line mentions a code.
#[must_use]
pub fn find_code(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let found = CODE.find(trimmed)?;
    let whole_line = found.start() == 0 && found.end() == trimmed.len();
    let lower = trimmed.to_lowercase();
    let mentions_code = lower.contains("code") || lower.contains("código");
    (whole_line || mentions_code).then(|| found.as_str().to_string())
}

/// Mask a line for [`LoginEvent::Output`]: `cincel_log::redact` (tokens,
/// `code=`/`state=` params, `XXXX-XXXXX` codes), then every URL's query and
/// fragment, then any code detected on the line. Never lets a login link or
/// code through (spec 06 §8).
#[must_use]
pub fn redact_line(line: &str, codes: &[String]) -> String {
    let mut text = URL
        .replace_all(line, |caps: &regex::Captures<'_>| {
            let url = &caps[0];
            match url.find(['?', '#']) {
                Some(cut) => format!("{}?[REDACTED]", &url[..cut]),
                None => url.to_string(),
            }
        })
        .into_owned();
    text = cincel_log::redact(&text);
    for code in codes {
        text = text.replace(code.as_str(), "[REDACTED]");
    }
    text
}

fn is_paste_prompt(text: &str) -> bool {
    let lower = text.to_lowercase();
    PASTE_PROMPTS.iter().any(|prompt| lower.contains(prompt))
}

/// Incremental scanner over the pty byte stream.
#[derive(Debug, Default)]
pub struct OutputScanner {
    bytes: Vec<u8>,
    partial: String,
    urls: HashSet<String>,
    codes: Vec<String>,
    /// Extra strings to mask (codes the user pasted: the pty echoes them).
    secrets: Vec<String>,
    asked_for_code: bool,
}

impl OutputScanner {
    /// New scanner.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Mask `secret` in every later [`LoginEvent::Output`] (the pty echoes
    /// what [`crate::LoginSession::send_code`] types).
    pub fn add_secret(&mut self, secret: &str) {
        let secret = secret.trim();
        if !secret.is_empty() && !self.secrets.iter().any(|known| known == secret) {
            self.secrets.push(secret.to_string());
        }
    }

    /// Feed raw bytes; returns the events they produce, in order.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<LoginEvent> {
        self.bytes.extend_from_slice(chunk);
        let valid = match std::str::from_utf8(&self.bytes) {
            Ok(text) => text.len(),
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            // Invalid bytes (not just a split sequence): decode lossily.
            Err(_) => self.bytes.len(),
        };
        let decoded = String::from_utf8_lossy(&self.bytes[..valid]).into_owned();
        self.bytes.drain(..valid);
        self.partial.push_str(&decoded);

        let mut events = Vec::new();
        while let Some(end) = self.partial.find(['\n', '\r']) {
            let line: String = self.partial.drain(..=end).collect();
            self.process_line(line.trim_end_matches(['\n', '\r']), &mut events);
        }
        // Prompts are printed without a trailing newline.
        if !self.asked_for_code && is_paste_prompt(&strip_ansi(&self.partial)) {
            self.asked_for_code = true;
            events.push(LoginEvent::NeedsPastedCode);
        }
        events
    }

    /// Flush whatever is left without a newline (end of stream).
    pub fn finish(&mut self) -> Vec<LoginEvent> {
        let mut events = Vec::new();
        let rest = std::mem::take(&mut self.partial);
        if !rest.is_empty() {
            self.process_line(&rest, &mut events);
        }
        events
    }

    fn process_line(&mut self, raw: &str, events: &mut Vec<LoginEvent>) {
        let line = strip_ansi(raw);
        let mut found_url = Vec::new();
        for url in find_login_urls(&line) {
            if self.urls.insert(url.clone()) {
                found_url.push(url);
            }
        }
        let code = find_code(&line).filter(|code| !self.codes.contains(code));
        if let Some(code) = &code {
            self.codes.push(code.clone());
        }
        if !line.trim().is_empty() {
            let mut secrets = self.codes.clone();
            secrets.extend(self.secrets.iter().cloned());
            events.push(LoginEvent::Output(redact_line(&line, &secrets)));
        }
        events.extend(found_url.into_iter().map(LoginEvent::UrlDetected));
        if let Some(code) = code {
            events.push(LoginEvent::CodeDetected(code));
        }
        if !self.asked_for_code && is_paste_prompt(&line) {
            self.asked_for_code = true;
            events.push(LoginEvent::NeedsPastedCode);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLAUDE_URL: &str = "https://claude.com/cai/oauth/authorize?code=true&client_id=9d1c&response_type=code&redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback&code_challenge=abc&state=zYkW";
    const OPENAI_URL: &str = "https://auth.openai.com/oauth/authorize?response_type=code&client_id=app_X&redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback&state=rrwM";
    const GOOGLE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth?redirect_uri=https%3A%2F%2Fcodeassist.google.com%2Fauthcode&access_type=offline&state=abc";

    #[test]
    fn detects_the_three_provider_urls() {
        for url in [CLAUDE_URL, OPENAI_URL, GOOGLE_URL] {
            let line = format!("If the browser didn't open, visit: {url}");
            assert_eq!(find_login_urls(&line), vec![url.to_string()], "{url}");
        }
        assert!(find_login_urls("docs: https://github.com/openai/codex").is_empty());
        assert_eq!(
            find_login_urls("   https://auth.openai.com/codex/device."),
            vec!["https://auth.openai.com/codex/device".to_string()]
        );
    }

    #[test]
    fn claude_transcript_yields_url_and_paste_prompt() {
        let mut scanner = OutputScanner::new();
        let mut events = scanner.feed(
            format!("Opening browser to sign in…\r\nIf the browser didn't open, visit: {CLAUDE_URL}\r\nPaste code here if prompted > ")
                .as_bytes(),
        );
        events.extend(scanner.finish());
        assert!(events.contains(&LoginEvent::UrlDetected(CLAUDE_URL.to_string())));
        assert!(events.contains(&LoginEvent::NeedsPastedCode));
        for event in &events {
            if let LoginEvent::Output(text) = event {
                assert!(!text.contains("state=zYkW"), "{text}");
                assert!(!text.contains("client_id"), "{text}");
            }
        }
    }

    #[test]
    fn claude_link_is_detected_with_a_neutral_browser_and_colors() {
        // With `BROWSER=/bin/true` the CLI still prints its fallback line;
        // it may color it or put the link on the next line.
        for transcript in [
            format!("\x1b[2mIf the browser didn't open, visit:\x1b[0m {CLAUDE_URL}\r\n"),
            format!("If the browser didn't open, visit:\r\n\x1b[94m{CLAUDE_URL}\x1b[0m\r\n"),
        ] {
            let mut scanner = OutputScanner::new();
            let mut events = scanner.feed(transcript.as_bytes());
            events.extend(scanner.finish());
            assert!(
                events.contains(&LoginEvent::UrlDetected(CLAUDE_URL.to_string())),
                "{transcript:?}: {events:?}"
            );
        }
    }

    #[test]
    fn codex_colored_device_flow_yields_url_and_code() {
        let mut scanner = OutputScanner::new();
        let transcript = "Follow these steps to sign in with ChatGPT using device code authorization:\r\n\r\n1. Open this link in your browser and sign in to your account\r\n   \x1b[94mhttps://auth.openai.com/codex/device\x1b[0m\r\n\r\n2. Enter this one-time code \x1b[90m(expires in 15 minutes)\x1b[0m\r\n   \x1b[94mABCD-EFGH2\x1b[0m\r\n";
        let events = scanner.feed(transcript.as_bytes());
        assert!(events.contains(&LoginEvent::UrlDetected(
            "https://auth.openai.com/codex/device".to_string()
        )));
        assert!(events.contains(&LoginEvent::CodeDetected("ABCD-EFGH2".to_string())));
        assert!(!events.contains(&LoginEvent::NeedsPastedCode));
        let outputs: Vec<&String> = events
            .iter()
            .filter_map(|event| match event {
                LoginEvent::Output(text) => Some(text),
                _ => None,
            })
            .collect();
        assert!(outputs.iter().all(|text| !text.contains("ABCD-EFGH2")));
        assert!(outputs.iter().all(|text| !text.contains('\x1b')));
    }

    #[test]
    fn chunks_split_mid_line_and_mid_utf8_are_reassembled() {
        let mut scanner = OutputScanner::new();
        let text = format!("Iniciá sesión acá: {OPENAI_URL}\n");
        let bytes = text.as_bytes();
        let mut events = Vec::new();
        // Split inside "á" (2 bytes) and inside the URL.
        assert_eq!(&bytes[..5], b"Inici");
        for chunk in [&bytes[..6], &bytes[6..30], &bytes[30..]] {
            events.extend(scanner.feed(chunk));
        }
        assert!(events.contains(&LoginEvent::UrlDetected(OPENAI_URL.to_string())));
        assert!(events.iter().any(
            |event| matches!(event, LoginEvent::Output(text) if text.starts_with("Iniciá sesión acá"))
        ));
    }

    #[test]
    fn antigravity_stderr_line_yields_the_link_and_redacts_its_query() {
        let url = "https://accounts.google.com/o/oauth2/v2/auth?response_type=code&client_id=abc.apps.googleusercontent.com&redirect_uri=http%3A%2F%2F127.0.0.1%3A54243%2F&scope=openid&state=S3CR3T&code_challenge=CH4LL&code_challenge_method=S256&access_type=offline&prompt=consent";
        let line = format!("Open the following link to authenticate the ACP server: {url}");
        assert_eq!(find_login_urls(&line), vec![url.to_string()]);
        let redacted = redact_line(&line, &[]);
        assert_eq!(
            redacted,
            "Open the following link to authenticate the ACP server: \
             https://accounts.google.com/o/oauth2/v2/auth?[REDACTED]"
        );
        for secret in ["S3CR3T", "CH4LL", "127.0.0.1"] {
            assert!(!redacted.contains(secret), "{redacted}");
        }
    }

    #[test]
    fn google_style_prompt_is_detected() {
        let mut scanner = OutputScanner::new();
        let events = scanner.feed(
            format!("Please visit the following URL to authorize the application:\n\n{GOOGLE_URL}\n\nEnter the authorization code: ").as_bytes(),
        );
        assert!(events.contains(&LoginEvent::UrlDetected(GOOGLE_URL.to_string())));
        assert!(events.contains(&LoginEvent::NeedsPastedCode));
        // Only once.
        assert!(
            !scanner
                .feed(b"Enter the authorization code: ")
                .contains(&LoginEvent::NeedsPastedCode)
        );
    }

    #[test]
    fn redact_line_masks_query_codes_and_tokens() {
        let line = format!("visit {CLAUDE_URL} Bearer sk-123 then use WXYZ-12345");
        let masked = redact_line(&line, &["WXYZ-12345".to_string()]);
        assert!(masked.contains("https://claude.com/cai/oauth/authorize?[REDACTED]"));
        assert!(!masked.contains("sk-123"));
        assert!(!masked.contains("WXYZ-12345"));
        assert_eq!(redact_line("hola", &[]), "hola");
    }

    #[test]
    fn code_detection_needs_context() {
        assert_eq!(find_code("   ABCD-EFGHI  "), Some("ABCD-EFGHI".to_string()));
        assert_eq!(
            find_code("Enter this code: WXYZ-9876"),
            Some("WXYZ-9876".to_string())
        );
        assert_eq!(find_code("Build UTF-8 done at ABCD-EFGH"), None);
        assert_eq!(find_code("versión 1.2.3"), None);
    }

    #[test]
    fn strip_ansi_removes_escapes() {
        assert_eq!(
            strip_ansi("\x1b[1;32mlisto\x1b[0m\x1b]0;título\x07"),
            "listo"
        );
    }
}
