//! The images side of the chat glue
//! (`docs/specs/09-etapa7-conexiones-imagenes-comentarios.md` §5.3.2, §5.3.4,
//! E7-G): which connection takes images, the file dialog of the paperclip, the
//! notices the chat asks for and the folder of the conversation on screen.
//!
//! A child module of [`crate::agents`] so the `impl` below reaches `Agents`'
//! private fields without growing `agents.rs`. The viewer (`ChatEvent::
//! OpenImage`) and the "Mencionar en el chat" drag live in the workspace
//! (`crate::image_viewer`, `Workspace::new`), which owns the window layers.

use std::path::PathBuf;

use cincel_acp::{AgentCommand, PromptBlock};
use cincel_chat::{
    ChatEvent, IMAGE_EXTENSIONS, NoticeLevel, PICK_DIALOG_FILTER, PICK_DIALOG_TITLE,
};
use gpui::{Context, Window};

use super::Agents;

#[cfg(all(test, feature = "test-support"))]
thread_local! {
    /// How many times the file dialog was asked for. The tests never open the
    /// real one (it would talk to the desktop portal over D-Bus): they read
    /// this and then hand the "chosen" files to [`Agents::attach_picked`],
    /// which is what the dialog's answer calls.
    static DIALOG_REQUESTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many times the file dialog of the paperclip was asked for on this
/// thread (tests).
#[cfg(all(test, feature = "test-support"))]
pub(crate) fn dialog_requests() -> usize {
    DIALOG_REQUESTS.with(std::cell::Cell::get)
}

impl Agents {
    /// Tells the chat whether the active connection takes images
    /// (`ChatPanel::set_image_support`): `Some(announced)` once the agent
    /// connected, `None` without a connection (D6).
    pub(super) fn publish_image_support(&self, support: Option<bool>, cx: &mut Context<Self>) {
        self.chat
            .update(cx, |chat, cx| chat.set_image_support(support, cx));
    }

    /// Points the chat at the folder of the conversation on screen
    /// (`<state>/conversations/<id>/`), where its sent images are read back
    /// from once their bytes are no longer in memory; `None` without a
    /// project store or a conversation. Called every time the conversation on
    /// screen changes.
    pub(super) fn sync_conversation_dir(&self, cx: &mut Context<Self>) {
        let dir = match (&self.store, &self.conversation) {
            (Some(store), Some(conversation)) => Some(store.conversation_dir(&conversation.id)),
            _ => None,
        };
        self.chat
            .update(cx, |chat, cx| chat.set_conversation_dir(dir, cx));
    }

    /// A prompt with images just left: the conversation is saved right away,
    /// which writes each image to `conversations/<id>/images/<sha256>.<ext>`
    /// (`ConversationStore::save`, §5.3.4) instead of waiting for the next
    /// autosave. The user message is already in the transcript when the chat
    /// emits the command.
    pub(super) fn store_sent_images(&mut self, command: &AgentCommand, cx: &mut Context<Self>) {
        if let AgentCommand::Prompt { blocks, .. } = command
            && blocks
                .iter()
                .any(|block| matches!(block, PromptBlock::Image { .. }))
        {
            self.save_conversation_now(cx);
        }
    }

    /// The chat events about images the workspace answers here. `true` when
    /// `event` was one of them.
    pub(super) fn handle_image_event(
        &mut self,
        event: &ChatEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match event {
            ChatEvent::PickImages => self.pick_images(window, cx),
            ChatEvent::Notify { level, text } => show_notice(*level, text, cx),
            // The viewer is a layer of the workspace.
            ChatEvent::OpenImage { .. } => {}
            _ => return false,
        }
        true
    }

    /// The paperclip: the system file dialog ("Adjuntar imagen", several
    /// files, filter "Imágenes"), through the xdg portal like "Abrir carpeta"
    /// (`Workspace::pick_folder`). Its answer goes to
    /// [`Agents::attach_picked`].
    fn pick_images(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if cfg!(all(test, feature = "test-support")) {
            #[cfg(all(test, feature = "test-support"))]
            DIALOG_REQUESTS.with(|requests| requests.set(requests.get() + 1));
            return;
        }
        let start = self
            .project
            .as_ref()
            .map(|project| project.read(cx).root().to_path_buf());
        let (sender, receiver) = async_channel::bounded::<Vec<PathBuf>>(1);
        // `rfd::AsyncFileDialog` talks to the portal over D-Bus: its future
        // runs on the background executor and the answer comes back through
        // the channel.
        cx.background_executor()
            .spawn(async move {
                let mut dialog = rfd::AsyncFileDialog::new()
                    .set_title(PICK_DIALOG_TITLE)
                    .add_filter(PICK_DIALOG_FILTER, &IMAGE_EXTENSIONS);
                if let Some(start) = start {
                    dialog = dialog.set_directory(start);
                }
                let picked: Vec<PathBuf> = dialog
                    .pick_files()
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|handle| handle.path().to_path_buf())
                    .collect();
                let _ = sender.send(picked).await;
            })
            .detach();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(paths) = receiver.recv().await else {
                return;
            };
            if paths.is_empty() {
                tracing::debug!("el usuario canceló el diálogo de imágenes");
                return;
            }
            let _ = this.update(cx, |agents, cx| agents.attach_picked(paths, cx));
        })
        .detach();
    }

    /// The files the dialog returned: attached to the draft. Each is checked
    /// by content (a file that is not an image is refused with its notice),
    /// and the capability is checked again, since the connection may have
    /// changed while the dialog was open.
    pub(crate) fn attach_picked(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        self.chat
            .update(cx, |chat, cx| chat.attach_paths(&paths, cx));
    }
}

/// A notice of the chat as a pop-up (`toast`), in the tone it asks for.
fn show_notice(level: NoticeLevel, text: &str, cx: &mut Context<Agents>) {
    match level {
        NoticeLevel::Info => crate::toast::info(text.to_string(), cx),
        NoticeLevel::Warning => crate::toast::warn(text.to_string(), cx),
        NoticeLevel::Error => crate::toast::error(text.to_string(), cx),
    }
}
