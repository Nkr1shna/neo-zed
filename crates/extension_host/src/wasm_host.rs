pub mod wit;

use crate::capability_granter::CapabilityGranter;
use crate::{ExtensionManifest, ExtensionSettings};
use anyhow::{Context as _, Result, anyhow, bail};
use async_trait::async_trait;
use dap::{DebugRequest, StartDebuggingRequestArgumentsRequest};
use extension::{
    CodeLabel, Command, CommandContext, Completion, ContextServerConfiguration, DebugAdapterBinary,
    DebugTaskDefinition, EventOutcome, ExtensionCapability, ExtensionHostProxy,
    KeyValueStoreDelegate, MountContext, ProjectDelegate, RemoteViewEvent, RemoteViewNode,
    RemoteViewTree, RenderReason, SlashCommand, SlashCommandArgumentCompletion, SlashCommandOutput,
    Symbol, VirtualListRange, WorktreeDelegate,
};
use fs::Fs;
use futures::future::LocalBoxFuture;
use futures::{
    AsyncBufReadExt as _, AsyncWriteExt as _, Future, FutureExt, StreamExt as _,
    channel::{
        mpsc::{self, UnboundedSender},
        oneshot,
    },
    future::BoxFuture,
    io::{BufReader, BufWriter},
};
use gpui::{App, AsyncApp, BackgroundExecutor, Task};
use http_client::HttpClient;
use language::LanguageName;
use lsp::LanguageServerName;
use moka::sync::Cache;
use node_runtime::NodeRuntime;
use release_channel::ReleaseChannel;
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use settings::Settings;
use std::{
    borrow::Cow,
    path::{Path, PathBuf},
    sync::{
        Arc, LazyLock, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering::SeqCst},
    },
    time::Duration,
};
use task::{DebugScenario, SpawnInTerminal, TaskTemplate, ZedDebugConfig};
use util::paths::SanitizedPath;
use wasmtime::{
    CacheStore, Engine, Store,
    component::{Component, ResourceTable},
};
use wasmtime_wasi::p2::{self as wasi, IoView as _};
use wit::Extension;

pub struct WasmHost {
    engine: Engine,
    release_channel: ReleaseChannel,
    http_client: Arc<dyn HttpClient>,
    node_runtime: NodeRuntime,
    #[allow(dead_code)]
    background_executor: BackgroundExecutor,
    pub(crate) proxy: Arc<ExtensionHostProxy>,
    fs: Arc<dyn Fs>,
    pub work_dir: PathBuf,
    /// The capabilities granted to extensions running on the host.
    pub(crate) granted_capabilities: Vec<ExtensionCapability>,
    _main_thread_message_task: Task<()>,
    main_thread_message_tx: mpsc::UnboundedSender<MainThreadCall>,
}

#[derive(Clone, Debug)]
pub struct WasmExtension {
    tx: UnboundedSender<ExtensionCall>,
    pub manifest: Arc<ExtensionManifest>,
    pub work_dir: Arc<Path>,
    #[allow(unused)]
    pub zed_api_version: Version,
    _task: Arc<Task<Result<(), gpui_tokio::JoinError>>>,
}

#[async_trait]
impl extension::Extension for WasmExtension {
    fn manifest(&self) -> Arc<ExtensionManifest> {
        self.manifest.clone()
    }

    fn work_dir(&self) -> Arc<Path> {
        self.work_dir.clone()
    }

    async fn language_server_command(
        &self,
        language_server_id: LanguageServerName,
        language_name: LanguageName,
        worktree: Arc<dyn WorktreeDelegate>,
    ) -> Result<Command> {
        self.call(|extension, store| {
            async move {
                let resource = store.data_mut().table().push(worktree)?;
                let command = extension
                    .call_language_server_command(
                        store,
                        &language_server_id,
                        &language_name,
                        resource,
                    )
                    .await?
                    .map_err(|err| store.data().extension_error(err))?;

                Ok(command.into())
            }
            .boxed()
        })
        .await?
    }

    async fn language_server_initialization_options(
        &self,
        language_server_id: LanguageServerName,
        language_name: LanguageName,
        worktree: Arc<dyn WorktreeDelegate>,
    ) -> Result<Option<String>> {
        self.call(|extension, store| {
            async move {
                let resource = store.data_mut().table().push(worktree)?;
                let options = extension
                    .call_language_server_initialization_options(
                        store,
                        &language_server_id,
                        &language_name,
                        resource,
                    )
                    .await?
                    .map_err(|err| store.data().extension_error(err))?;
                anyhow::Ok(options)
            }
            .boxed()
        })
        .await?
    }

    async fn language_server_workspace_configuration(
        &self,
        language_server_id: LanguageServerName,
        worktree: Arc<dyn WorktreeDelegate>,
    ) -> Result<Option<String>> {
        self.call(|extension, store| {
            async move {
                let resource = store.data_mut().table().push(worktree)?;
                let options = extension
                    .call_language_server_workspace_configuration(
                        store,
                        &language_server_id,
                        resource,
                    )
                    .await?
                    .map_err(|err| store.data().extension_error(err))?;
                anyhow::Ok(options)
            }
            .boxed()
        })
        .await?
    }

    async fn language_server_initialization_options_schema(
        &self,
        language_server_id: LanguageServerName,
        worktree: Arc<dyn WorktreeDelegate>,
    ) -> Result<Option<String>> {
        self.call(|extension, store| {
            async move {
                let resource = store.data_mut().table().push(worktree)?;
                extension
                    .call_language_server_initialization_options_schema(
                        store,
                        &language_server_id,
                        resource,
                    )
                    .await
            }
            .boxed()
        })
        .await?
    }

    async fn language_server_workspace_configuration_schema(
        &self,
        language_server_id: LanguageServerName,
        worktree: Arc<dyn WorktreeDelegate>,
    ) -> Result<Option<String>> {
        self.call(|extension, store| {
            async move {
                let resource = store.data_mut().table().push(worktree)?;
                extension
                    .call_language_server_workspace_configuration_schema(
                        store,
                        &language_server_id,
                        resource,
                    )
                    .await
            }
            .boxed()
        })
        .await?
    }

    async fn language_server_additional_initialization_options(
        &self,
        language_server_id: LanguageServerName,
        target_language_server_id: LanguageServerName,
        worktree: Arc<dyn WorktreeDelegate>,
    ) -> Result<Option<String>> {
        self.call(|extension, store| {
            async move {
                let resource = store.data_mut().table().push(worktree)?;
                let options = extension
                    .call_language_server_additional_initialization_options(
                        store,
                        &language_server_id,
                        &target_language_server_id,
                        resource,
                    )
                    .await?
                    .map_err(|err| store.data().extension_error(err))?;
                anyhow::Ok(options)
            }
            .boxed()
        })
        .await?
    }

    async fn language_server_additional_workspace_configuration(
        &self,
        language_server_id: LanguageServerName,
        target_language_server_id: LanguageServerName,
        worktree: Arc<dyn WorktreeDelegate>,
    ) -> Result<Option<String>> {
        self.call(|extension, store| {
            async move {
                let resource = store.data_mut().table().push(worktree)?;
                let options = extension
                    .call_language_server_additional_workspace_configuration(
                        store,
                        &language_server_id,
                        &target_language_server_id,
                        resource,
                    )
                    .await?
                    .map_err(|err| store.data().extension_error(err))?;
                anyhow::Ok(options)
            }
            .boxed()
        })
        .await?
    }

    async fn labels_for_completions(
        &self,
        language_server_id: LanguageServerName,
        completions: Vec<Completion>,
    ) -> Result<Vec<Option<CodeLabel>>> {
        self.call(|extension, store| {
            async move {
                let labels = extension
                    .call_labels_for_completions(
                        store,
                        &language_server_id,
                        completions.into_iter().map(Into::into).collect(),
                    )
                    .await?
                    .map_err(|err| store.data().extension_error(err))?;

                Ok(labels
                    .into_iter()
                    .map(|label| label.map(Into::into))
                    .collect())
            }
            .boxed()
        })
        .await?
    }

    async fn labels_for_symbols(
        &self,
        language_server_id: LanguageServerName,
        symbols: Vec<Symbol>,
    ) -> Result<Vec<Option<CodeLabel>>> {
        self.call(|extension, store| {
            async move {
                let labels = extension
                    .call_labels_for_symbols(
                        store,
                        &language_server_id,
                        symbols.into_iter().map(Into::into).collect(),
                    )
                    .await?
                    .map_err(|err| store.data().extension_error(err))?;

                Ok(labels
                    .into_iter()
                    .map(|label| label.map(Into::into))
                    .collect())
            }
            .boxed()
        })
        .await?
    }

    async fn complete_slash_command_argument(
        &self,
        command: SlashCommand,
        arguments: Vec<String>,
    ) -> Result<Vec<SlashCommandArgumentCompletion>> {
        self.call(|extension, store| {
            async move {
                let completions = extension
                    .call_complete_slash_command_argument(store, &command.into(), &arguments)
                    .await?
                    .map_err(|err| store.data().extension_error(err))?;

                Ok(completions.into_iter().map(Into::into).collect())
            }
            .boxed()
        })
        .await?
    }

    async fn run_slash_command(
        &self,
        command: SlashCommand,
        arguments: Vec<String>,
        delegate: Option<Arc<dyn WorktreeDelegate>>,
    ) -> Result<SlashCommandOutput> {
        self.call(|extension, store| {
            async move {
                let resource = if let Some(delegate) = delegate {
                    Some(store.data_mut().table().push(delegate)?)
                } else {
                    None
                };

                let output = extension
                    .call_run_slash_command(store, &command.into(), &arguments, resource)
                    .await?
                    .map_err(|err| store.data().extension_error(err))?;

                Ok(output.into())
            }
            .boxed()
        })
        .await?
    }

    async fn context_server_command(
        &self,
        context_server_id: Arc<str>,
        project: Arc<dyn ProjectDelegate>,
    ) -> Result<Command> {
        self.call(|extension, store| {
            async move {
                let project_resource = store.data_mut().table().push(project)?;
                let command = extension
                    .call_context_server_command(store, context_server_id.clone(), project_resource)
                    .await?
                    .map_err(|err| store.data().extension_error(err))?;
                anyhow::Ok(command.into())
            }
            .boxed()
        })
        .await?
    }

    async fn context_server_configuration(
        &self,
        context_server_id: Arc<str>,
        project: Arc<dyn ProjectDelegate>,
    ) -> Result<Option<ContextServerConfiguration>> {
        self.call(|extension, store| {
            async move {
                let project_resource = store.data_mut().table().push(project)?;
                let Some(configuration) = extension
                    .call_context_server_configuration(
                        store,
                        context_server_id.clone(),
                        project_resource,
                    )
                    .await?
                    .map_err(|err| store.data().extension_error(err))?
                else {
                    return Ok(None);
                };

                Ok(Some(configuration))
            }
            .boxed()
        })
        .await?
    }

    async fn suggest_docs_packages(&self, provider: Arc<str>) -> Result<Vec<String>> {
        self.call(|extension, store| {
            async move {
                let packages = extension
                    .call_suggest_docs_packages(store, provider.as_ref())
                    .await?
                    .map_err(|err| store.data().extension_error(err))?;

                Ok(packages)
            }
            .boxed()
        })
        .await?
    }

    async fn index_docs(
        &self,
        provider: Arc<str>,
        package_name: Arc<str>,
        kv_store: Arc<dyn KeyValueStoreDelegate>,
    ) -> Result<()> {
        self.call(|extension, store| {
            async move {
                let kv_store_resource = store.data_mut().table().push(kv_store)?;
                extension
                    .call_index_docs(
                        store,
                        provider.as_ref(),
                        package_name.as_ref(),
                        kv_store_resource,
                    )
                    .await?
                    .map_err(|err| store.data().extension_error(err))?;

                anyhow::Ok(())
            }
            .boxed()
        })
        .await?
    }

    async fn run_command(
        &self,
        command_id: Arc<str>,
        context: CommandContext,
        payload_json: Option<String>,
    ) -> Result<()> {
        self.call(move |extension, store| {
            async move {
                extension
                    .call_run_command(store, command_id, context, payload_json)
                    .await?
                    .map_err(|err| store.data().extension_error(err))?;
                Ok(())
            }
            .boxed()
        })
        .await?
    }

    async fn open_view(&self, contribution_id: Arc<str>, context: MountContext) -> Result<u64> {
        self.call(move |extension, store| {
            async move {
                extension
                    .call_open_view(store, contribution_id, context)
                    .await?
                    .map_err(|err| store.data().extension_error(err))
            }
            .boxed()
        })
        .await?
    }

    async fn render_view(
        &self,
        instance_id: u64,
        context: MountContext,
        reason: RenderReason,
    ) -> Result<RemoteViewTree> {
        self.call(move |extension, store| {
            async move {
                extension
                    .call_render_view(store, instance_id, context, reason)
                    .await?
                    .map_err(|err| store.data().extension_error(err))
            }
            .boxed()
        })
        .await?
    }

    async fn handle_view_event(
        &self,
        instance_id: u64,
        context: MountContext,
        event: RemoteViewEvent,
    ) -> Result<EventOutcome> {
        self.call(move |extension, store| {
            async move {
                extension
                    .call_handle_view_event(store, instance_id, context, event)
                    .await?
                    .map_err(|err| store.data().extension_error(err))
            }
            .boxed()
        })
        .await?
    }

    async fn render_virtual_list_range(
        &self,
        instance_id: u64,
        node_id: String,
        range: VirtualListRange,
        context: MountContext,
    ) -> Result<Vec<RemoteViewNode>> {
        self.call(move |extension, store| {
            async move {
                extension
                    .call_render_virtual_list_range(store, instance_id, node_id, range, context)
                    .await?
                    .map_err(|err| store.data().extension_error(err))
            }
            .boxed()
        })
        .await?
    }

    async fn close_view(&self, instance_id: u64) -> Result<()> {
        self.call(move |extension, store| {
            async move { extension.call_close_view(store, instance_id).await }.boxed()
        })
        .await?
    }

    async fn get_dap_binary(
        &self,
        dap_name: Arc<str>,
        config: DebugTaskDefinition,
        user_installed_path: Option<PathBuf>,
        worktree: Arc<dyn WorktreeDelegate>,
    ) -> Result<DebugAdapterBinary> {
        self.call(|extension, store| {
            async move {
                let resource = store.data_mut().table().push(worktree)?;
                let dap_binary = extension
                    .call_get_dap_binary(store, dap_name, config, user_installed_path, resource)
                    .await?
                    .map_err(|err| store.data().extension_error(err))?;
                Ok(dap_binary)
            }
            .boxed()
        })
        .await?
    }
    async fn dap_request_kind(
        &self,
        dap_name: Arc<str>,
        config: serde_json::Value,
    ) -> Result<StartDebuggingRequestArgumentsRequest> {
        self.call(|extension, store| {
            async move {
                let kind = extension
                    .call_dap_request_kind(store, dap_name, config)
                    .await?
                    .map_err(|err| store.data().extension_error(err))?;
                Ok(kind)
            }
            .boxed()
        })
        .await?
    }

    async fn dap_config_to_scenario(&self, config: ZedDebugConfig) -> Result<DebugScenario> {
        self.call(|extension, store| {
            async move {
                let kind = extension
                    .call_dap_config_to_scenario(store, config)
                    .await?
                    .map_err(|err| store.data().extension_error(err))?;
                Ok(kind)
            }
            .boxed()
        })
        .await?
    }

    async fn dap_locator_create_scenario(
        &self,
        locator_name: String,
        build_config_template: TaskTemplate,
        resolved_label: String,
        debug_adapter_name: String,
    ) -> Result<Option<DebugScenario>> {
        self.call(|extension, store| {
            async move {
                extension
                    .call_dap_locator_create_scenario(
                        store,
                        locator_name,
                        build_config_template,
                        resolved_label,
                        debug_adapter_name,
                    )
                    .await
            }
            .boxed()
        })
        .await?
    }
    async fn run_dap_locator(
        &self,
        locator_name: String,
        config: SpawnInTerminal,
    ) -> Result<DebugRequest> {
        self.call(|extension, store| {
            async move {
                extension
                    .call_run_dap_locator(store, locator_name, config)
                    .await?
                    .map_err(|err| store.data().extension_error(err))
            }
            .boxed()
        })
        .await?
    }
}

pub struct WasmState {
    manifest: Arc<ExtensionManifest>,
    extension_dir: PathBuf,
    pub table: ResourceTable,
    ctx: wasi::WasiCtx,
    pub host: Arc<WasmHost>,
    pub(crate) capability_granter: CapabilityGranter,
    #[allow(dead_code)]
    sidecars: SidecarCollection,
}

type MainThreadCall = Box<dyn Send + for<'a> FnOnce(&'a mut AsyncApp) -> LocalBoxFuture<'a, ()>>;

type ExtensionCall = Box<
    dyn Send + for<'a> FnOnce(&'a mut Extension, &'a mut Store<WasmState>) -> BoxFuture<'a, ()>,
>;

#[allow(dead_code)]
const SIDECAR_JSON_RPC_VERSION: &str = "2.0";
#[allow(dead_code)]
const DEFAULT_SIDECAR_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SidecarId(u64);

#[allow(dead_code)]
#[derive(Default)]
struct SidecarCollection {
    next_id: AtomicU64,
    default_id: Option<SidecarId>,
    sessions: std::collections::HashMap<SidecarId, Arc<SidecarTransport>>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Serialize)]
struct SidecarRpcError {
    code: i64,
    message: String,
    #[serde(default)]
    data: Option<Value>,
}

#[allow(dead_code)]
#[derive(Serialize)]
struct SidecarRpcRequest<'a> {
    jsonrpc: &'static str,
    id: u64,
    method: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<&'a Value>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct SidecarRpcResponse {
    jsonrpc: String,
    id: u64,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<SidecarRpcError>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct SidecarRpcNotification {
    jsonrpc: String,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[allow(dead_code)]
struct SidecarTransport {
    extension_id: Arc<str>,
    outbound_tx: mpsc::UnboundedSender<String>,
    pending_responses:
        Arc<Mutex<std::collections::HashMap<u64, oneshot::Sender<Result<Value, anyhow::Error>>>>>,
    next_request_id: AtomicU64,
    background_executor: BackgroundExecutor,
    _child: util::process::Child,
    _read_task: Task<()>,
    _write_task: Task<()>,
    _stderr_task: Task<()>,
}

impl Drop for SidecarTransport {
    fn drop(&mut self) {
        if let Err(error) = self._child.kill() {
            log::debug!(
                "failed to kill extension sidecar for {}: {error}",
                self.extension_id
            );
        }
    }
}

#[allow(dead_code)]
impl SidecarTransport {
    fn spawn(
        command: &Command,
        work_dir: &Path,
        extension_id: Arc<str>,
        background_executor: BackgroundExecutor,
    ) -> Result<Self> {
        let mut sidecar = util::command::new_std_command(command.command.as_os_str());
        sidecar.args(&command.args);
        sidecar.envs(command.env.iter().cloned());
        sidecar.current_dir(work_dir);

        let mut child = util::process::Child::spawn(
            sidecar,
            std::process::Stdio::piped(),
            std::process::Stdio::piped(),
            std::process::Stdio::piped(),
        )
        .with_context(|| {
            format!(
                "failed to spawn sidecar for extension {}: {} {:?}",
                extension_id,
                command.command.display(),
                command.args
            )
        })?;

        let stdin = child.stdin.take().context("sidecar stdin capture failed")?;
        let stdout = child
            .stdout
            .take()
            .context("sidecar stdout capture failed")?;
        let stderr = child
            .stderr
            .take()
            .context("sidecar stderr capture failed")?;

        let (outbound_tx, outbound_rx) = mpsc::unbounded::<String>();
        let pending_responses = Arc::new(Mutex::new(std::collections::HashMap::default()));

        let read_task = background_executor.spawn({
            let pending_responses = pending_responses.clone();
            let extension_id = extension_id.clone();
            async move {
                if let Err(error) =
                    Self::read_stdout(stdout, pending_responses.clone(), extension_id.clone()).await
                {
                    log::error!(
                        "extension sidecar stdout failed for {}: {error}",
                        extension_id
                    );
                }
                Self::fail_pending_requests(
                    &pending_responses,
                    anyhow!("sidecar stdout closed for extension {}", extension_id),
                );
            }
        });
        let write_task = background_executor.spawn({
            let extension_id = extension_id.clone();
            async move {
                if let Err(error) = Self::write_stdin(stdin, outbound_rx).await {
                    log::error!(
                        "extension sidecar stdin failed for {}: {error}",
                        extension_id
                    );
                }
            }
        });
        let stderr_task = background_executor.spawn({
            let extension_id = extension_id.clone();
            async move {
                if let Err(error) = Self::read_stderr(stderr, extension_id.clone()).await {
                    log::error!(
                        "extension sidecar stderr failed for {}: {error}",
                        extension_id
                    );
                }
            }
        });

        Ok(Self {
            extension_id,
            outbound_tx,
            pending_responses,
            next_request_id: AtomicU64::new(0),
            background_executor,
            _child: child,
            _read_task: read_task,
            _write_task: write_task,
            _stderr_task: stderr_task,
        })
    }

    async fn request(
        &self,
        method: &str,
        params: Option<Value>,
        timeout: Option<Duration>,
    ) -> Result<Value> {
        let request_id = self.next_request_id.fetch_add(1, SeqCst);
        let message = serde_json::to_string(&SidecarRpcRequest {
            jsonrpc: SIDECAR_JSON_RPC_VERSION,
            id: request_id,
            method,
            params: params.as_ref(),
        })
        .context("serializing sidecar request")?;

        let (response_tx, response_rx) = oneshot::channel();
        {
            let mut pending_responses = self
                .pending_responses
                .lock()
                .map_err(|_| anyhow!("sidecar pending response lock poisoned"))?;
            pending_responses.insert(request_id, response_tx);
        }

        if self.outbound_tx.unbounded_send(message).is_err() {
            self.remove_pending_response(request_id);
            anyhow::bail!(
                "sidecar stdin is closed for extension {}",
                self.extension_id
            );
        }

        let timeout = timeout.unwrap_or(DEFAULT_SIDECAR_REQUEST_TIMEOUT);
        let mut timer = self.background_executor.timer(timeout).fuse();
        let mut response_rx = response_rx.fuse();
        futures::select_biased! {
            response = response_rx => {
                match response {
                    Ok(result) => result,
                    Err(_) => {
                        anyhow::bail!(
                            "sidecar response channel closed for extension {} request {request_id}",
                            self.extension_id
                        );
                    }
                }
            }
            _ = timer => {
                self.remove_pending_response(request_id);
                anyhow::bail!(
                    "sidecar request timed out for extension {} after {:?}",
                    self.extension_id,
                    timeout
                );
            }
        }
    }

    fn remove_pending_response(&self, request_id: u64) {
        if let Ok(mut pending_responses) = self.pending_responses.lock() {
            pending_responses.remove(&request_id);
        }
    }

    fn fail_pending_requests(
        pending_responses: &Arc<
            Mutex<std::collections::HashMap<u64, oneshot::Sender<Result<Value, anyhow::Error>>>>,
        >,
        error: anyhow::Error,
    ) {
        let mut pending_responses = match pending_responses.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        for (_, response_tx) in pending_responses.drain() {
            let _ = response_tx.send(Err(anyhow!(error.to_string())));
        }
    }

    async fn read_stdout(
        stdout: impl futures::AsyncRead + Unpin,
        pending_responses: Arc<
            Mutex<std::collections::HashMap<u64, oneshot::Sender<Result<Value, anyhow::Error>>>>,
        >,
        extension_id: Arc<str>,
    ) -> Result<()> {
        let mut stdout = BufReader::new(stdout);
        let mut line = String::new();

        loop {
            line.clear();
            let bytes_read = stdout.read_line(&mut line).await?;
            if bytes_read == 0 {
                break;
            }

            let message = line.trim();
            if message.is_empty() {
                continue;
            }

            if let Ok(response) = serde_json::from_str::<SidecarRpcResponse>(message) {
                let mut pending_responses = pending_responses
                    .lock()
                    .map_err(|_| anyhow!("sidecar pending response lock poisoned"))?;
                if let Some(response_tx) = pending_responses.remove(&response.id) {
                    let result = match (response.result, response.error) {
                        (Some(result), None) => Ok(result),
                        (None, Some(error)) => Err(anyhow!(
                            "sidecar JSON-RPC error {} (code {}): {}",
                            extension_id,
                            error.code,
                            error.message
                        )),
                        (Some(_), Some(error)) => Err(anyhow!(
                            "sidecar JSON-RPC response contained both result and error for extension {}: {}",
                            extension_id,
                            error.message
                        )),
                        (None, None) => Err(anyhow!(
                            "sidecar JSON-RPC response missing result and error for extension {}",
                            extension_id
                        )),
                    };
                    let _ = response_tx.send(result);
                } else {
                    log::warn!(
                        "dropping unmatched sidecar response {} for extension {}",
                        response.id,
                        extension_id
                    );
                }
                continue;
            }

            if let Ok(notification) = serde_json::from_str::<SidecarRpcNotification>(message) {
                log::debug!(
                    "extension sidecar notification for {}: {} {:?}",
                    extension_id,
                    notification.method,
                    notification.params
                );
                continue;
            }

            log::warn!(
                "failed to parse sidecar stdout JSON for extension {}: {}",
                extension_id,
                message
            );
        }

        Ok(())
    }

    async fn write_stdin(
        stdin: impl futures::AsyncWrite + Unpin,
        mut outbound_rx: mpsc::UnboundedReceiver<String>,
    ) -> Result<()> {
        let mut stdin = BufWriter::new(stdin);
        while let Some(message) = outbound_rx.next().await {
            stdin.write_all(message.as_bytes()).await?;
            stdin.write_all(b"\n").await?;
            stdin.flush().await?;
        }
        Ok(())
    }

    async fn read_stderr(
        stderr: impl futures::AsyncRead + Unpin,
        extension_id: Arc<str>,
    ) -> Result<()> {
        let mut stderr = BufReader::new(stderr);
        let mut line = String::new();
        loop {
            line.clear();
            let bytes_read = stderr.read_line(&mut line).await?;
            if bytes_read == 0 {
                break;
            }
            log::debug!(
                "extension sidecar stderr {}: {}",
                extension_id,
                line.trim_end()
            );
        }
        Ok(())
    }
}

fn wasm_engine(executor: &BackgroundExecutor) -> wasmtime::Engine {
    static WASM_ENGINE: OnceLock<wasmtime::Engine> = OnceLock::new();
    WASM_ENGINE
        .get_or_init(|| {
            let mut config = wasmtime::Config::new();
            config.wasm_component_model(true);
            config.async_support(true);
            config
                .enable_incremental_compilation(cache_store())
                .unwrap();
            // Async support introduces the issue that extension execution happens during `Future::poll`,
            // which could block an async thread.
            // https://docs.rs/wasmtime/latest/wasmtime/struct.Config.html#execution-in-poll
            //
            // Epoch interruption is a lightweight mechanism to allow the extensions to yield control
            // back to the executor at regular intervals.
            config.epoch_interruption(true);

            let engine = wasmtime::Engine::new(&config).unwrap();

            // It might be safer to do this on a non-async thread to make sure it makes progress
            // regardless of if extensions are blocking.
            // However, due to our current setup, this isn't a likely occurrence and we'd rather
            // not have a dedicated thread just for this. If it becomes an issue, we can consider
            // creating a separate thread for epoch interruption.
            let engine_ref = engine.weak();
            let executor2 = executor.clone();
            executor
                .spawn(async move {
                    // Somewhat arbitrary interval, as it isn't a guaranteed interval.
                    // But this is a rough upper bound for how long the extension execution can block on
                    // `Future::poll`.
                    const EPOCH_INTERVAL: Duration = Duration::from_millis(100);
                    loop {
                        executor2.timer(EPOCH_INTERVAL).await;
                        // Exit the loop and thread once the engine is dropped.
                        let Some(engine) = engine_ref.upgrade() else {
                            break;
                        };
                        engine.increment_epoch();
                    }
                })
                .detach();

            engine
        })
        .clone()
}

fn cache_store() -> Arc<IncrementalCompilationCache> {
    static CACHE_STORE: LazyLock<Arc<IncrementalCompilationCache>> =
        LazyLock::new(|| Arc::new(IncrementalCompilationCache::new()));
    CACHE_STORE.clone()
}

impl WasmHost {
    pub fn new(
        fs: Arc<dyn Fs>,
        http_client: Arc<dyn HttpClient>,
        node_runtime: NodeRuntime,
        proxy: Arc<ExtensionHostProxy>,
        work_dir: PathBuf,
        cx: &mut App,
    ) -> Arc<Self> {
        let (tx, mut rx) = mpsc::unbounded::<MainThreadCall>();
        let task = cx.spawn(async move |cx| {
            while let Some(message) = rx.next().await {
                message(cx).await;
            }
        });

        let extension_settings = ExtensionSettings::get_global(cx);

        Arc::new(Self {
            engine: wasm_engine(cx.background_executor()),
            fs,
            work_dir,
            http_client,
            node_runtime,
            background_executor: cx.background_executor().clone(),
            proxy,
            release_channel: ReleaseChannel::global(cx),
            granted_capabilities: extension_settings.granted_capabilities.clone(),
            _main_thread_message_task: task,
            main_thread_message_tx: tx,
        })
    }

    pub fn load_extension(
        self: &Arc<Self>,
        extension_dir: PathBuf,
        wasm_bytes: Vec<u8>,
        manifest: &Arc<ExtensionManifest>,
        cx: &AsyncApp,
    ) -> Task<Result<WasmExtension>> {
        let this = self.clone();
        let manifest = manifest.clone();
        let executor = cx.background_executor().clone();

        // Parse version and compile component on gpui's background executor.
        // These are cpu-bound operations that don't require a tokio runtime.
        let compile_task = {
            let manifest_id = manifest.id.clone();
            let engine = this.engine.clone();

            executor.spawn(async move {
                let zed_api_version = parse_wasm_extension_version(&manifest_id, &wasm_bytes)?;
                let component = Component::from_binary(&engine, &wasm_bytes)
                    .context("failed to compile wasm component")?;

                anyhow::Ok((zed_api_version, component))
            })
        };

        let load_extension = |zed_api_version: Version, component| async move {
            let wasi_ctx = this.build_wasi_ctx(&manifest).await?;
            let mut store = wasmtime::Store::new(
                &this.engine,
                WasmState {
                    ctx: wasi_ctx,
                    manifest: manifest.clone(),
                    extension_dir: extension_dir.clone(),
                    table: ResourceTable::new(),
                    host: this.clone(),
                    capability_granter: CapabilityGranter::new(
                        this.granted_capabilities.clone(),
                        manifest.clone(),
                    ),
                    sidecars: SidecarCollection::default(),
                },
            );
            // Store will yield after 1 tick, and get a new deadline of 1 tick after each yield.
            store.set_epoch_deadline(1);
            store.epoch_deadline_async_yield_and_update(1);

            let mut extension = Extension::instantiate_async(
                &executor,
                &mut store,
                this.release_channel,
                zed_api_version.clone(),
                &component,
            )
            .await?;

            extension
                .call_init_extension(&mut store)
                .await
                .context("failed to initialize wasm extension")?;

            let (tx, mut rx) = mpsc::unbounded::<ExtensionCall>();
            let extension_task = async move {
                while let Some(call) = rx.next().await {
                    (call)(&mut extension, &mut store).await;
                }
            };

            anyhow::Ok((
                extension_task,
                manifest.clone(),
                this.work_dir.join(manifest.id.as_ref()).into(),
                tx,
                zed_api_version,
            ))
        };

        cx.spawn(async move |cx| {
            let (zed_api_version, component) = compile_task.await?;

            // Run wasi-dependent operations on tokio.
            // wasmtime_wasi internally uses tokio for I/O operations.
            let (extension_task, manifest, work_dir, tx, zed_api_version) =
                gpui_tokio::Tokio::spawn(cx, load_extension(zed_api_version, component)).await??;

            // Run the extension message loop on tokio since extension
            // calls may invoke wasi functions that require a tokio runtime.
            let task = Arc::new(gpui_tokio::Tokio::spawn(cx, extension_task));

            Ok(WasmExtension {
                manifest,
                work_dir,
                tx,
                zed_api_version,
                _task: task,
            })
        })
    }

    async fn build_wasi_ctx(&self, manifest: &Arc<ExtensionManifest>) -> Result<wasi::WasiCtx> {
        let extension_work_dir = self.work_dir.join(manifest.id.as_ref());
        self.fs
            .create_dir(&extension_work_dir)
            .await
            .context("failed to create extension work dir")?;

        let file_perms = wasmtime_wasi::FilePerms::all();
        let dir_perms = wasmtime_wasi::DirPerms::all();
        let path = SanitizedPath::new(&extension_work_dir).to_string();
        #[cfg(target_os = "windows")]
        let path = path.replace('\\', "/");

        let mut ctx = wasi::WasiCtxBuilder::new();
        ctx.inherit_stdio()
            .env("PWD", &path)
            .env("RUST_BACKTRACE", "full");

        ctx.preopened_dir(&path, ".", dir_perms, file_perms)?;
        ctx.preopened_dir(&path, &path, dir_perms, file_perms)?;

        Ok(ctx.build())
    }

    pub async fn writeable_path_from_extension(
        &self,
        id: &Arc<str>,
        path: &Path,
    ) -> Result<PathBuf> {
        let canonical_work_dir = self
            .fs
            .canonicalize(&self.work_dir)
            .await
            .with_context(|| format!("canonicalizing work dir {:?}", self.work_dir))?;
        let extension_work_dir = canonical_work_dir.join(id.as_ref());

        let absolute = if path.is_relative() {
            extension_work_dir.join(path)
        } else {
            path.to_path_buf()
        };

        let normalized = util::paths::normalize_lexically(&absolute)
            .map_err(|_| anyhow!("path {path:?} escapes its parent"))?;

        // Canonicalize the nearest existing ancestor to resolve any symlinks
        // in the on-disk portion of the path. Components beyond that ancestor
        // are re-appended, which lets this work for destinations that don't
        // exist yet (e.g. nested directories created by tar extraction).
        let mut existing = normalized.as_path();
        let mut tail_components = Vec::new();
        let canonical_prefix = loop {
            match self.fs.canonicalize(existing).await {
                Ok(canonical) => break canonical,
                Err(_) => {
                    if let Some(file_name) = existing.file_name() {
                        tail_components.push(file_name.to_owned());
                    }
                    existing = existing
                        .parent()
                        .context(format!("cannot resolve path {path:?}"))?;
                }
            }
        };

        let mut resolved = canonical_prefix;
        for component in tail_components.into_iter().rev() {
            resolved.push(component);
        }

        anyhow::ensure!(
            resolved.starts_with(&extension_work_dir),
            "cannot write to path {resolved:?}",
        );
        Ok(resolved)
    }
}

pub fn parse_wasm_extension_version(extension_id: &str, wasm_bytes: &[u8]) -> Result<Version> {
    let mut version = None;

    for part in wasmparser::Parser::new(0).parse_all(wasm_bytes) {
        if let wasmparser::Payload::CustomSection(s) =
            part.context("error parsing wasm extension")?
            && s.name() == "zed:api-version"
        {
            version = parse_wasm_extension_version_custom_section(s.data());
            if version.is_none() {
                bail!(
                    "extension {} has invalid zed:api-version section: {:?}",
                    extension_id,
                    s.data()
                );
            }
        }
    }

    // The reason we wait until we're done parsing all of the Wasm bytes to return the version
    // is to work around a panic that can happen inside of Wasmtime when the bytes are invalid.
    //
    // By parsing the entirety of the Wasm bytes before we return, we're able to detect this problem
    // earlier as an `Err` rather than as a panic.
    version.with_context(|| format!("extension {extension_id} has no zed:api-version section"))
}

fn parse_wasm_extension_version_custom_section(data: &[u8]) -> Option<Version> {
    if data.len() == 6 {
        Some(Version::new(
            u16::from_be_bytes([data[0], data[1]]) as _,
            u16::from_be_bytes([data[2], data[3]]) as _,
            u16::from_be_bytes([data[4], data[5]]) as _,
        ))
    } else {
        None
    }
}

impl WasmExtension {
    pub async fn load(
        extension_dir: &Path,
        manifest: &Arc<ExtensionManifest>,
        wasm_host: Arc<WasmHost>,
        cx: &AsyncApp,
    ) -> Result<Self> {
        let path = extension_dir.join("extension.wasm");

        let mut wasm_file = wasm_host
            .fs
            .open_sync(&path)
            .await
            .context(format!("opening wasm file, path: {path:?}"))?;

        let mut wasm_bytes = Vec::new();
        wasm_file
            .read_to_end(&mut wasm_bytes)
            .context(format!("reading wasm file, path: {path:?}"))?;

        wasm_host
            .load_extension(extension_dir.to_path_buf(), wasm_bytes, manifest, cx)
            .await
            .with_context(|| format!("loading wasm extension: {}", manifest.id))
    }

    pub async fn call<T, Fn>(&self, f: Fn) -> Result<T>
    where
        T: 'static + Send,
        Fn: 'static
            + Send
            + for<'a> FnOnce(&'a mut Extension, &'a mut Store<WasmState>) -> BoxFuture<'a, T>,
    {
        let (return_tx, return_rx) = oneshot::channel();
        self.tx
            .unbounded_send(Box::new(move |extension, store| {
                async {
                    let result = f(extension, store).await;
                    return_tx.send(result).ok();
                }
                .boxed()
            }))
            .map_err(|_| {
                anyhow!(
                    "wasm extension channel should not be closed yet, extension {} (id {})",
                    self.manifest.name,
                    self.manifest.id,
                )
            })?;
        return_rx.await.with_context(|| {
            format!(
                "wasm extension channel, extension {} (id {})",
                self.manifest.name, self.manifest.id,
            )
        })
    }
}

impl WasmState {
    fn on_main_thread<T, Fn>(&self, f: Fn) -> impl 'static + Future<Output = T>
    where
        T: 'static + Send,
        Fn: 'static + Send + for<'a> FnOnce(&'a mut AsyncApp) -> LocalBoxFuture<'a, T>,
    {
        let (return_tx, return_rx) = oneshot::channel();
        self.host
            .main_thread_message_tx
            .clone()
            .unbounded_send(Box::new(move |cx| {
                async {
                    let result = f(cx).await;
                    return_tx.send(result).ok();
                }
                .boxed_local()
            }))
            .unwrap_or_else(|_| {
                panic!(
                    "main thread message channel should not be closed yet, extension {} (id {})",
                    self.manifest.name, self.manifest.id,
                )
            });
        let name = self.manifest.name.clone();
        let id = self.manifest.id.clone();
        async move {
            return_rx.await.unwrap_or_else(|_| {
                panic!("main thread message channel, extension {name} (id {id})")
            })
        }
    }

    fn work_dir(&self) -> PathBuf {
        self.host.work_dir.join(self.manifest.id.as_ref())
    }

    fn resolve_sidecar_path(&self, value: &str) -> Option<PathBuf> {
        let path = PathBuf::from(value);
        if path.as_os_str().is_empty() || path.is_absolute() {
            return None;
        }

        let candidate = self.extension_dir.join(&path);
        candidate.exists().then_some(candidate)
    }

    fn resolved_sidecar_command(&self) -> Command {
        let sidecar = self
            .manifest
            .sidecar
            .as_ref()
            .expect("resolved_sidecar_command called without a sidecar manifest");
        let command = self
            .resolve_sidecar_path(&sidecar.command)
            .unwrap_or_else(|| sidecar.command.clone().into());
        let args = sidecar
            .args
            .iter()
            .map(|arg| {
                self.resolve_sidecar_path(arg)
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_else(|| arg.clone())
            })
            .collect();

        Command {
            command,
            args,
            env: sidecar.env.clone().into_iter().collect(),
        }
    }

    fn extension_error(&self, message: String) -> anyhow::Error {
        anyhow!(
            "from extension \"{}\" version {}: {}",
            self.manifest.name,
            self.manifest.version,
            message
        )
    }

    #[allow(dead_code)]
    pub(crate) fn ensure_sidecar(&mut self) -> Result<SidecarId> {
        if let Some(sidecar_id) = self.sidecars.default_id {
            return Ok(sidecar_id);
        }

        let sidecar = self
            .manifest
            .sidecar
            .as_ref()
            .context("extension does not declare a sidecar")?;
        self.capability_granter
            .grant_exec(&sidecar.command, &sidecar.args)?;

        let command = self.resolved_sidecar_command();
        let sidecar_id = SidecarId(self.sidecars.next_id.fetch_add(1, SeqCst));
        let transport = Arc::new(SidecarTransport::spawn(
            &command,
            &self.work_dir(),
            self.manifest.id.clone(),
            self.host.background_executor.clone(),
        )?);
        self.sidecars.default_id = Some(sidecar_id);
        self.sidecars.sessions.insert(sidecar_id, transport);
        Ok(sidecar_id)
    }

    #[allow(dead_code)]
    pub(crate) async fn request_sidecar(
        &mut self,
        method: &str,
        params: Option<Value>,
        timeout: Option<Duration>,
    ) -> Result<Value> {
        let sidecar_id = self.ensure_sidecar()?;
        let sidecar = self
            .sidecars
            .sessions
            .get(&sidecar_id)
            .cloned()
            .with_context(|| format!("unknown sidecar session {:?}", sidecar_id))?;
        sidecar.request(method, params, timeout).await
    }

    #[allow(dead_code)]
    pub(crate) fn close_sidecar(&mut self) -> Result<()> {
        let sidecar_id = self
            .sidecars
            .default_id
            .take()
            .context("extension sidecar is not running")?;
        self.sidecars
            .sessions
            .remove(&sidecar_id)
            .with_context(|| format!("unknown sidecar session {:?}", sidecar_id))?;
        Ok(())
    }
}

impl wasi::IoView for WasmState {
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

impl wasi::WasiView for WasmState {
    fn ctx(&mut self) -> &mut wasi::WasiCtx {
        &mut self.ctx
    }
}

/// Wrapper around a mini-moka bounded cache for storing incremental compilation artifacts.
/// Since wasm modules have many similar elements, this can save us a lot of work at the
/// cost of a small memory footprint. However, we don't want this to be unbounded, so we use
/// a LFU/LRU cache to evict less used cache entries.
#[derive(Debug)]
struct IncrementalCompilationCache {
    cache: Cache<Vec<u8>, Vec<u8>>,
}

impl IncrementalCompilationCache {
    fn new() -> Self {
        let cache = Cache::builder()
            // Cap this at 32 MB for now. Our extensions turn into roughly 512kb in the cache,
            // which means we could store 64 completely novel extensions in the cache, but in
            // practice we will more than that, which is more than enough for our use case.
            .max_capacity(32 * 1024 * 1024)
            .weigher(|k: &Vec<u8>, v: &Vec<u8>| (k.len() + v.len()).try_into().unwrap_or(u32::MAX))
            .build();
        Self { cache }
    }
}

impl CacheStore for IncrementalCompilationCache {
    fn get(&self, key: &[u8]) -> Option<Cow<'_, [u8]>> {
        self.cache.get(key).map(|v| v.into())
    }

    fn insert(&self, key: &[u8], value: Vec<u8>) -> bool {
        self.cache.insert(key.to_vec(), value);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use collections::BTreeMap;
    use extension::{
        ExtensionHostProxy, ExtensionSidecarManifestEntry, ProcessExecCapability, SchemaVersion,
    };
    use fs::{FakeFs, RealFs};
    use gpui::TestAppContext;
    use http_client::FakeHttpClient;
    use node_runtime::NodeRuntime;
    use serde_json::json;
    use settings::SettingsStore;
    use std::collections::HashMap;

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let store = SettingsStore::test(cx);
            cx.set_global(store);
            release_channel::init(semver::Version::new(0, 0, 0), cx);
            extension::init(cx);
            gpui_tokio::init(cx);
        });
    }

    fn test_manifest(sidecar_args: Vec<String>) -> Arc<ExtensionManifest> {
        Arc::new(ExtensionManifest {
            id: "test-extension".into(),
            name: "Test Extension".into(),
            version: "0.1.0".into(),
            schema_version: SchemaVersion::ZERO,
            description: None,
            repository: None,
            authors: Vec::new(),
            lib: Default::default(),
            themes: Vec::new(),
            icon_themes: Vec::new(),
            languages: Vec::new(),
            grammars: BTreeMap::default(),
            language_servers: BTreeMap::default(),
            context_servers: BTreeMap::default(),
            agent_servers: BTreeMap::default(),
            slash_commands: BTreeMap::default(),
            snippets: None,
            sidecar: Some(ExtensionSidecarManifestEntry {
                command: "/bin/sh".into(),
                args: sidecar_args,
                env: HashMap::default(),
            }),
            capabilities: vec![ExtensionCapability::ProcessExec(ProcessExecCapability {
                command: "/bin/sh".into(),
                args: vec!["**".into()],
            })],
            debug_adapters: BTreeMap::default(),
            debug_locators: BTreeMap::default(),
            language_model_providers: BTreeMap::default(),
        })
    }

    fn test_state(host: Arc<WasmHost>, manifest: Arc<ExtensionManifest>) -> WasmState {
        WasmState {
            manifest: manifest.clone(),
            extension_dir: host.work_dir.join(manifest.id.as_ref()),
            table: ResourceTable::new(),
            ctx: wasi::WasiCtxBuilder::new().build(),
            host,
            capability_granter: CapabilityGranter::new(
                vec![ExtensionCapability::ProcessExec(ProcessExecCapability {
                    command: "/bin/sh".into(),
                    args: vec!["**".into()],
                })],
                manifest,
            ),
            sidecars: SidecarCollection::default(),
        }
    }

    #[gpui::test]
    async fn test_writeable_path_rejects_escape_attempts(cx: &mut TestAppContext) {
        init_test(cx);

        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(
            "/work",
            json!({
                "test-extension": {
                    "legit.txt": "legitimate content"
                }
            }),
        )
        .await;
        fs.insert_tree("/outside", json!({ "secret.txt": "sensitive data" }))
            .await;
        fs.insert_symlink("/work/test-extension/escape", PathBuf::from("/outside"))
            .await;

        let host = cx.update(|cx| {
            WasmHost::new(
                fs.clone(),
                FakeHttpClient::with_200_response(),
                NodeRuntime::unavailable(),
                Arc::new(ExtensionHostProxy::default()),
                PathBuf::from("/work"),
                cx,
            )
        });

        let extension_id: Arc<str> = "test-extension".into();

        // A path traversing through a symlink that points outside the work dir
        // must be rejected. Canonicalization resolves the symlink before the
        // prefix check, so this is caught.
        let result = host
            .writeable_path_from_extension(
                &extension_id,
                Path::new("/work/test-extension/escape/secret.txt"),
            )
            .await;
        assert!(
            result.is_err(),
            "symlink escape should be rejected, but got: {result:?}",
        );

        // A path using `..` to escape the extension work dir must be rejected.
        let result = host
            .writeable_path_from_extension(
                &extension_id,
                Path::new("/work/test-extension/../../outside/secret.txt"),
            )
            .await;
        assert!(
            result.is_err(),
            "parent traversal escape should be rejected, but got: {result:?}",
        );

        // A legitimate path within the extension work dir should succeed.
        let result = host
            .writeable_path_from_extension(
                &extension_id,
                Path::new("/work/test-extension/legit.txt"),
            )
            .await;
        assert!(
            result.is_ok(),
            "legitimate path should be accepted, but got: {result:?}",
        );

        // A relative path with non-existent intermediate directories should
        // succeed, mirroring the integration test pattern where an extension
        // downloads a tar to e.g. "gleam-v1.2.3" (creating the directory)
        // and then references "gleam-v1.2.3/gleam" inside it.
        let result = host
            .writeable_path_from_extension(&extension_id, Path::new("new-dir/nested/binary"))
            .await;
        assert!(
            result.is_ok(),
            "relative path with non-existent parents should be accepted, but got: {result:?}",
        );

        // A symlink deeper than the immediate parent must still be caught.
        // Here "escape" is a symlink to /outside, so "escape/deep/file.txt"
        // has multiple non-existent components beyond the symlink.
        let result = host
            .writeable_path_from_extension(&extension_id, Path::new("escape/deep/nested/file.txt"))
            .await;
        assert!(
            result.is_err(),
            "symlink escape through deep non-existent path should be rejected, but got: {result:?}",
        );
    }

    #[cfg(not(windows))]
    #[gpui::test]
    #[ignore = "uses a real sidecar subprocess over async-io"]
    async fn test_sidecar_request_round_trip(cx: &mut TestAppContext) {
        init_test(cx);

        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let work_dir = temp_dir.path().join("work");
        std::fs::create_dir_all(work_dir.join("test-extension"))
            .expect("sidecar work dir should be created");
        let fs = Arc::new(RealFs::new(None, cx.executor()));

        let host = cx.update(|cx| {
            WasmHost::new(
                fs.clone(),
                FakeHttpClient::with_200_response(),
                NodeRuntime::unavailable(),
                Arc::new(ExtensionHostProxy::default()),
                work_dir.clone(),
                cx,
            )
        });

        let manifest = test_manifest(vec![
            "-c".into(),
            r#"IFS= read -r line
case "$line" in
  *'"method":"ping"'*) printf '%s\n' '{"jsonrpc":"2.0","id":0,"result":{"ok":true}}' ;;
  *) printf '%s\n' '{"jsonrpc":"2.0","id":0,"error":{"code":-32601,"message":"unknown method"}}' ;;
esac"#
                .into(),
        ]);
        let mut state = test_state(host, manifest);

        let response = state
            .request_sidecar("ping", Some(json!({ "value": 1 })), None)
            .await
            .expect("sidecar request should succeed");
        assert_eq!(response, json!({ "ok": true }));

        state.close_sidecar().expect("sidecar should close cleanly");
    }

    #[cfg(not(windows))]
    #[gpui::test]
    #[ignore = "uses a real sidecar subprocess over async-io"]
    async fn test_sidecar_request_surfaces_rpc_error(cx: &mut TestAppContext) {
        init_test(cx);

        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let work_dir = temp_dir.path().join("work");
        std::fs::create_dir_all(work_dir.join("test-extension"))
            .expect("sidecar work dir should be created");
        let fs = Arc::new(RealFs::new(None, cx.executor()));

        let host = cx.update(|cx| {
            WasmHost::new(
                fs.clone(),
                FakeHttpClient::with_200_response(),
                NodeRuntime::unavailable(),
                Arc::new(ExtensionHostProxy::default()),
                work_dir.clone(),
                cx,
            )
        });

        let manifest = test_manifest(vec![
            "-c".into(),
            r#"IFS= read -r line
printf '%s\n' '{"jsonrpc":"2.0","id":0,"error":{"code":123,"message":"nope"}}'"#
                .into(),
        ]);
        let mut state = test_state(host, manifest);

        let error = state
            .request_sidecar("ping", None, None)
            .await
            .expect_err("sidecar request should return an RPC error");
        assert!(
            error.to_string().contains("nope"),
            "expected RPC error message, got {error:?}"
        );
    }
}
