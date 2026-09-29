//! "Cancelar" during "Preparando…" really stops the download/install
//! thread instead of leaving it running unattended in the background
//! (`docs/specs/07-etapa5-productividad.md` §10.3, E5-J).
//!
//! **Deviation from the letter of §10.3's test recipe:** the recipe ("un
//! descargador de prueba que entrega 1 MB cada 100 ms") needs a
//! controllable, slow, cancellable downloader; that fixture lives inside
//! `cincel-connections`'s own `tests/engine.rs` (E5-C) and is exercised
//! there. `cincel-connections` is off limits for this subetapa (E5-J only
//! touches `workspace`/`chat`), and `cincel-workspace`'s own fixture
//! (`FakeEnv`) has no registry, so `Connections::prepare` fails almost
//! instantly (`AdapterMissing`/`RuntimeMissing`, no network call) rather
//! than running a genuine slow download. Simulating a slow download here
//! would mean a real `sleep`/timing-dependent race, which the task's own
//! rules forbid (no background loops, no sleeps) and which would make the
//! test flaky besides.
//!
//! What this file tests instead, deterministically and with no sleep: that
//! pressing "Cancelar" while `Step::Preparing` is up (1) sets the *real*
//! `CancelToken` the background thread holds (not a throwaway one, as
//! before E5-J), (2) resets the flow to "Elegí un agente" synchronously,
//! without waiting for the thread, and (3) never turns that thread's late
//! `Done` message (whatever `prepare` resolved to) into "Algo salió mal".
//! This covers the modal-side half of §10.3's criteria; the download loop's
//! own cancellation is E5-C's `tests/engine.rs`.

use cincel_connections::AgentKind;
use gpui::{TestAppContext, VisualTestContext};

use crate::connection_modal::{ConnectionsModal, Step};
use crate::test_support::{FakeEnv, isolate_state};

fn init_test(cx: &mut TestAppContext) {
    isolate_state();
    cx.update(|cx| crate::init(cincel_settings::Config::default(), cx));
}

fn open_modal(
    cx: &mut TestAppContext,
) -> (
    gpui::Entity<ConnectionsModal>,
    FakeEnv,
    &mut VisualTestContext,
) {
    init_test(cx);
    // No registry installed: `prepare` fails fast (`AdapterMissing`), but
    // the real background thread (`background: true`) still runs, still
    // holds a real `CancelToken`, and still races the test's single
    // threaded executor, which is exactly what this file checks against.
    let env = FakeEnv::empty();
    let connections = env.connections.clone();
    let (modal, cx) =
        cx.add_window_view(|window, cx| ConnectionsModal::new(connections, true, window, cx));
    (modal, env, cx)
}

#[gpui::test]
fn cancelling_prepare_sets_the_real_token_and_resets_at_once(cx: &mut TestAppContext) {
    let (modal, _env, cx) = open_modal(cx);

    modal.update_in(cx, |modal, window, cx| {
        modal.open_connect(window, cx);
        modal.choose(AgentKind::Antigravity, window, cx);
    });
    assert_eq!(
        modal.read_with(cx, |modal, _| modal
            .connect_flow()
            .map(|flow| flow.step.clone())),
        Some(Step::Preparing),
        "elegir un agente arranca «Preparando…»"
    );

    let token = modal
        .read_with(cx, |modal, _| modal.prepare_cancel_for_test())
        .expect("«Preparando…» tiene un token de cancelación real");
    assert!(
        !token.is_cancelled(),
        "todavía no se apretó «Cancelar»: nada debe estar cancelado"
    );

    modal.update_in(cx, |modal, window, cx| modal.cancel_login(window, cx));

    assert!(
        token.is_cancelled(),
        "«Cancelar» tiene que disparar el token real que sostiene el hilo, \
         no uno descartable (docs/specs/07-etapa5-productividad.md §10.3)"
    );
    assert_eq!(
        modal.read_with(cx, |modal, _| modal
            .connect_flow()
            .map(|flow| flow.step.clone())),
        Some(Step::Choose),
        "el modal vuelve a «Elegí un agente» de inmediato, sin esperar al hilo"
    );

    // The thread's late result (whatever `prepare` resolved to: `Cancelled`
    // if it noticed in time, or a plain `AdapterMissing` otherwise) must
    // never resurrect as "Algo salió mal".
    cx.run_until_parked();
    assert_eq!(
        modal.read_with(cx, |modal, _| modal
            .connect_flow()
            .map(|flow| flow.step.clone())),
        Some(Step::Choose),
        "el resultado tardío del hilo cancelado se descarta, nunca se muestra como fallo"
    );
}

#[gpui::test]
fn closing_the_modal_mid_prepare_also_cancels_the_token(cx: &mut TestAppContext) {
    let (modal, _env, cx) = open_modal(cx);

    modal.update_in(cx, |modal, window, cx| {
        modal.open_connect(window, cx);
        modal.choose(AgentKind::Antigravity, window, cx);
    });
    let token = modal
        .read_with(cx, |modal, _| modal.prepare_cancel_for_test())
        .expect("«Preparando…» tiene un token de cancelación real");

    // Closing (the `×`/`Esc` confirmado path, `docs/specs/07-etapa5-…md`
    // v1.1 note on the window's `×`) goes through the same `cancel_session`
    // as the "Cancelar" button: the deviation retired in this subetapa
    // ("Cerrar el modal durante «Preparando…» no corta una descarga en
    // curso", `modulos/workspace.md` E5-K) no longer applies.
    modal.update_in(cx, |modal, window, cx| modal.close(window, cx));

    assert!(
        token.is_cancelled(),
        "cerrar el modal a mitad de «Preparando…» también corta la descarga"
    );
}
