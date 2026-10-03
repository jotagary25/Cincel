//! Closing gpui-kit's popup menus by themselves
//! (`docs/specs/09-etapa7-conexiones-imagenes-comentarios.md` §4.1).
//!
//! A `PopupMenu` closes on a click outside and on `Esc`. The other two ways
//! of leaving a drop-down menu are not its own: the keyboard focus going to
//! another element (the editor, `Ctrl+L`, a modal) and the window losing the
//! focus. [`dismiss_on_focus_loss`] adds them from Cincel's side, by sending
//! the menu the same `Cancel` its `Esc` would, so the menu closes the way it
//! always does (its `DismissEvent`, its own focus restore) and gpui-kit does
//! not change.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{App, Context, FocusHandle, Focusable as _, Subscription, Window};
use gpui_kit::base::actions::Cancel;
use gpui_kit::component::menu::PopupMenu;

/// Makes `menu` close when the focus leaves it or the window goes to the
/// background. The subscriptions live as long as the returned value: hold it
/// while the menu is open, and replacing it (the next time a menu is built)
/// drops the old listeners instead of piling them up.
pub(crate) fn dismiss_on_focus_loss<T: 'static>(
    menu: &PopupMenu,
    window: &mut Window,
    cx: &mut Context<T>,
) -> Vec<Subscription> {
    let handle = menu.focus_handle(cx);
    // One `Cancel` per menu is enough: a normal dismissal moves the focus
    // away itself, which is a "focus out" too.
    let sent = Rc::new(Cell::new(false));
    let on_focus_out = {
        let (handle, sent) = (handle.clone(), sent.clone());
        cx.on_focus_out(&handle.clone(), window, move |_, _, window, cx| {
            cancel(&handle, &sent, window, cx);
        })
    };
    let on_deactivation = cx.observe_window_activation(window, move |_, window, cx| {
        if !window.is_window_active() {
            cancel(&handle, &sent, window, cx);
        }
    });
    vec![on_focus_out, on_deactivation]
}

/// Sends `Cancel` to the menu's own node, once, after the current frame's
/// focus bookkeeping (the listeners run while the window is drawing).
fn cancel(handle: &FocusHandle, sent: &Rc<Cell<bool>>, window: &mut Window, cx: &mut App) {
    if sent.replace(true) {
        return;
    }
    send_cancel(handle, window, cx);
}

/// Sends `Cancel` to the node of `handle` at the end of the current effect
/// cycle. Nothing happens when that node is not on screen any more (a menu
/// already closed), so a stale handle is harmless.
///
/// For shortcuts that gpui-kit's menu would otherwise survive: while a
/// context menu is open its element takes the focus back on every frame, so
/// `Ctrl+L` cannot pull the focus away from it and the menu has to be told
/// to close first.
pub(crate) fn send_cancel(handle: &FocusHandle, window: &mut Window, cx: &mut App) {
    let handle = handle.clone();
    let window_handle = window.window_handle();
    cx.defer(move |cx| {
        window_handle
            .update(cx, |_, window, cx| {
                handle.dispatch_action(&Cancel, window, cx);
            })
            .ok();
    });
}
