//! The Connections section of the settings tab, seen from [`Agents`]
//! (`docs/specs/07-etapa5-productividad.md` §4.4, §10.3, §10.4).
//!
//! The settings tab ([`SettingsView`]) never touches the connections engine:
//! it emits [`SettingsViewEvent`]s and shows the [`ConnectionsInfo`] this
//! module pushes into it. Here live the decisions: renaming goes to the
//! store, "Volver a conectar", "Reparar", "Eliminar…" and "Conectar nuevo
//! agente…" open the existing flows of the connections modal, and the two
//! updates ("Actualizar adaptador", "Actualizar Node") run on their own OS
//! thread with a [`CancelToken`] that "Cancelar" trips.
//!
//! The ACP registry is asked once per session, the first time the
//! Connections section is on screen, and again with "Buscar
//! actualizaciones". Old adapter and runtime versions are pruned at start-up
//! (nothing runs yet) and whenever the agent process stops (D13): an update
//! never removes the version a live agent is using.

use std::collections::HashMap;
use std::sync::Arc;

use cincel_acp::AgentRegistry;
use cincel_connections::{
    AdapterProgress, AgentKind, CancelToken, Connections, ConnectionsError, RuntimeProgress,
};
use gpui::{Context, Entity, Focusable as _, Task, WeakEntity, Window};

use super::Agents;
use crate::settings_view::{
    CatalogState, ConnectionsInfo, InstalledAgent, NodeInfo, SettingsView, SettingsViewEvent,
    UpdateOperation, UpdateOutcome, UpdateTarget,
};

/// Every agent the section can list, in display order.
const AGENT_KINDS: [AgentKind; 3] = [AgentKind::Claude, AgentKind::Codex, AgentKind::Antigravity];

/// How the registry is obtained: `true` asks the network even when a fresh
/// cache exists ("Buscar actualizaciones"). Replaced by the tests.
pub(crate) type RegistryLoader = Arc<dyn Fn(bool) -> Result<AgentRegistry, String> + Send + Sync>;

/// What the Connections section needs `Agents` to remember.
pub(crate) struct SettingsBridge {
    /// The settings tab, while it is open.
    view: Option<WeakEntity<SettingsView>>,
    /// The registry of the last successful check.
    registry: Option<Arc<AgentRegistry>>,
    /// Whether the registry was asked in this session.
    checked: bool,
    catalog: CatalogState,
    /// Newer versions the registry publishes, per installed agent.
    adapter_updates: HashMap<AgentKind, String>,
    /// The newer Node the policy offers.
    node_update: Option<String>,
    outcomes: HashMap<UpdateTarget, UpdateOutcome>,
    operation: Option<UpdateOperation>,
    /// Trips the update in flight.
    token: Option<CancelToken>,
    /// Bumped by every update and cancel, so a late message of an older
    /// thread is ignored.
    generation: u64,
    _check_task: Option<Task<()>>,
    _update_task: Option<Task<()>>,
    loader: RegistryLoader,
    /// A modal opened from the tab gives the keyboard back to it.
    refocus: bool,
}

impl SettingsBridge {
    /// Nothing checked, nothing running. Without `background` (the tests)
    /// the default loader answers "offline" instead of touching the network.
    pub(crate) fn new(background: bool) -> Self {
        let loader: RegistryLoader = if background {
            Arc::new(|force| {
                let registry = if force {
                    AgentRegistry::fetch()
                } else {
                    AgentRegistry::load()
                };
                registry.map_err(|error| error.to_string())
            })
        } else {
            Arc::new(|_| Err("sin conexión".to_string()))
        };
        Self {
            view: None,
            registry: None,
            checked: false,
            catalog: CatalogState::NotChecked,
            adapter_updates: HashMap::new(),
            node_update: None,
            outcomes: HashMap::new(),
            operation: None,
            token: None,
            generation: 0,
            _check_task: None,
            _update_task: None,
            loader,
            refocus: false,
        }
    }
}

/// What the check thread reports.
struct CheckResult {
    registry: Result<Arc<AgentRegistry>, String>,
    adapters: Vec<(AgentKind, String)>,
    node: Option<String>,
}

/// What an update thread reports.
enum UpdateMessage {
    Progress { percent: Option<f32>, text: String },
    Finished(UpdateResult),
}

/// How an update ended.
enum UpdateResult {
    /// Installed; the new version.
    Done(String),
    /// "Cancelar".
    Cancelled,
    /// Anything else, already in Spanish.
    Failed(String),
}

impl Agents {
    /// The settings tab opened (or is still open): remember it and show it
    /// what there is.
    pub fn attach_settings_view(&mut self, view: &Entity<SettingsView>, cx: &mut Context<Self>) {
        self.settings.view = Some(view.downgrade());
        self.push_settings_view(cx);
    }

    /// Replaces how the registry is obtained (the tests hand a fake one).
    #[cfg(all(test, feature = "test-support"))]
    pub(crate) fn set_registry_loader(&mut self, loader: RegistryLoader) {
        self.settings.loader = loader;
    }

    /// Hands the settings tab, if open, the current summary.
    pub(crate) fn push_settings_view(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.settings.view.as_ref().and_then(WeakEntity::upgrade) else {
            self.settings.view = None;
            return;
        };
        let info = self.settings_info(cx);
        view.update(cx, |view, cx| view.set_connections_info(info, cx));
    }

    /// The summary of §4.4: the popover's rows, what is installed, and what
    /// the last check and update said.
    fn settings_info(&self, cx: &Context<Self>) -> ConnectionsInfo {
        let bridge = &self.settings;
        let adapters = self.connections.adapters();
        let agents = AGENT_KINDS
            .into_iter()
            .filter_map(|kind| {
                let version = adapters.installed(kind.agent_id())?;
                Some(InstalledAgent {
                    kind,
                    update: bridge
                        .adapter_updates
                        .get(&kind)
                        .filter(|update| **update != version)
                        .cloned(),
                    outcome: bridge.outcomes.get(&UpdateTarget::Adapter(kind)).cloned(),
                    version,
                })
            })
            .collect();
        let installed = self
            .connections
            .runtime()
            .installed()
            .map(|node| node.version);
        let node = NodeInfo {
            update: bridge
                .node_update
                .clone()
                .filter(|update| Some(update) != installed.as_ref()),
            outcome: bridge.outcomes.get(&UpdateTarget::Node).cloned(),
            installed,
        };
        ConnectionsInfo {
            connections: self.chat.read(cx).connections().to_vec(),
            agents,
            node,
            catalog: bridge.catalog,
            operation: bridge.operation.clone(),
        }
    }

    /// What the settings tab asks for. The workspace forwards every event
    /// of the tab here; the ones about `settings.json` are not ours.
    pub fn handle_settings_event(
        &mut self,
        event: &SettingsViewEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            SettingsViewEvent::Written | SettingsViewEvent::OpenSettingsFile => {}
            SettingsViewEvent::Rename { id, label } => {
                match self.connections.store().rename(*id, label) {
                    Ok(_) => {
                        tracing::info!("conexión renombrada desde la configuración");
                        self.refresh_connections(cx);
                    }
                    Err(error) => {
                        crate::toast::error(
                            format!("No se pudo renombrar la conexión: {error}"),
                            cx,
                        );
                    }
                }
            }
            SettingsViewEvent::Reconnect { id } => {
                self.settings.refocus = true;
                self.modal
                    .update(cx, |modal, cx| modal.open_relogin(*id, window, cx));
            }
            SettingsViewEvent::Repair { id } => {
                self.settings.refocus = true;
                self.modal
                    .update(cx, |modal, cx| modal.open_repair(*id, window, cx));
            }
            SettingsViewEvent::Delete { id } => {
                self.settings.refocus = true;
                self.modal.update(cx, |modal, cx| {
                    modal.open_delete(window, cx);
                    modal.pick_for_deletion(*id, cx);
                });
            }
            SettingsViewEvent::NewConnection => {
                self.settings.refocus = true;
                self.modal
                    .update(cx, |modal, cx| modal.open_connect(window, cx));
            }
            SettingsViewEvent::UpdateAdapter(kind) => {
                self.start_update(UpdateTarget::Adapter(*kind), cx);
            }
            SettingsViewEvent::UpdateNode => self.start_update(UpdateTarget::Node, cx),
            SettingsViewEvent::CheckUpdates => self.check_updates(true, cx),
            SettingsViewEvent::CancelUpdate => self.cancel_update(cx),
            SettingsViewEvent::ConnectionsShown => {
                if !self.settings.checked {
                    self.check_updates(false, cx);
                }
            }
        }
    }

    /// After a modal opened from the settings tab closes, the keyboard goes
    /// back to the tab. Returns whether it did.
    pub(crate) fn refocus_settings_after_modal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !std::mem::take(&mut self.settings.refocus) {
            return false;
        }
        let Some(view) = self.settings.view.as_ref().and_then(WeakEntity::upgrade) else {
            return false;
        };
        let handle = view.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        true
    }

    // ------------------------------------------------------------ checks

    /// Asks the registry (and nodejs.org, per the Node policy) which updates
    /// there are.
    fn check_updates(&mut self, force: bool, cx: &mut Context<Self>) {
        if self.settings.catalog == CatalogState::Checking {
            return;
        }
        self.settings.checked = true;
        self.settings.catalog = CatalogState::Checking;
        self.push_settings_view(cx);
        let connections = self.connections.clone();
        let loader = self.settings.loader.clone();
        if !self.background {
            let result = check(&connections, &loader, force);
            self.on_checked(result, cx);
            return;
        }
        let (sender, receiver) = async_channel::bounded::<CheckResult>(1);
        let spawned = std::thread::Builder::new()
            .name("cincel-check-updates".to_string())
            .spawn(move || {
                let _ = sender.send_blocking(check(&connections, &loader, force));
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "no se pudo buscar actualizaciones");
            self.settings.catalog = CatalogState::Offline;
            self.push_settings_view(cx);
            return;
        }
        self.settings._check_task = Some(cx.spawn(async move |this, cx| {
            if let Ok(result) = receiver.recv().await {
                let _ = this.update(cx, |agents, cx| agents.on_checked(result, cx));
            }
        }));
    }

    fn on_checked(&mut self, result: CheckResult, cx: &mut Context<Self>) {
        let bridge = &mut self.settings;
        match result.registry {
            Ok(registry) => {
                bridge.registry = Some(registry);
                bridge.catalog = CatalogState::Checked;
                bridge.adapter_updates = result.adapters.into_iter().collect();
            }
            Err(error) => {
                tracing::info!(%error, "no se pudo consultar el catálogo de agentes");
                bridge.catalog = CatalogState::Offline;
                bridge.adapter_updates.clear();
            }
        }
        bridge.node_update = result.node;
        bridge._check_task = None;
        self.push_settings_view(cx);
    }

    // ----------------------------------------------------------- updates

    /// "Actualizar a X.Y.Z" of an agent or of Node: one at a time.
    fn start_update(&mut self, target: UpdateTarget, cx: &mut Context<Self>) {
        if self.settings.operation.is_some() {
            return;
        }
        self.settings.generation += 1;
        let generation = self.settings.generation;
        let token = CancelToken::new();
        self.settings.token = Some(token.clone());
        self.settings.outcomes.remove(&target);
        let node_version = self.settings.node_update.clone();
        self.settings.operation = Some(UpdateOperation {
            target,
            percent: None,
            text: starting_text(target, node_version.as_deref()),
        });
        self.push_settings_view(cx);

        let connections = self.connections.clone();
        let registry = self.settings.registry.clone();
        let loader = self.settings.loader.clone();
        if !self.background {
            let mut messages = Vec::new();
            let result = run_update(
                &connections,
                registry,
                &loader,
                target,
                node_version.as_deref(),
                &token,
                &mut |message| messages.push(message),
            );
            for message in messages {
                self.on_update_message(generation, message, cx);
            }
            self.on_update_message(generation, UpdateMessage::Finished(result), cx);
            return;
        }

        let (sender, receiver) = async_channel::unbounded::<UpdateMessage>();
        let spawned = std::thread::Builder::new()
            .name("cincel-update".to_string())
            .spawn(move || {
                let result = run_update(
                    &connections,
                    registry,
                    &loader,
                    target,
                    node_version.as_deref(),
                    &token,
                    &mut |message| {
                        let _ = sender.send_blocking(message);
                    },
                );
                let _ = sender.send_blocking(UpdateMessage::Finished(result));
            });
        if let Err(error) = spawned {
            self.on_update_message(
                generation,
                UpdateMessage::Finished(UpdateResult::Failed(error.to_string())),
                cx,
            );
            return;
        }
        self.settings._update_task = Some(cx.spawn(async move |this, cx| {
            while let Ok(message) = receiver.recv().await {
                let alive = this
                    .update(cx, |agents, cx| {
                        agents.on_update_message(generation, message, cx)
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        }));
    }

    fn on_update_message(
        &mut self,
        generation: u64,
        message: UpdateMessage,
        cx: &mut Context<Self>,
    ) {
        if generation != self.settings.generation {
            return;
        }
        let Some(target) = self
            .settings
            .operation
            .as_ref()
            .map(|operation| operation.target)
        else {
            return;
        };
        match message {
            UpdateMessage::Progress { percent, text } => {
                if let Some(operation) = self.settings.operation.as_mut() {
                    operation.percent = percent;
                    operation.text = text;
                }
            }
            UpdateMessage::Finished(result) => {
                self.settings.operation = None;
                self.settings.token = None;
                self.settings._update_task = None;
                match result {
                    UpdateResult::Done(version) => {
                        let message = match target {
                            UpdateTarget::Adapter(kind) => {
                                self.settings.adapter_updates.remove(&kind);
                                let in_use = self.connection.is_some()
                                    && self
                                        .active
                                        .as_ref()
                                        .is_some_and(|active| active.agent_id == kind.agent_id());
                                tracing::info!(agente = kind.agent_id(), %version, "adaptador actualizado");
                                if in_use {
                                    format!(
                                        "Actualizado a {version} · se usará al volver a conectar"
                                    )
                                } else {
                                    format!("Actualizado a {version}")
                                }
                            }
                            UpdateTarget::Node => {
                                self.settings.node_update = None;
                                tracing::info!(%version, "Node privado actualizado");
                                format!("Node actualizado a {version}")
                            }
                        };
                        self.settings
                            .outcomes
                            .insert(target, UpdateOutcome::Updated(message));
                        // With no agent running nothing uses the previous
                        // version; otherwise it waits for the process to stop
                        // (D13).
                        if self.connection.is_none() {
                            self.prune_unused_versions();
                        }
                    }
                    UpdateResult::Cancelled => {}
                    UpdateResult::Failed(reason) => {
                        tracing::warn!(%reason, "no se pudo actualizar");
                        self.settings.outcomes.insert(
                            target,
                            UpdateOutcome::Failed(format!("No se pudo actualizar: {reason}")),
                        );
                    }
                }
            }
        }
        self.push_settings_view(cx);
    }

    /// "Cancelar": trips the token and forgets the update right away; the
    /// thread removes what it left half-done and its late answer is ignored.
    fn cancel_update(&mut self, cx: &mut Context<Self>) {
        if let Some(token) = self.settings.token.take() {
            token.cancel();
            tracing::info!("actualización cancelada");
        }
        self.settings.operation = None;
        self.settings.generation += 1;
        self.settings._update_task = None;
        self.push_settings_view(cx);
    }

    // ----------------------------------------------------------- pruning

    /// Removes the adapter and runtime versions nothing uses
    /// (`Connections::prune_unused(&[])`). Called at start-up, before any
    /// agent runs, and when the agent process stops. On its own thread in
    /// the real window: removing an old binary agent can take a moment.
    pub(crate) fn prune_unused_versions(&self) {
        if self.settings.operation.is_some() {
            // An update is writing next to the versions being removed.
            return;
        }
        let connections = self.connections.clone();
        let prune = move || match connections.prune_unused(&[]) {
            Ok(report) if !report.runtimes.is_empty() || !report.adapters.is_empty() => {
                tracing::info!(
                    runtimes = ?report.runtimes,
                    adaptadores = ?report.adapters,
                    "se borraron versiones que ya no se usan"
                );
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "no se pudieron borrar las versiones viejas"),
        };
        if !self.background {
            prune();
            return;
        }
        if let Err(error) = std::thread::Builder::new()
            .name("cincel-prune".to_string())
            .spawn(prune)
        {
            tracing::warn!(%error, "no se pudo lanzar la limpieza de versiones viejas");
        }
    }
}

/// The check itself: the registry, then the newer version of every
/// installed agent and of Node. Blocks on the network.
fn check(connections: &Connections, loader: &RegistryLoader, force: bool) -> CheckResult {
    let registry = loader(force).map(Arc::new);
    let adapters = match &registry {
        Ok(registry) => AGENT_KINDS
            .into_iter()
            .filter_map(|kind| {
                connections
                    .adapters()
                    .update_available(registry, kind)
                    .map(|version| (kind, version))
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    let node = match connections.runtime().update_available() {
        Ok(node) => node,
        Err(error) => {
            tracing::info!(%error, "no se pudo consultar la versión de Node");
            None
        }
    };
    CheckResult {
        registry,
        adapters,
        node,
    }
}

/// One update, start to end. Blocks on the network and on `npm`.
fn run_update(
    connections: &Connections,
    registry: Option<Arc<AgentRegistry>>,
    loader: &RegistryLoader,
    target: UpdateTarget,
    node_version: Option<&str>,
    token: &CancelToken,
    send: &mut dyn FnMut(UpdateMessage),
) -> UpdateResult {
    let result = match target {
        UpdateTarget::Adapter(kind) => {
            let registry = match registry {
                Some(registry) => registry,
                None => match loader(false) {
                    Ok(registry) => Arc::new(registry),
                    Err(error) => return UpdateResult::Failed(error),
                },
            };
            let node = connections.runtime().installed();
            connections
                .adapters()
                .update(
                    &registry,
                    kind,
                    node.as_ref(),
                    &mut |step| {
                        if let Some((percent, text)) = adapter_progress(kind, &step) {
                            send(UpdateMessage::Progress { percent, text });
                        }
                    },
                    token,
                )
                .map(|install| install.version)
        }
        UpdateTarget::Node => connections
            .runtime()
            .update(
                &mut |step| {
                    if let Some((percent, text)) = node_progress(node_version, &step) {
                        send(UpdateMessage::Progress { percent, text });
                    }
                },
                token,
            )
            .map(|node| node.version),
    };
    match result {
        Ok(version) => UpdateResult::Done(version),
        Err(ConnectionsError::Cancelled) => UpdateResult::Cancelled,
        Err(_) if token.is_cancelled() => UpdateResult::Cancelled,
        Err(error) => UpdateResult::Failed(error.to_string()),
    }
}

/// The first line of the bar, before any progress arrives.
fn starting_text(target: UpdateTarget, node_version: Option<&str>) -> String {
    match target {
        UpdateTarget::Adapter(kind) => adapter_text(kind, None),
        UpdateTarget::Node => node_text(node_version, None),
    }
}

/// "Actualizando el adaptador de Claude… 45 %".
fn adapter_text(kind: AgentKind, percent: Option<f32>) -> String {
    let base = if kind.needs_node() {
        format!("Actualizando el adaptador de {}…", kind.display_name())
    } else {
        format!("Actualizando {}…", kind.full_name())
    };
    with_percent(base, percent)
}

/// "Actualizando Node a v24.2.0… 45 %".
fn node_text(version: Option<&str>, percent: Option<f32>) -> String {
    let base = match version {
        Some(version) => format!("Actualizando Node a {version}…"),
        None => "Actualizando Node…".to_string(),
    };
    with_percent(base, percent)
}

fn with_percent(base: String, percent: Option<f32>) -> String {
    match percent {
        Some(percent) => format!("{base} {} %", percent.round() as i64),
        None => base,
    }
}

fn percent_of(done: u64, total: Option<u64>) -> Option<f32> {
    total
        .filter(|total| *total > 0)
        .map(|total| (done as f32 * 100. / total as f32).min(100.))
}

fn adapter_progress(kind: AgentKind, step: &AdapterProgress) -> Option<(Option<f32>, String)> {
    let percent = match step {
        AdapterProgress::Downloading { done, total, .. } => percent_of(*done, *total),
        AdapterProgress::Verifying | AdapterProgress::Extracting => Some(100.),
        AdapterProgress::Installing { .. } | AdapterProgress::Retrying { .. } => None,
        AdapterProgress::Done { .. } => return None,
    };
    Some((percent, adapter_text(kind, percent)))
}

fn node_progress(version: Option<&str>, step: &RuntimeProgress) -> Option<(Option<f32>, String)> {
    let (version, percent) = match step {
        RuntimeProgress::Downloading {
            version,
            done,
            total,
        } => (Some(version.as_str()), percent_of(*done, *total)),
        RuntimeProgress::Verifying | RuntimeProgress::Extracting => (version, Some(100.)),
        RuntimeProgress::Resolving | RuntimeProgress::Retrying { .. } => (version, None),
        RuntimeProgress::Done => return None,
    };
    Some((percent, node_text(version, percent)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_texts_follow_the_spec() {
        assert_eq!(
            adapter_text(AgentKind::Claude, Some(45.2)),
            "Actualizando el adaptador de Claude… 45 %"
        );
        assert_eq!(
            node_text(Some("v24.2.0"), None),
            "Actualizando Node a v24.2.0…"
        );
        assert_eq!(percent_of(50, Some(200)), Some(25.));
        assert_eq!(percent_of(50, None), None);
    }
}
