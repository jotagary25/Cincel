//! Cancelling a real, slow HTTP download really cuts the connection
//! (`docs/specs/08-etapa6-cierre-1-0.md` §5.6.3): a local, hand-rolled
//! HTTP/1.1 server (`std::net::TcpListener`, no framework — the only thing
//! it needs to do is trickle bytes and notice a broken pipe) serves 1 MB
//! every 100 ms, `Runtime::ensure` runs on its own thread with the *real*
//! [`HttpDownloader`], and the test cancels it 300 ms in.
//!
//! `#[ignore]`d on purpose (it sleeps for real, on purpose — `08-etapa6-
//! cierre-1-0.md` §5.6.3 is explicit that this one lives outside every
//! regular verification): run it with
//! `cargo +stable test -p cincel-connections --test slow_download -- --ignored`.
//! The CI runs it in its own weekly/manual job, never on every push.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use cincel_connections::{CancelToken, CincelPaths, HttpDownloader, NodeVersion, Runtime};

/// D15: the CI's shared runners are slower and more variable; the reference
/// machine (the variable unset) gets the real budget.
fn perf_budget_factor() -> f64 {
    std::env::var("CINCEL_PERF_BUDGET_FACTOR")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|factor| *factor > 0.)
        .unwrap_or(1.)
}

const VERSION: &str = "v1.0.0";
/// Any string works: [`NodeVersion::Exact`] never asks nodejs.org's
/// `index.json` for it, so the archive name only has to match what
/// [`serve`] and the SHASUMS response agree on.
const PLATFORM: &str = "linux-x64";

fn archive_name() -> String {
    format!("node-{VERSION}-{PLATFORM}.tar.xz")
}

/// A deliberately wrong SHA-256 (the download never finishes, so it is
/// never checked) — any 64 lowercase hex digits satisfy the file's shape.
fn fake_sha256() -> String {
    "0".repeat(64)
}

/// Reads one HTTP request line + headers (up to the blank line) off `stream`
/// and returns the request's path (`/v1.0.0/....tar.xz`, say). Panics on a
/// malformed request: there is only ever one client in this test.
fn read_request_path(stream: &mut TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        stream.read_exact(&mut byte).expect("leer la petición");
        buffer.push(byte[0]);
        if buffer.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let text = String::from_utf8_lossy(&buffer);
    let request_line = text.lines().next().expect("línea de pedido");
    request_line
        .split_whitespace()
        .nth(1)
        .expect("la ruta pedida")
        .to_string()
}

/// Runs the server for exactly one connection at a time, forever (the test
/// only ever makes two requests: `SHASUMS256.txt`, then the archive).
/// `disconnected` is signalled the moment a write into the slow archive body
/// fails (the client dropped the connection) — proof the cancellation
/// really reached the socket, not just the caller's return value.
fn serve(listener: TcpListener, disconnected: mpsc::Sender<Instant>) {
    let name = archive_name();
    let shasums = format!("{}  {name}\n", fake_sha256());
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { break };
        let path = read_request_path(&mut stream);
        if path.ends_with("SHASUMS256.txt") {
            let body = shasums.as_bytes();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(body);
            continue;
        }
        if path.ends_with(".tar.xz") {
            // A total the 100 ms/MB drip never reaches within the test:
            // long enough that cancelling at 300 ms always lands mid-body.
            let total = 50_000_000u64;
            let header =
                format!("HTTP/1.1 200 OK\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n");
            if stream.write_all(header.as_bytes()).is_err() {
                continue;
            }
            let chunk = vec![0u8; 1_000_000];
            let mut sent = 0u64;
            while sent < total {
                std::thread::sleep(Duration::from_millis(100));
                if stream.write_all(&chunk).is_err() {
                    let _ = disconnected.send(Instant::now());
                    break;
                }
                sent += chunk.len() as u64;
            }
            // The server has done its part; the test process exits shortly
            // after, closing the listener with it.
            return;
        }
    }
}

/// §5.6.3: cancelling 300 ms into a real slow download stops the thread in
/// under 500 ms (× `CINCEL_PERF_BUDGET_FACTOR`), leaves no `.part` file and
/// no staging directory, and the server itself sees the connection drop.
#[test]
#[ignore = "sleeps for real on purpose; run with --ignored (§5.6.3)"]
fn cancelling_cuts_a_real_slow_download() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("puerto").port();
    let (disconnect_tx, disconnect_rx) = mpsc::channel();
    let server = std::thread::Builder::new()
        .name("slow-download-server".to_string())
        .spawn(move || serve(listener, disconnect_tx))
        .expect("servidor");

    let dir = tempfile::tempdir().expect("tempdir");
    let paths = CincelPaths::under(dir.path());
    let runtime = Runtime::new(paths.clone())
        .with_base_url(format!("http://127.0.0.1:{port}/"))
        .with_version(NodeVersion::parse(VERSION.trim_start_matches('v')))
        .with_downloader(Box::new(HttpDownloader::default()))
        .with_retry(1, Duration::ZERO);

    let cancel = CancelToken::new();
    let download_cancel = cancel.clone();
    let download = std::thread::Builder::new()
        .name("slow-download-client".to_string())
        .spawn(move || runtime.ensure(&mut |_step| {}, &download_cancel))
        .expect("hilo de descarga");

    std::thread::sleep(Duration::from_millis(300));
    let cancelled_at = Instant::now();
    cancel.cancel();

    let result = download
        .join()
        .expect("el hilo de descarga no debe entrar en pánico");
    let stopped_within = cancelled_at.elapsed();
    let budget = Duration::from_millis((500. * perf_budget_factor()) as u64);
    assert!(
        stopped_within <= budget,
        "el hilo tardó {stopped_within:?} en terminar tras cancelar (presupuesto {budget:?})"
    );
    assert!(
        matches!(result, Err(cincel_connections::ConnectionsError::Cancelled)),
        "{result:?}"
    );

    assert!(
        disconnect_rx.recv_timeout(budget).is_ok(),
        "el servidor tiene que notar que el cliente cortó la conexión"
    );

    let downloads: Vec<_> = std::fs::read_dir(paths.downloads_dir())
        .into_iter()
        .flatten()
        .flatten()
        .collect();
    assert!(downloads.is_empty(), "sin .part: {downloads:?}");
    let runtime_dir: Vec<_> = std::fs::read_dir(paths.runtime_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with('.'))
        .collect();
    assert!(runtime_dir.is_empty(), "sin staging: {runtime_dir:?}");

    // The server thread exits on its own once the client disconnects (or the
    // process ends); nothing to join here without risking a hang if it is
    // still mid-`accept` for a connection that will never come.
    drop(server);
}
