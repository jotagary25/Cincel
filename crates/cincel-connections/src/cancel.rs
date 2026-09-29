//! Cancellation of long downloads and installs
//! (`docs/specs/07-etapa5-productividad.md` §10.3, D14).
//!
//! `ureq` 3 has no "abort": the download loops check a [`CancelToken`]
//! before every 64 KiB read, between retries (the retry pause is split into
//! 50 ms slices) and before verifying or unpacking; unpacking checks it for
//! every archive entry and every read ([`CancelReader`], [`CancelWriter`]).
//! Returning drops the response, which closes the connection. `npm install`
//! runs in its own process group, killed on cancel.
//!
//! [`WorkLock`] serializes the work on one shared target (the Node runtime,
//! one agent's install directory) inside the process, so a second
//! preparation started while a cancelled one is still unwinding waits for it
//! instead of racing for the same `.part` file.

use std::collections::HashSet;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::time::{Duration, Instant};

use crate::error::{ConnectionsError, Result};

/// Slice of every cancellable pause.
const PAUSE_SLICE: Duration = Duration::from_millis(50);

/// Shared cancellation flag: clones observe the same state. Cancelling is
/// permanent; use a new token for a new operation.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// A token that is not cancelled.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask every operation holding a clone of this token to stop.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Whether [`CancelToken::cancel`] was called on any clone.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// `Err(Cancelled)` once cancelled.
    ///
    /// # Errors
    ///
    /// [`ConnectionsError::Cancelled`].
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(ConnectionsError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Sleep `duration` in 50 ms slices, returning early when cancelled.
    ///
    /// # Errors
    ///
    /// [`ConnectionsError::Cancelled`].
    pub fn sleep(&self, duration: Duration) -> Result<()> {
        let deadline = Instant::now() + duration;
        loop {
            self.check()?;
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(());
            }
            std::thread::sleep(left.min(PAUSE_SLICE));
        }
    }

    /// The I/O error the cancellable readers and writers fail with.
    pub(crate) fn io_error() -> std::io::Error {
        std::io::Error::other(CANCELLED_IO)
    }
}

const CANCELLED_IO: &str = "cincel: cancelled";

/// Map an error raised while `token` may have been cancelled: a cancelled
/// token wins over whatever the interrupted step reported.
pub(crate) fn or_cancelled(token: &CancelToken, error: ConnectionsError) -> ConnectionsError {
    if token.is_cancelled() {
        ConnectionsError::Cancelled
    } else {
        error
    }
}

/// A reader that fails with an I/O error once the token is cancelled
/// (checked before every read), so `io::copy` of one huge archive entry
/// stops promptly.
pub(crate) struct CancelReader<'a, R> {
    pub(crate) inner: R,
    pub(crate) token: &'a CancelToken,
}

impl<R: Read> Read for CancelReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.token.is_cancelled() {
            return Err(CancelToken::io_error());
        }
        self.inner.read(buf)
    }
}

/// A writer that fails once the token is cancelled (checked before every
/// write): the xz decoder writes as it goes.
pub(crate) struct CancelWriter<'a, W> {
    pub(crate) inner: W,
    pub(crate) token: &'a CancelToken,
}

impl<W: Write> Write for CancelWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.token.is_cancelled() {
            return Err(CancelToken::io_error());
        }
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

static WORK_LOCKS: LazyLock<(Mutex<HashSet<String>>, Condvar)> =
    LazyLock::new(|| (Mutex::new(HashSet::new()), Condvar::new()));

/// Held while one download/install of a shared target runs (released on
/// drop). Not reentrant.
#[derive(Debug)]
pub(crate) struct WorkLock(String);

impl WorkLock {
    /// Take the lock for `key`, waiting (cancellably) while another thread
    /// holds it.
    pub(crate) fn acquire(key: &str, token: &CancelToken) -> Result<Self> {
        let (lock, condvar) = &*WORK_LOCKS;
        let mut held = lock
            .lock()
            .map_err(|_| ConnectionsError::Network("candado envenenado".to_string()))?;
        while held.contains(key) {
            token.check()?;
            held = condvar
                .wait_timeout(held, PAUSE_SLICE)
                .map_err(|_| ConnectionsError::Network("candado envenenado".to_string()))?
                .0;
        }
        token.check()?;
        held.insert(key.to_string());
        Ok(Self(key.to_string()))
    }

    /// Take the lock for `key` only if nobody holds it.
    pub(crate) fn try_acquire(key: &str) -> Result<Self> {
        let (lock, _) = &*WORK_LOCKS;
        let mut held = lock
            .lock()
            .map_err(|_| ConnectionsError::Network("candado envenenado".to_string()))?;
        if held.contains(key) {
            return Err(ConnectionsError::Network(format!("{key} está ocupado")));
        }
        held.insert(key.to_string());
        Ok(Self(key.to_string()))
    }
}

impl Drop for WorkLock {
    fn drop(&mut self) {
        let (lock, condvar) = &*WORK_LOCKS;
        if let Ok(mut held) = lock.lock() {
            held.remove(&self.0);
        }
        condvar.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_share_the_flag() {
        let token = CancelToken::new();
        let clone = token.clone();
        assert!(!clone.is_cancelled());
        assert!(token.check().is_ok());
        token.cancel();
        assert!(clone.is_cancelled());
        assert!(matches!(clone.check(), Err(ConnectionsError::Cancelled)));
        assert_eq!(ConnectionsError::Cancelled.to_string(), "Cancelado");
    }

    #[test]
    fn sleep_returns_early_when_cancelled() {
        let token = CancelToken::new();
        let canceller = token.clone();
        let started = Instant::now();
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            canceller.cancel();
        });
        assert!(matches!(
            token.sleep(Duration::from_secs(10)),
            Err(ConnectionsError::Cancelled)
        ));
        assert!(started.elapsed() < Duration::from_secs(2));
        handle.join().expect("join");
        assert!(CancelToken::new().sleep(Duration::from_millis(10)).is_ok());
    }

    #[test]
    fn readers_and_writers_stop_once_cancelled() {
        let token = CancelToken::new();
        let mut reader = CancelReader {
            inner: std::io::Cursor::new(vec![1u8; 8]),
            token: &token,
        };
        let mut buf = [0u8; 4];
        assert_eq!(reader.read(&mut buf).expect("read"), 4);
        let mut writer = CancelWriter {
            inner: Vec::new(),
            token: &token,
        };
        writer.write_all(b"ok").expect("write");
        token.cancel();
        assert!(reader.read(&mut buf).is_err());
        assert!(writer.write_all(b"no").is_err());
        assert!(matches!(
            or_cancelled(&token, ConnectionsError::Network("x".to_string())),
            ConnectionsError::Cancelled
        ));
    }

    #[test]
    fn work_lock_waits_and_can_be_cancelled_while_waiting() {
        let key = "test-work-lock";
        let first = WorkLock::acquire(key, &CancelToken::new()).expect("first");
        let token = CancelToken::new();
        let canceller = token.clone();
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            canceller.cancel();
        });
        assert!(matches!(
            WorkLock::acquire(key, &token),
            Err(ConnectionsError::Cancelled)
        ));
        handle.join().expect("join");
        drop(first);
        let _again = WorkLock::acquire(key, &CancelToken::new()).expect("free again");
    }
}
