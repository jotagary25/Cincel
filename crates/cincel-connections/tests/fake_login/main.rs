//! Fake provider login command for the pty engine tests. No network, no
//! real provider: it only mimics what `claude auth login`, `codex login`
//! and `gemini` print.
//!
//! Flags (any order):
//! * `--url URL`: print `If the browser didn't open, visit: URL`;
//! * `--code CODE`: print a device code the way Codex does;
//! * `--secret TOKEN`: print lines carrying a bearer token and `code=`/
//!   `state=` query parameters (redaction tests);
//! * `--print-env NAME`: print `env NAME=<value|unset>`;
//! * `--expect CODE`: print `Paste code here if prompted > ` (no newline)
//!   and read one line from stdin; exit 0 if it matches, 1 otherwise;
//! * `--creds-var VAR` / `--creds-file NAME`: on success write
//!   `$VAR/NAME` (default `.credentials.json`), like the real CLIs do in
//!   their profile directory;
//! * `--write-after-ms N PATH`: write `PATH` after N ms (Gemini-style
//!   success detection) while hanging;
//! * `--spawn-child PIDFILE`: spawn `sleep 600` and write its pid (process
//!   group kill tests);
//! * `--hang`: never exit on its own;
//! * `--exit N`: exit with N right after printing.

use std::io::{BufRead, Write};
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut expect = None;
    let mut exit_code = 0;
    let mut hang = false;
    let mut creds_var = None;
    let mut creds_file = ".credentials.json".to_string();
    let mut write_after: Option<(u64, String)> = None;
    let mut out = std::io::stdout();

    let mut index = 0;
    let value = |index: usize| args.get(index + 1).cloned().unwrap_or_default();
    while index < args.len() {
        match args[index].as_str() {
            "--url" => {
                let _ = writeln!(out, "Opening browser to sign in…");
                let _ = writeln!(out, "If the browser didn't open, visit: {}", value(index));
                index += 1;
            }
            "--code" => {
                let _ = writeln!(out, "2. Enter this one-time code (expires in 15 minutes)");
                let _ = writeln!(out, "   \x1b[94m{}\x1b[0m", value(index));
                index += 1;
            }
            "--secret" => {
                let secret = value(index);
                let _ = writeln!(out, "Authorization: Bearer {secret}");
                let _ = writeln!(
                    out,
                    "callback https://example.invalid/cb?code={secret}&state={secret}"
                );
                index += 1;
            }
            "--print-env" => {
                let name = value(index);
                let shown = std::env::var(&name).unwrap_or_else(|_| "<unset>".to_string());
                let _ = writeln!(out, "env {name}={shown}");
                index += 1;
            }
            "--expect" => {
                expect = Some(value(index));
                index += 1;
            }
            "--creds-var" => {
                creds_var = Some(value(index));
                index += 1;
            }
            "--creds-file" => {
                creds_file = value(index);
                index += 1;
            }
            "--write-after-ms" => {
                let ms = value(index).parse().unwrap_or(0);
                let path = args.get(index + 2).cloned().unwrap_or_default();
                write_after = Some((ms, path));
                index += 2;
            }
            "--spawn-child" => {
                let pid_file = value(index);
                if let Ok(child) = std::process::Command::new("sleep").arg("600").spawn() {
                    let _ = std::fs::write(pid_file, child.id().to_string());
                }
                index += 1;
            }
            "--hang" => hang = true,
            "--exit" => {
                exit_code = value(index).parse().unwrap_or(1);
                index += 1;
            }
            _ => {}
        }
        index += 1;
    }
    let _ = out.flush();

    let write_creds = || {
        if let Some(var) = &creds_var
            && let Ok(dir) = std::env::var(var)
        {
            let path = std::path::Path::new(&dir).join(&creds_file);
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(path, r#"{"fake":"credentials"}"#);
        }
    };

    if let Some(expected) = expect {
        let _ = write!(out, "Paste code here if prompted > ");
        let _ = out.flush();
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
        if line.trim() == expected {
            write_creds();
            let _ = writeln!(out, "Login successful.");
            std::process::exit(0);
        }
        let _ = writeln!(out, "Invalid code.");
        std::process::exit(1);
    }

    if hang {
        if let Some((ms, path)) = write_after {
            std::thread::sleep(Duration::from_millis(ms));
            let _ = std::fs::write(path, r#"{"fake":"credentials"}"#);
        }
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    if exit_code == 0 {
        write_creds();
        let _ = writeln!(out, "Successfully logged in");
    }
    std::process::exit(exit_code);
}
