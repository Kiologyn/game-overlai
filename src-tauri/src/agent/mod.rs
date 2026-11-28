//! AI agents behind one interface. The chat layer talks to `AgentHandle` and receives
//! `AgentEvent`s; everything ACP-specific stays in `acp.rs`, everything agent-specific in
//! `spec.rs`. A supervisor task installs the adapter (first run), keeps one agent process
//! running and restarts it if it dies.

mod acp;
pub mod install;
pub mod spec;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::{Mutex as AsyncMutex, mpsc, oneshot};
use tracing::{info, warn};

pub use spec::AgentSpec;

use crate::protocol::{AgentOption, AgentState, AgentStatus, PermissionPrompt, ToolInfo, ToolStatus, Usage};

/// What the chat layer gets from an agent, independent of ACP.
#[derive(Debug)]
pub enum AgentEvent {
    Status(AgentStatus),
    Update { session_id: String, update: Update },
    Usage(Usage),
    /// Model / effort choices of the live session (or of a new one, before there is any).
    Options(Vec<AgentOption>),
    PermissionRequested { session_id: String, prompt: PermissionPrompt },
    PermissionClosed { id: String },
}

#[derive(Debug)]
pub enum Update {
    Text(String),
    Thought(String),
    Tool(ToolInfo),
    /// Partial change of an existing tool call.
    ToolPatch { id: String, title: Option<String>, kind: Option<String>, status: Option<ToolStatus> },
    Title(String),
}

#[derive(Clone)]
pub enum PromptBlock {
    Text(String),
}

/// Our MCP server (game screenshots), offered to every session the agent opens.
#[derive(Debug, Clone)]
pub struct McpEndpoint {
    pub name: &'static str,
    pub url: String,
    /// Sent as `Authorization: Bearer …`; other local programs can't use the server.
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    Cancelled,
    Refusal,
    MaxTokens,
    Other(String),
}

pub(crate) enum Cmd {
    NewSession { cwd: PathBuf, system_prompt: String, reply: oneshot::Sender<Result<String, String>> },
    /// Continues an earlier session (after a restart of the agent or the app): `session/resume`
    /// if the agent has it, else `session/load` (its history replay is not shown).
    RestoreSession {
        session_id: String,
        cwd: PathBuf,
        /// Agents don't keep the system prompt with a session: it is sent again.
        system_prompt: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Prompt { session_id: String, blocks: Vec<PromptBlock>, reply: oneshot::Sender<Result<StopReason, String>> },
    Cancel { session_id: String },
    /// Applies the user's choices (`Shared::prefs`) to the live session; without one, reports
    /// what a new session would get.
    ApplyOptions { session_id: Option<String> },
    /// Frees the session's process (~220 MB for Claude Code); it can be resumed later.
    CloseSession { session_id: String },
    /// Ends the connection; the supervisor starts the agent again (e.g. with a new proxy).
    Restart,
}

impl Cmd {
    /// Answers the command with an error without running it (agent unavailable).
    fn fail(self, msg: &str) {
        match self {
            Cmd::NewSession { reply, .. } => drop(reply.send(Err(msg.into()))),
            Cmd::RestoreSession { reply, .. } => drop(reply.send(Err(msg.into()))),
            Cmd::Prompt { reply, .. } => drop(reply.send(Err(msg.into()))),
            Cmd::Cancel { .. } | Cmd::CloseSession { .. } | Cmd::ApplyOptions { .. } | Cmd::Restart => {}
        }
    }
}

struct PendingPermission {
    session_id: String,
    reply: oneshot::Sender<Option<String>>,
}

/// State shared between the handle, the supervisor and the ACP connection.
pub(crate) struct Shared {
    spec: &'static AgentSpec,
    mcp: Option<McpEndpoint>,
    events: mpsc::UnboundedSender<AgentEvent>,
    /// +1 every time a new agent process is connected; sessions must be re-loaded after a change.
    generation: AtomicU64,
    permissions: Mutex<HashMap<String, PendingPermission>>,
    /// Sessions being loaded: their history replay must not reach the UI.
    replaying: Mutex<HashSet<String>>,
    /// Proxy the running agent process was started with; `None` while it is not running.
    proxy: Mutex<Option<crate::proxy::ProxyEnv>>,
    /// Prompts being answered: the agent is not restarted under them.
    prompts: AtomicUsize,
    /// The next start comes without the crash backoff (a restart we asked for).
    restart_now: AtomicBool,
    /// The user's model / effort choices: option id -> value. Applied to every session.
    prefs: Mutex<BTreeMap<String, String>>,
    /// Options of the live session as the agent last reported them (ACP form).
    options: Mutex<Vec<acp::RawOption>>,
}

impl Shared {
    fn emit(&self, event: AgentEvent) {
        let _ = self.events.send(event);
    }

    fn status(&self, state: AgentState, detail: Option<String>) {
        self.emit(AgentEvent::Status(AgentStatus { agent: self.spec.name.into(), state, detail }));
    }

    fn register_permission(&self, session_id: String, prompt: PermissionPrompt) -> oneshot::Receiver<Option<String>> {
        let (reply, rx) = oneshot::channel();
        self.permissions
            .lock()
            .unwrap()
            .insert(prompt.id.clone(), PendingPermission { session_id: session_id.clone(), reply });
        self.emit(AgentEvent::PermissionRequested { session_id, prompt });
        rx
    }

    fn resolve_permission(&self, id: &str, option_id: Option<String>) -> bool {
        let pending = self.permissions.lock().unwrap().remove(id);
        match pending {
            Some(p) => {
                let _ = p.reply.send(option_id);
                self.emit(AgentEvent::PermissionClosed { id: id.into() });
                true
            }
            None => false,
        }
    }

    /// Dismisses pending prompts of one session (or of all sessions with `None`).
    fn dismiss_permissions(&self, session_id: Option<&str>) {
        let ids: Vec<String> = self
            .permissions
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, p)| session_id.is_none_or(|s| p.session_id == s))
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.resolve_permission(&id, None);
        }
    }
}

#[derive(Clone)]
pub struct AgentHandle {
    pub spec: &'static AgentSpec,
    tx: mpsc::UnboundedSender<Cmd>,
    shared: Arc<Shared>,
}

impl AgentHandle {
    /// Starts the supervisor (installs the adapter on first run, then keeps the agent running).
    /// `proxy_setting` is `[network] proxy`; it is resolved on every (re)start of the agent.
    pub fn start(
        spec: &'static AgentSpec,
        agents_dir: PathBuf,
        proxy_setting: String,
        require_proxy: bool,
        prefs: BTreeMap<String, String>,
        mcp: Option<McpEndpoint>,
        events: mpsc::UnboundedSender<AgentEvent>,
    ) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let shared = Arc::new(Shared {
            spec,
            mcp,
            events,
            generation: AtomicU64::new(0),
            permissions: Mutex::new(HashMap::new()),
            replaying: Mutex::new(HashSet::new()),
            proxy: Mutex::new(None),
            prompts: AtomicUsize::new(0),
            restart_now: AtomicBool::new(false),
            prefs: Mutex::new(prefs),
            options: Mutex::new(Vec::new()),
        });
        tauri::async_runtime::spawn(supervise(
            spec,
            agents_dir,
            proxy_setting.clone(),
            require_proxy,
            shared.clone(),
            Arc::new(AsyncMutex::new(rx)),
        ));
        tauri::async_runtime::spawn(watch_proxy(proxy_setting, shared.clone(), tx.clone()));
        Self { spec, tx, shared }
    }

    /// Changes whenever the agent process is restarted.
    pub fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::SeqCst)
    }

    async fn call<T>(&self, make: impl FnOnce(oneshot::Sender<Result<T, String>>) -> Cmd) -> Result<T, String> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(make(reply)).map_err(|_| tr!("err_agent_stopped"))?;
        rx.await.unwrap_or_else(|_| Err(tr!("err_agent_restarted")))
    }

    pub async fn new_session(&self, cwd: PathBuf, system_prompt: String) -> Result<String, String> {
        self.call(|reply| Cmd::NewSession { cwd, system_prompt, reply }).await
    }

    pub async fn restore_session(&self, session_id: String, cwd: PathBuf, system_prompt: String) -> Result<(), String> {
        self.call(|reply| Cmd::RestoreSession { session_id, cwd, system_prompt, reply }).await
    }

    pub async fn prompt(&self, session_id: String, blocks: Vec<PromptBlock>) -> Result<StopReason, String> {
        self.shared.prompts.fetch_add(1, Ordering::SeqCst);
        let result = self.call(|reply| Cmd::Prompt { session_id, blocks, reply }).await;
        self.shared.prompts.fetch_sub(1, Ordering::SeqCst);
        result
    }

    pub fn cancel(&self, session_id: String) {
        self.shared.dismiss_permissions(Some(&session_id));
        let _ = self.tx.send(Cmd::Cancel { session_id });
    }

    /// Called when a chat is left: the agent process of that session is freed.
    pub fn close_session(&self, session_id: String) {
        self.shared.dismiss_permissions(Some(&session_id));
        let _ = self.tx.send(Cmd::CloseSession { session_id });
    }

    /// The user picked `value` for option `id`: applied to `session_id` (the live one) now, and
    /// to every session from then on.
    pub fn set_option(&self, session_id: Option<String>, id: String, value: String) {
        self.shared.prefs.lock().unwrap().insert(id, value);
        let _ = self.tx.send(Cmd::ApplyOptions { session_id });
    }

    /// `None` dismisses the prompt (the agent sees it as cancelled).
    pub fn respond_permission(&self, id: &str, option_id: Option<String>) -> bool {
        self.shared.resolve_permission(id, option_id)
    }
}

async fn supervise(
    spec: &'static AgentSpec,
    agents_dir: PathBuf,
    proxy_setting: String,
    require_proxy: bool,
    shared: Arc<Shared>,
    commands: Arc<AsyncMutex<mpsc::UnboundedReceiver<Cmd>>>,
) {
    let mut backoff = Duration::from_secs(1);
    let mut first: Option<Cmd> = None;
    let mut waiting_for_proxy = false;
    loop {
        // Re-read every start: the user may have turned the system proxy on/off meanwhile.
        let setting = proxy_setting.clone();
        let proxy = tauri::async_runtime::spawn_blocking(move || crate::proxy::resolve(&setting))
            .await
            .unwrap_or_else(|_| crate::proxy::resolve("none"));
        info!("agent network: {}", proxy.describe());

        // `[network] require_proxy_for`: this user must never reach the agent directly, so wait
        // for a proxy instead of starting.
        if require_proxy && proxy.url.is_none() {
            let msg = tr!("agent_needs_proxy", agent = spec.name);
            if !waiting_for_proxy {
                warn!("agent {}: no proxy ({}); not starting it without one", spec.id, proxy.describe());
                shared.status(AgentState::Error, Some(msg.clone()));
                waiting_for_proxy = true;
            }
            let mut rx = commands.lock().await;
            tokio::select! {
                cmd = rx.recv() => match cmd {
                    Some(cmd) => cmd.fail(&msg),
                    None => return,
                },
                _ = tokio::time::sleep(PROXY_CHECK / 4) => {}
            }
            continue;
        }
        if std::mem::take(&mut waiting_for_proxy) {
            info!("proxy is back: starting the agent");
        }

        if !install::is_installed(&agents_dir, spec) {
            shared.status(AgentState::Installing, Some(tr!("agent_installing_adapter", agent = spec.name)));
        }
        let dir = agents_dir.clone();
        let install_proxy = proxy.clone();
        let installed =
            tauri::async_runtime::spawn_blocking(move || install::ensure_installed(&dir, spec, &install_proxy)).await;
        let entry = match installed {
            Ok(Ok(entry)) => entry,
            other => {
                let msg = match other {
                    Ok(Err(msg)) => msg,
                    _ => tr!("err_install_interrupted"),
                };
                warn!("agent {} unavailable: {msg}", spec.id);
                shared.status(AgentState::Error, Some(msg.clone()));
                // Fail requests until one arrives after a retry delay; then try installing again.
                let Some(cmd) = commands.lock().await.recv().await else { return };
                cmd.fail(&msg);
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };

        shared.status(AgentState::Starting, None);
        let generation = shared.generation.fetch_add(1, Ordering::SeqCst) + 1;
        info!("starting agent {} (generation {generation})", spec.id);
        let started = Instant::now();
        *shared.proxy.lock().unwrap() = Some(proxy.clone());
        let result = acp::run(spec, &entry, &proxy, shared.clone(), commands.clone(), first.take()).await;
        *shared.proxy.lock().unwrap() = None;
        shared.dismiss_permissions(None);
        match result {
            Ok(()) => info!("agent {} connection closed", spec.id),
            Err(e) => warn!("agent {} stopped: {e}", spec.id),
        }
        if commands.lock().await.is_closed() {
            return;
        }
        if shared.restart_now.swap(false, Ordering::SeqCst) {
            shared.status(AgentState::Starting, Some(tr!("agent_restarting")));
            continue;
        }
        backoff = if started.elapsed() > Duration::from_secs(30) {
            Duration::from_secs(1)
        } else {
            (backoff * 2).min(Duration::from_secs(30))
        };
        shared.status(AgentState::Starting, Some(tr!("agent_restarting")));
        tokio::time::sleep(backoff).await;
    }
}

/// How often the system proxy is checked while the agent runs.
const PROXY_CHECK: Duration = Duration::from_secs(20);

/// The agent process gets the proxy at start. If the user turns their proxy (v2rayN & co.) on,
/// off or to another port later, the agent is restarted with the new one once it is idle;
/// chats then resume their sessions.
async fn watch_proxy(setting: String, shared: Arc<Shared>, tx: mpsc::UnboundedSender<Cmd>) {
    loop {
        tokio::time::sleep(PROXY_CHECK).await;
        if tx.is_closed() {
            return;
        }
        let Some(running) = shared.proxy.lock().unwrap().clone() else { continue };
        let s = setting.clone();
        let Ok(now) = tauri::async_runtime::spawn_blocking(move || crate::proxy::resolve(&s)).await else { continue };
        if now.url == running.url || shared.prompts.load(Ordering::SeqCst) > 0 {
            continue;
        }
        info!("proxy changed: {} -> {}; restarting the agent", running.describe(), now.describe());
        shared.restart_now.store(true, Ordering::SeqCst);
        if tx.send(Cmd::Restart).is_err() {
            return;
        }
        // Don't ask again before the agent is back with the new proxy.
        for _ in 0..60 {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if shared.proxy.lock().unwrap().as_ref().is_some_and(|p| p.url != running.url) {
                break;
            }
        }
    }
}
