//! The measuring subcommands that talk to the compositor: `launch`, `idle`,
//! `quick-open`, `type`, `scroll`, `key`, `stop` and `probe`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rustix::process::{Pid, Signal, kill_process};
use serde_json::{Value, json};

use crate::clock::{ms_between, now_ns};
use crate::frames::{first_damage_after, intervals_ms, stable_frame};
use crate::keys::{parse_combo, text_strokes};
use crate::procfs;
use crate::profile::{self, AppKind};
use crate::stats::{round2, summarize};
use crate::wl::{Session, ToplevelProtocol};

const MS: u64 = 1_000_000;
const PROC: &str = "/proc";

/// Detection protocol of `launch` (`none` = only `/proc`, no Wayland).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detection {
    Protocol(ToplevelProtocol),
    None,
}

#[derive(Debug, Clone)]
pub struct LaunchOptions {
    pub name: String,
    pub tag: Option<String>,
    pub app_id: Option<String>,
    pub profile: Option<PathBuf>,
    pub detection: Detection,
    pub settle_ms: u64,
    pub idle_ms: u64,
    pub timeout_ms: u64,
    pub open_file: Option<String>,
    /// Key combinations injected once the window has content (per-app
    /// normalization, such as closing an AI side panel).
    pub setup_keys: Vec<String>,
    pub keep_open: bool,
    pub app_log: Option<PathBuf>,
    pub command: Vec<String>,
}

fn emit(value: &Value) {
    println!("{value}");
}

/// SIGTERM to every process of the tree of `pid`, then SIGKILL to what is
/// left after `grace`. Returns how many processes were signalled.
pub fn stop_tree(pid: u32, grace: Duration) -> usize {
    let proc_root = Path::new(PROC);
    let pids = procfs::tree(proc_root, pid);
    for &p in &pids {
        if let Some(p) = Pid::from_raw(p as i32) {
            let _ = kill_process(p, Signal::TERM);
        }
    }
    let deadline = now_ns() + grace.as_nanos() as u64;
    while now_ns() < deadline {
        let alive = pids
            .iter()
            .filter(|p| proc_root.join(p.to_string()).join("stat").exists() && !is_zombie(**p))
            .count();
        if alive == 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    for &p in &pids {
        if proc_root.join(p.to_string()).exists()
            && let Some(p) = Pid::from_raw(p as i32)
        {
            let _ = kill_process(p, Signal::KILL);
        }
    }
    pids.len()
}

fn is_zombie(pid: u32) -> bool {
    fs::read_to_string(Path::new(PROC).join(pid.to_string()).join("stat"))
        .ok()
        .and_then(|line| {
            let close = line.rfind(')')?;
            line[close + 1..]
                .split_whitespace()
                .next()
                .map(|state| state == "Z")
        })
        .unwrap_or(true)
}

fn spawn(options: &LaunchOptions, prepared: Option<&profile::Profile>, t0: u64) -> Result<Child> {
    let (program, rest) = options
        .command
        .split_first()
        .context("falta el comando de la app después de --")?;
    let mut command = Command::new(program);
    if let Some(prepared) = prepared {
        command.args(&prepared.args);
        for (key, value) in &prepared.env {
            command.env(key, value);
        }
    }
    command.args(rest);
    command.env("CINCEL_BENCH_T0", t0.to_string());
    command.stdin(Stdio::null());
    match &options.app_log {
        Some(path) => {
            let log =
                fs::File::create(path).with_context(|| format!("creando {}", path.display()))?;
            command.stdout(log.try_clone()?);
            command.stderr(log);
        }
        None => {
            command.stdout(Stdio::null());
            command.stderr(Stdio::null());
        }
    }
    command
        .spawn()
        .with_context(|| format!("no se pudo lanzar {program}"))
}

/// Memory (M7) and idle CPU/frames (M8) of the tree of `root`
/// (`include_root`: whether `root` itself belongs to the app).
fn idle_measure(
    session: Option<&mut Session>,
    root: u32,
    include_root: bool,
    main_pid: u32,
    idle_ms: u64,
) -> Result<Value> {
    let proc_root = Path::new(PROC);
    let pids = procfs::descendants(&procfs::all_stats(proc_root), root, include_root);
    let memory = procfs::memory(proc_root, main_pid, &pids);
    let mut result = json!({
        "rss_main_kb": memory.rss_main_kb,
        "rss_tree_kb": memory.rss_tree_kb,
        "pss_tree_kb": memory.pss_tree_kb,
        "processes": memory.processes,
    });
    if idle_ms == 0 {
        return Ok(result);
    }
    let before = procfs::cpu_ticks(proc_root, &pids);
    let start = now_ns();
    let frames_before = session.as_ref().map(|s| s.state.frames.len());
    let deadline = start + idle_ms * MS;
    let mut session = session;
    match session.as_deref_mut() {
        Some(session) => session.dispatch_until(deadline)?,
        None => std::thread::sleep(Duration::from_millis(idle_ms)),
    }
    let end = now_ns();
    // Processes born during the window count too.
    let pids_after = procfs::descendants(&procfs::all_stats(proc_root), root, include_root);
    let after = procfs::cpu_ticks(proc_root, &pids_after);
    let ticks = procfs::ticks_between(&before, &after);
    let seconds = (end - start) as f64 / 1e9;
    let cpu_pct = ticks as f64 / rustix::param::clock_ticks_per_second() as f64 / seconds * 100.0;
    result["idle_s"] = json!(round2(seconds));
    result["cpu_pct"] = json!(round2(cpu_pct));
    result["idle_frames"] = match (session, frames_before) {
        (Some(session), Some(before)) => json!(session.state.frames.len() - before),
        _ => Value::Null,
    };
    Ok(result)
}

/// `launch`: start → mapped → first content → settle → memory → idle.
pub fn launch(options: &LaunchOptions) -> Result<()> {
    let kind = AppKind::parse(&options.name).ok();
    if options.profile.is_some() && kind.is_none() {
        bail!("--profile necesita --name cincel, zed, antigravity o vscode");
    }
    let expected_app_id = options
        .app_id
        .clone()
        .or_else(|| kind.and_then(AppKind::default_app_id).map(str::to_owned));

    let mut session = match options.detection {
        Detection::Protocol(protocol) => {
            let mut session = Session::connect()?;
            session.watch_toplevels(protocol)?;
            if protocol == ToplevelProtocol::Wlr {
                session.start_capture()?;
                session.snapshot_desktop()?;
            }
            session.arm_toplevels();
            Some(session)
        }
        Detection::None => None,
    };
    // Orphans of the app (double forks) are re-parented to us, so the tree
    // measured below is complete.
    rustix::process::set_child_subreaper(Some(rustix::process::getpid()))?;

    // The profile is written before t0: it is not part of the start.
    let prepared = match (kind, &options.profile) {
        (Some(kind), Some(dir)) => Some(profile::prepare(kind, dir)?),
        _ => None,
    };
    let t0 = now_ns();
    let mut child = spawn(options, prepared.as_ref(), t0)?;
    let pid = child.id();
    let deadline = t0 + options.timeout_ms * MS;

    let mut mapped: Option<u64> = None;
    let mut app_id: Option<String> = None;
    let mut first_frame = None;
    let mut first_content = None;
    let mut fraction = 0.0;
    let mut error: Option<String> = None;

    if let Some(session) = session.as_mut() {
        let wanted = expected_app_id.clone();
        // Wait for the window.
        loop {
            let hit = session
                .toplevels()
                .filter(|t| t.new && t.done_ns.is_some())
                .filter(|t| wanted.as_deref().is_none_or(|id| t.app_id == id))
                .min_by_key(|t| t.done_ns)
                .map(|t| (t.done_ns.unwrap_or(0), t.app_id.clone()));
            if let Some((at, id)) = hit {
                mapped = Some(at);
                app_id = Some(id);
                break;
            }
            if let Ok(Some(status)) = child.try_wait() {
                error = Some(format!(
                    "la app terminó antes de mostrar la ventana ({status})"
                ));
                break;
            }
            if now_ns() >= deadline {
                let seen: Vec<String> = session.toplevels().map(|t| t.app_id.clone()).collect();
                error = Some(format!(
                    "no apareció la ventana esperada ({}) en {} ms; ventanas vistas: {seen:?}",
                    wanted.as_deref().unwrap_or("cualquiera"),
                    options.timeout_ms
                ));
                break;
            }
            session.dispatch_for(Duration::from_millis(20))?;
        }
        if mapped.is_some() && options.detection == Detection::Protocol(ToplevelProtocol::Wlr) {
            session.start_content_probe();
            let content_deadline = now_ns() + options.timeout_ms * MS;
            loop {
                let (frame, content, last) = session.content_result();
                first_frame = frame;
                fraction = last;
                if content.is_some() {
                    first_content = content;
                    break;
                }
                if now_ns() >= content_deadline {
                    error = Some("la ventana no mostró contenido a tiempo".into());
                    break;
                }
                if let Ok(Some(status)) = child.try_wait() {
                    error = Some(format!("la app terminó antes de pintar ({status})"));
                    break;
                }
                session.dispatch_for(Duration::from_millis(20))?;
            }
        }
    } else {
        std::thread::sleep(Duration::from_millis(100));
    }

    let mut line = json!({
        "kind": "launch",
        "app": options.name,
        "tag": options.tag,
        "pid": pid,
        "protocol": match options.detection {
            Detection::Protocol(ToplevelProtocol::Wlr) => "wlr",
            Detection::Protocol(ToplevelProtocol::Ext) => "ext",
            Detection::None => "none",
        },
        "app_id": app_id,
        "mapped_ms": mapped.map(|at| ms_between(t0, at)),
        "first_frame_ms": first_frame.map(|at| ms_between(t0, at)),
        "first_content_ms": first_content.map(|at| ms_between(t0, at)),
        "content_fraction": (fraction * 10_000.0).round() / 10_000.0,
        "content_colors": session.as_ref().map(Session::content_colors),
        "background": session.as_ref().and_then(Session::content_background),
        "desktop": session.as_ref().and_then(Session::desktop_color),
        "clock": session.as_ref().map(|s| s.clock().name()),
    });

    if error.is_none() {
        if let (false, Some(session)) = (options.setup_keys.is_empty(), session.as_mut()) {
            session.keyboard()?;
            session.dispatch_until(now_ns() + 1_000 * MS)?;
            for combo in &options.setup_keys {
                press(session, combo)?;
                session.dispatch_until(now_ns() + 500 * MS)?;
            }
            line["setup_keys"] = json!(options.setup_keys);
        }
        if let (Some(file), Some(session)) = (&options.open_file, session.as_mut()) {
            session.dispatch_until(now_ns() + 1_000 * MS)?;
            let opened = quick_open_once(session, file, false)?;
            line["open_file"] = json!(file);
            line["open_file_ms"] = opened["stable_ms"].clone();
        }
        let settle_deadline = now_ns() + options.settle_ms * MS;
        match session.as_mut() {
            Some(session) => session.dispatch_until(settle_deadline)?,
            None => std::thread::sleep(Duration::from_millis(options.settle_ms)),
        }
        line["settle_ms"] = json!(options.settle_ms);
        let self_pid = rustix::process::getpid().as_raw_nonzero().get() as u32;
        let idle = idle_measure(session.as_mut(), self_pid, false, pid, options.idle_ms)?;
        if let Value::Object(fields) = idle {
            for (key, value) in fields {
                line[key] = value;
            }
        }
    }
    if let Some(session) = &session {
        line["capture_failures"] = json!(session.state.capture_failures);
    }
    if let Some(message) = &error {
        line["error"] = json!(message);
    }
    emit(&line);

    if options.keep_open && error.is_none() {
        return Ok(());
    }
    let self_pid = rustix::process::getpid().as_raw_nonzero().get() as u32;
    let own: Vec<u32> = procfs::descendants(&procfs::all_stats(Path::new(PROC)), self_pid, false);
    for p in own {
        if p != pid
            && let Some(p) = Pid::from_raw(p as i32)
        {
            let _ = kill_process(p, Signal::TERM);
        }
    }
    stop_tree(pid, Duration::from_secs(5));
    let _ = child.wait();
    reap_orphans();
    if let Some(dir) = &options.profile {
        let _ = fs::remove_dir_all(dir);
    }
    if error.is_some() {
        bail!("la medición de arranque falló");
    }
    Ok(())
}

/// Collects re-parented children (we are a subreaper) so none stays zombie.
fn reap_orphans() {
    let self_pid = rustix::process::getpid().as_raw_nonzero().get() as u32;
    let deadline = now_ns() + 3_000 * MS;
    loop {
        let own = procfs::descendants(&procfs::all_stats(Path::new(PROC)), self_pid, false);
        if own.is_empty() || now_ns() > deadline {
            for p in own {
                if let Some(p) = Pid::from_raw(p as i32) {
                    let _ = kill_process(p, Signal::KILL);
                }
            }
            let _ = rustix::process::waitpid(None, rustix::process::WaitOptions::NOHANG);
            return;
        }
        let _ = rustix::process::waitpid(None, rustix::process::WaitOptions::NOHANG);
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// `idle`: memory and idle CPU/frames of an app that is already running.
pub fn idle(app: &str, pid: u32, settle_ms: u64, idle_ms: u64) -> Result<()> {
    let mut session = Session::connect()?;
    session.start_capture()?;
    session.dispatch_until(now_ns() + settle_ms * MS)?;
    let mut line = json!({"kind": "idle", "app": app, "pid": pid, "settle_ms": settle_ms});
    let idle = idle_measure(Some(&mut session), pid, true, pid, idle_ms)?;
    if let Value::Object(fields) = idle {
        for (key, value) in fields {
            line[key] = value;
        }
    }
    emit(&line);
    Ok(())
}

/// Waits until no frame arrived for `quiet_ms` (at most `max_ms`).
fn wait_quiet(session: &mut Session, quiet_ms: u64, max_ms: u64) -> Result<bool> {
    let deadline = now_ns() + max_ms * MS;
    loop {
        let last = session.state.frames.last().map_or(0, |f| f.ts_ns);
        let now = now_ns();
        if now.saturating_sub(last) >= quiet_ms * MS {
            return Ok(true);
        }
        if now >= deadline {
            return Ok(false);
        }
        session.dispatch_for(Duration::from_millis(5))?;
    }
}

fn press(session: &mut Session, combo: &str) -> Result<u64> {
    let stroke = parse_combo(combo)?;
    session.queue_stroke(&stroke)?;
    session.flush_now()
}

/// Prepares keyboard and capture, and lets the app receive the new keymap
/// with a harmless Shift before anything is measured.
fn input_session() -> Result<Session> {
    let mut session = Session::connect()?;
    session.keyboard()?;
    session.start_capture()?;
    press(&mut session, "shift")?;
    session.dispatch_until(now_ns() + 300 * MS)?;
    Ok(session)
}

fn quick_open_once(session: &mut Session, file: &str, close_after: bool) -> Result<Value> {
    session.keyboard()?;
    let region = session.output_region();
    press(session, "ctrl+p")?;
    session.dispatch_until(now_ns() + 500 * MS)?;
    for stroke in text_strokes(file)? {
        session.queue_stroke(&stroke)?;
        session.flush_now()?;
        session.dispatch_until(now_ns() + 30 * MS)?;
    }
    // Let the finder finish (§3.2).
    session.dispatch_until(now_ns() + 1_000 * MS)?;
    let mark = press(session, "enter")?;
    let deadline = mark + 10_000 * MS;
    let quiet = 100 * MS;
    let found = session.dispatch_while(deadline, |state| {
        stable_frame(&state.frames, &region, mark, quiet, now_ns()).is_some()
    })?;
    let end = now_ns();
    let frames = &session.state.frames;
    let first = first_damage_after(frames, &region, mark).map(|f| f.ts_ns);
    let stable = stable_frame(frames, &region, mark, quiet, end).map(|f| f.ts_ns);
    let result = json!({
        "first_frame_ms": first.map(|at| ms_between(mark, at)),
        "stable_ms": stable.map(|at| ms_between(mark, at)),
        "timeout": !found,
    });
    if close_after {
        session.dispatch_until(now_ns() + 300 * MS)?;
        press(session, "ctrl+w")?;
        wait_quiet(session, 300, 3_000)?;
    }
    Ok(result)
}

/// `quick-open`: Ctrl+P, name, 1 s, Enter → first stable frame (M3).
pub fn quick_open(app: &str, file: &str, repeat: usize, close_after: bool) -> Result<()> {
    let mut session = input_session()?;
    for rep in 0..repeat.max(1) {
        wait_quiet(&mut session, 200, 3_000)?;
        let close = close_after || rep + 1 < repeat;
        let mut line = quick_open_once(&mut session, file, close)?;
        line["kind"] = json!("quick_open");
        line["app"] = json!(app);
        line["file"] = json!(file);
        line["rep"] = json!(rep);
        emit(&line);
    }
    Ok(())
}

/// `type`: key and its deletion alternated; key → first damaged frame (M4).
pub fn type_keys(
    app: &str,
    file: Option<&str>,
    count: usize,
    interval_ms: u64,
    key: &str,
) -> Result<()> {
    let mut session = input_session()?;
    let region = session.output_region();
    let letter = parse_combo(key)?;
    let erase = parse_combo("backspace")?;
    let mut samples = Vec::with_capacity(count);
    let mut missed = 0usize;
    let mut not_quiet = 0usize;
    for index in 0..count {
        session.dispatch_until(now_ns() + interval_ms * MS)?;
        if !wait_quiet(&mut session, 50, 1_000)? {
            not_quiet += 1;
        }
        let stroke = if index % 2 == 0 { &letter } else { &erase };
        session.queue_stroke(stroke)?;
        let mark = session.flush_now()?;
        let deadline = mark + 1_000 * MS;
        session.dispatch_while(deadline, |state| {
            first_damage_after(&state.frames, &region, mark).is_some()
        })?;
        match first_damage_after(&session.state.frames, &region, mark) {
            Some(frame) => samples.push((frame.ts_ns - mark) as f64 / 1e6),
            None => missed += 1,
        }
    }
    let summary = summarize(&samples);
    emit(&json!({
        "kind": "type",
        "app": app,
        "file": file,
        "count": count,
        "interval_ms": interval_ms,
        "p50_ms": summary.map(|s| round2(s.p50)),
        "p95_ms": summary.map(|s| round2(s.p95)),
        "max_ms": summary.map(|s| round2(s.max)),
        "missed": missed,
        "not_quiet": not_quiet,
        "clock": session.clock().name(),
        "samples_ms": samples.iter().map(|v| round2(*v)).collect::<Vec<_>>(),
    }));
    Ok(())
}

/// `scroll`: continuous wheel over the window center; frame intervals (M5/M6).
pub fn scroll(app: &str, file: Option<&str>, seconds: f64, every_ms: u64) -> Result<()> {
    let mut session = Session::connect()?;
    session.pointer()?;
    session.start_capture()?;
    let region = session.output_region();
    session.move_pointer((region.width / 2) as u32, (region.height / 2) as u32)?;
    session.dispatch_until(now_ns() + 300 * MS)?;
    wait_quiet(&mut session, 200, 3_000)?;
    let start = now_ns();
    let end = start + (seconds * 1e9) as u64;
    let mut notches = 0usize;
    let mut next = start;
    while now_ns() < end {
        session.queue_wheel()?;
        session.flush_now()?;
        notches += 1;
        next += every_ms * MS;
        session.dispatch_until(next)?;
    }
    // Frames that answer the last notches.
    session.dispatch_until(now_ns() + 100 * MS)?;
    let stop = now_ns();
    let frames = &session.state.frames;
    let first = first_damage_after(frames, &region, start).map(|f| f.ts_ns);
    let intervals = intervals_ms(
        frames,
        &region,
        first.unwrap_or(start),
        stop.min(end + 50 * MS),
    );
    let summary = summarize(&intervals);
    emit(&json!({
        "kind": "scroll",
        "app": app,
        "file": file,
        "seconds": seconds,
        "every_ms": every_ms,
        "notches": notches,
        "frames": if first.is_some() { intervals.len() + 1 } else { 0 },
        "first_frame_ms": first.map(|at| ms_between(start, at)),
        "p50_ms": summary.map(|s| round2(s.p50)),
        "p95_ms": summary.map(|s| round2(s.p95)),
        "max_ms": summary.map(|s| round2(s.max)),
        "over_33ms": intervals.iter().filter(|v| **v > 33.4).count(),
        "clock": session.clock().name(),
    }));
    Ok(())
}

/// `key`: injects combinations in order (setup steps of the scripts).
pub fn keys(combos: &[String], gap_ms: u64) -> Result<()> {
    let mut session = Session::connect()?;
    session.keyboard()?;
    press(&mut session, "shift")?;
    session.dispatch_until(now_ns() + 100 * MS)?;
    for combo in combos {
        if let Some(text) = combo.strip_prefix("text:") {
            for stroke in text_strokes(text)? {
                session.queue_stroke(&stroke)?;
                session.flush_now()?;
                session.dispatch_until(now_ns() + 30 * MS)?;
            }
        } else {
            press(&mut session, combo)?;
        }
        session.dispatch_until(now_ns() + gap_ms * MS)?;
    }
    session.roundtrip()?;
    emit(&json!({"kind": "key", "combos": combos}));
    Ok(())
}

/// `stop`: ends an app left open with `launch --keep-open` and deletes its
/// profile.
pub fn stop(pid: u32, profile: Option<&Path>) -> Result<()> {
    let signalled = stop_tree(pid, Duration::from_secs(5));
    if let Some(dir) = profile {
        let _ = fs::remove_dir_all(dir);
    }
    emit(&json!({"kind": "stop", "pid": pid, "processes": signalled}));
    Ok(())
}

/// `wait-wayland`: waits until `$WAYLAND_DISPLAY` accepts connections (a
/// compositor that is still starting), without shell sleep loops.
pub fn wait_wayland(timeout_ms: u64) -> Result<()> {
    let start = now_ns();
    let deadline = start + timeout_ms * MS;
    loop {
        match Session::connect() {
            Ok(_) => {
                emit(&json!({"kind": "wayland_ready", "waited_ms": ms_between(start, now_ns())}));
                return Ok(());
            }
            Err(error) if now_ns() >= deadline => {
                return Err(error.context(format!("el compositor no respondió en {timeout_ms} ms")));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

/// `probe`: which of the bench protocols the compositor offers to an
/// ordinary client (nothing is captured).
pub fn probe() -> Result<()> {
    let session = Session::connect()?;
    let globals = session.globals();
    let mut offered = serde_json::Map::new();
    for name in crate::wl::PROBED {
        let version = globals
            .iter()
            .find(|(interface, _)| interface == name)
            .map(|(_, v)| *v);
        offered.insert((*name).to_owned(), json!(version));
    }
    let mut all: Vec<String> = globals
        .iter()
        .map(|(name, v)| format!("{name} v{v}"))
        .collect();
    all.sort();
    all.dedup();
    emit(&json!({"kind": "probe", "bench_protocols": offered, "globals": all}));
    Ok(())
}
