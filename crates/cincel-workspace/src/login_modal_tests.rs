//! Tests of the "Avisos del inicio de sesión" fixes to `connection_modal.rs`:
//!
//! * (a) the muted line next to the link (`crate::connection_modal::
//!   link_expiry_notice`, exercised end to end through `render_connect`);
//! * (b) Antigravity's own 5-minute login-server timeout
//!   (`is_antigravity_link_expired`) gets a plain-language message, with the
//!   raw English text moved to "Ver detalles técnicos" instead of lost;
//! * (c) a link already on screen survives into the error screen.
//!
//! Events are fed to the modal by hand (`ConnectionsModal::handle_login_event`,
//! `background: false`, exactly as `crate::connection_modal`'s own module
//! doc describes for tests): deterministic, and independent from the real
//! login thread `choose` still starts in the background (it gets cancelled
//! and joined when the modal drops, `LoginSession`'s own `Drop`).
//!
//! The exact wording of (a) and (b) — `link_expiry_notice`,
//! `is_antigravity_link_expired`, `failure_message` — is unit-tested where
//! they live, in `connection_modal.rs`'s own `#[cfg(test)] mod tests`; this
//! file checks the GPUI-visible behavior: what step the flow lands on, what
//! survives into it, and that the relevant elements actually paint
//! (`debug_selector`/`debug_bounds`, the same mechanism `title_bar_tests.rs`
//! and `click_tests.rs` use).

use cincel_connections::{AgentKind, LoginEvent, LoginFailure};
use gpui::{Entity, TestAppContext, VisualTestContext};

use crate::connection_modal::{ConnectionsModal, Step};
use crate::test_support::{FAKE_AGY_URL, FakeEnv, isolate_state};

fn init_test(cx: &mut TestAppContext) {
    isolate_state();
    cx.update(|cx| crate::init(cincel_settings::Config::default(), cx));
}

/// A closed modal over a fresh fake engine, `background: false` (the doc
/// comment above): the real login thread still runs once `choose` starts
/// it, but nothing drains its events automatically, so the test controls
/// exactly which `LoginEvent`s the modal sees and when.
fn open_modal(
    cx: &mut TestAppContext,
) -> (Entity<ConnectionsModal>, FakeEnv, &mut VisualTestContext) {
    init_test(cx);
    let env = FakeEnv::new();
    let connections = env.connections.clone();
    let (modal, cx) =
        cx.add_window_view(|window, cx| ConnectionsModal::new(connections, false, window, cx));
    (modal, env, cx)
}

/// (a): once a link is known, the muted expiry line paints next to it —
/// checked for both an ACP agent (Antigravity) and a pty one (Claude), the
/// two branches `link_expiry_notice` distinguishes.
#[gpui::test]
fn the_link_notice_paints_next_to_the_link_for_every_kind(cx: &mut TestAppContext) {
    for kind in [AgentKind::Claude, AgentKind::Antigravity] {
        let (modal, _env, cx) = open_modal(cx);
        cx.update(|window, cx| {
            modal.update(cx, |modal, cx| {
                modal.open_connect(window, cx);
                modal.choose(kind, window, cx);
            })
        });
        cx.run_until_parked();

        cx.update(|window, cx| {
            modal.update(cx, |modal, cx| {
                modal.handle_login_event(
                    LoginEvent::UrlDetected(FAKE_AGY_URL.to_string()),
                    window,
                    cx,
                );
            })
        });
        cx.run_until_parked();

        assert_eq!(
            modal.read_with(cx, |modal, _| modal.step().cloned()),
            Some(Step::Link),
            "{kind:?}"
        );
        assert!(
            cx.debug_bounds("login-url").is_some(),
            "{kind:?}: el enlace se pinta"
        );
        assert!(
            cx.debug_bounds("login-link-expiry").is_some(),
            "{kind:?}: el aviso de vencimiento se pinta junto al enlace"
        );
    }
}

/// (b) + (c): Antigravity's own "Onboarding failed: Timed out waiting…"
/// becomes the plain-language sentence, the raw text moves to "Ver detalles
/// técnicos" instead of disappearing, and the link already shown stays on
/// screen — above the error, with "Copiar" and "Abrir en el navegador"
/// still there (`render_link`, reused verbatim for `Step::Failed`).
#[gpui::test]
fn antigravity_link_expiry_gets_a_plain_message_and_keeps_the_link(cx: &mut TestAppContext) {
    let (modal, _env, cx) = open_modal(cx);
    cx.update(|window, cx| {
        modal.update(cx, |modal, cx| {
            modal.open_connect(window, cx);
            modal.choose(AgentKind::Antigravity, window, cx);
        })
    });
    cx.run_until_parked();

    cx.update(|window, cx| {
        modal.update(cx, |modal, cx| {
            modal.handle_login_event(
                LoginEvent::UrlDetected(FAKE_AGY_URL.to_string()),
                window,
                cx,
            );
        })
    });
    cx.run_until_parked();
    assert_eq!(
        modal.read_with(cx, |modal, _| modal.step().cloned()),
        Some(Step::Link)
    );

    let raw_message =
        "Onboarding failed: Timed out waiting for the authentication flow to complete";
    cx.update(|window, cx| {
        modal.update(cx, |modal, cx| {
            modal.handle_login_event(
                LoginEvent::Failed {
                    message: raw_message.to_string(),
                    reason: LoginFailure::Rejected,
                },
                window,
                cx,
            );
        })
    });
    cx.run_until_parked();

    let (step, url, output) = modal.read_with(cx, |modal, _| {
        let flow = modal.connect_flow().expect("sigue en el flujo de conexión");
        (flow.step.clone(), flow.url.clone(), flow.output.clone())
    });
    match step {
        Step::Failed { message } => assert_eq!(
            message,
            "El enlace venció sin completarse. Antigravity da 5 minutos para iniciar sesión."
        ),
        other => panic!("se esperaba Step::Failed: {other:?}"),
    }
    assert_eq!(
        url.as_deref(),
        Some(FAKE_AGY_URL),
        "(c): el enlace ya detectado sigue en el estado tras el error"
    );
    assert!(
        output.iter().any(|line| line.contains(raw_message)),
        "(b): el texto original en inglés queda accesible en «Ver detalles técnicos»: {output:?}"
    );

    // (c): the rendered error screen still paints the link box (with
    // "Copiar"/"Abrir en el navegador") above the error text.
    assert!(
        cx.debug_bounds("login-url").is_some(),
        "el enlace sigue pintado sobre el error"
    );
    assert!(cx.debug_bounds("connect-error").is_some());
}
