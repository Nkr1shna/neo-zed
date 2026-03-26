use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use urlencoding::encode;

use zed_extension_api::{
    self as zed,
    http_client::{HttpMethod, HttpRequest},
    CommandContext, EventOutcome, HostMutation, MountContext, ProgressBarProps,
    RemoteViewEvent, RemoteViewEventKind, RemoteViewNode, RemoteViewNodeKind, RemoteViewProperty,
    RemoteViewTree, RenderReason,
};

const AUTH_STATE_PATH: &str = "codex-chatgpt-auth.json";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const DEFAULT_TOKEN_EXPIRY_SECONDS: u64 = 3_600;
const TOKEN_REFRESH_SKEW_MILLIS: u64 = 5 * 60 * 1_000;

struct RemoteUiExampleExtension {
    next_instance_id: u64,
    views: HashMap<u64, DemoViewState>,
}

struct DemoViewState {
    contribution_id: String,
    revision: u64,
    snapshot: SidecarSnapshot,
    sidecar_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct SidecarSnapshot {
    auth_status: String,
    plan_type: Option<String>,
    primary_used_percent: u32,
    usage_limits: Vec<UsageLimitCard>,
    credits_summary: CreditsSummary,
}

#[derive(Clone, Debug, PartialEq)]
struct UsageLimitCard {
    id: String,
    title: String,
    remaining_percent: u32,
    resets_at_label: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct CreditsSummary {
    balance_label: String,
    detail: String,
}

#[derive(Clone, Debug, PartialEq)]
struct PersistedAuthState {
    auth_status: String,
    access_token: Option<String>,
    refresh_token: Option<String>,
    account_id: Option<String>,
    expires_at: u64,
    last_refresh_at: u64,
    last_snapshot: Option<SidecarSnapshot>,
    last_fetched_at: u64,
    last_error: Option<String>,
}

impl SidecarSnapshot {
    fn signed_out() -> Self {
        Self {
            auth_status: "signed-out".to_string(),
            plan_type: None,
            primary_used_percent: 0,
            usage_limits: Vec::new(),
            credits_summary: CreditsSummary {
                balance_label: "0".to_string(),
                detail: "Use credits to send messages beyond your plan limit.".to_string(),
            },
        }
    }

    fn status_badge(&self) -> String {
        format!("{}%", self.primary_used_percent)
    }

    fn footer_tooltip(&self, sidecar_error: Option<&str>) -> &'static str {
        if sidecar_error.is_some() {
            "Retry ChatGPT Codex usage refresh"
        } else if self.is_pending() {
            "Waiting for ChatGPT sign-in to complete"
        } else if self.is_authenticated() {
            "Open the Codex usage panel"
        } else {
            "Sign in to view Codex usage"
        }
    }

    fn is_authenticated(&self) -> bool {
        self.auth_status == "authenticated"
    }

    fn is_pending(&self) -> bool {
        self.auth_status == "pending"
    }

    fn primary_remaining_percent(&self) -> u32 {
        self.usage_limits
            .first()
            .map(|usage_limit| usage_limit.remaining_percent)
            .unwrap_or_else(|| 100_u32.saturating_sub(self.primary_used_percent.min(100)))
    }

    fn footer_summary(&self) -> String {
        format!("{}% left", self.primary_remaining_percent())
    }

    fn panel_plan_label(&self) -> Option<String> {
        let plan_type = self.plan_type.as_deref()?;
        let normalized_plan_type = plan_type.replace(['-', '_'], " ");
        let formatted_plan_type = normalized_plan_type
            .split_whitespace()
            .map(capitalize_plan_word)
            .collect::<Vec<_>>()
            .join(" ");

        let label = if formatted_plan_type.to_ascii_lowercase().contains("codex") {
            formatted_plan_type
        } else {
            format!("Codex {formatted_plan_type}")
        };

        Some(format!("Plan: {label}"))
    }
}

impl PersistedAuthState {
    fn signed_out() -> Self {
        Self {
            auth_status: "signed-out".to_string(),
            access_token: None,
            refresh_token: None,
            account_id: None,
            expires_at: 0,
            last_refresh_at: 0,
            last_snapshot: None,
            last_fetched_at: 0,
            last_error: None,
        }
    }
}

impl zed::Extension for RemoteUiExampleExtension {
    fn new() -> Self {
        Self {
            next_instance_id: 1,
            views: HashMap::new(),
        }
    }

    fn run_command(
        &mut self,
        command_id: String,
        context: CommandContext,
        _payload_json: Option<String>,
    ) -> zed::Result<(), String> {
        match command_id.as_str() {
            "demo-toggle-panel" => toggle_panel(context.workspace_id),
            "demo-refresh" => {
                refresh_sidecar_usage()?;
                zed::request_host_mutation(
                    context.workspace_id,
                    &HostMutation::ShowToast("Codex usage refreshed".to_string()),
                )
            }
            "demo-begin-login" => {
                begin_sidecar_login()?;
                zed::request_host_mutation(
                    context.workspace_id,
                    &HostMutation::ShowToast("Opened ChatGPT sign-in in your browser".to_string()),
                )
            }
            "demo-logout" => {
                logout_sidecar()?;
                zed::request_host_mutation(
                    context.workspace_id,
                    &HostMutation::ShowToast("Cleared the saved ChatGPT session".to_string()),
                )
            }
            "demo-open-url" => zed::request_host_mutation(
                context.workspace_id,
                &HostMutation::OpenExternalUrl("https://chatgpt.com".to_string()),
            ),
            unknown => Err(format!("unknown command `{unknown}`")),
        }
    }

    fn open_view(&mut self, contribution_id: String, _context: MountContext) -> zed::Result<u64> {
        match contribution_id.as_str() {
            "demo.panel" | "demo.titlebar" | "demo.footer" => {
                let open_started_at = Instant::now();
                let instance_id = self.next_instance_id;
                self.next_instance_id += 1;
                let contribution_id_for_log = contribution_id.clone();
                println!(
                    "remote-ui-example open_view start contribution_id={} instance_id={}",
                    contribution_id_for_log, instance_id
                );
                let snapshot_started_at = Instant::now();
                let (snapshot, sidecar_error) = load_sidecar_snapshot_result(false);
                println!(
                    "remote-ui-example open_view snapshot contribution_id={} instance_id={} snapshot_ms={} sidecar_error={}",
                    contribution_id_for_log,
                    instance_id,
                    snapshot_started_at.elapsed().as_millis(),
                    sidecar_error.as_deref().unwrap_or("none"),
                );
                self.views.insert(
                    instance_id,
                    DemoViewState {
                        contribution_id,
                        revision: 1,
                        snapshot,
                        sidecar_error,
                    },
                );
                println!(
                    "remote-ui-example open_view done contribution_id={} instance_id={} total_ms={}",
                    contribution_id_for_log,
                    instance_id,
                    open_started_at.elapsed().as_millis(),
                );
                Ok(instance_id)
            }
            unknown => Err(format!("unknown contribution `{unknown}`")),
        }
    }

    fn render_view(
        &mut self,
        instance_id: u64,
        _context: MountContext,
        reason: RenderReason,
    ) -> zed::Result<RemoteViewTree, String> {
        let render_started_at = Instant::now();
        let Some(view) = self.views.get_mut(&instance_id) else {
            return Err(format!("unknown view instance `{instance_id}`"));
        };
        let snapshot_started_at = Instant::now();
        sync_sidecar_snapshot(view, should_force_usage_refresh(reason));
        println!(
            "remote-ui-example render_view snapshot contribution_id={} instance_id={} reason={} snapshot_ms={} revision={} sidecar_error={}",
            view.contribution_id,
            instance_id,
            render_reason_label(reason),
            snapshot_started_at.elapsed().as_millis(),
            view.revision,
            view.sidecar_error.as_deref().unwrap_or("none"),
        );

        let tree = match view.contribution_id.as_str() {
            "demo.titlebar" => Ok(render_titlebar_view(view)),
            "demo.footer" => Ok(render_footer_view(view)),
            "demo.panel" => Ok(render_panel_view(view)),
            unknown => Err(format!("unknown view `{unknown}`")),
        }?;
        println!(
            "remote-ui-example render_view done contribution_id={} instance_id={} reason={} total_ms={} node_count={}",
            view.contribution_id,
            instance_id,
            render_reason_label(reason),
            render_started_at.elapsed().as_millis(),
            tree.nodes.len(),
        );
        Ok(tree)
    }

    fn handle_view_event(
        &mut self,
        instance_id: u64,
        context: MountContext,
        event: RemoteViewEvent,
    ) -> zed::Result<EventOutcome, String> {
        let Some(view) = self.views.get_mut(&instance_id) else {
            return Err(format!("unknown view instance `{instance_id}`"));
        };

        match (event.node_id.as_str(), event.kind) {
            ("titlebar.sync", RemoteViewEventKind::Click)
            | ("panel.refresh", RemoteViewEventKind::Click) => {
                refresh_sidecar_usage()?;
                sync_sidecar_snapshot(view, true);
                Ok(EventOutcome::Rerender)
            }
            ("footer.root", RemoteViewEventKind::Click) => {
                if view.snapshot.is_authenticated() {
                    toggle_panel(context.workspace_id)?;
                    Ok(EventOutcome::Noop)
                } else if !view.snapshot.is_pending() {
                    begin_sidecar_login()?;
                    sync_sidecar_snapshot(view, false);
                    Ok(EventOutcome::Rerender)
                } else {
                    Ok(EventOutcome::Noop)
                }
            }
            ("titlebar.root", RemoteViewEventKind::Click)
            | ("titlebar.icon", RemoteViewEventKind::Click)
            | ("titlebar.badge", RemoteViewEventKind::Click)
            | ("titlebar.toggle-panel", RemoteViewEventKind::Click) => {
                toggle_panel(context.workspace_id)?;
                Ok(EventOutcome::Noop)
            }
            ("titlebar.auth", RemoteViewEventKind::Click)
            | ("footer.auth", RemoteViewEventKind::Click)
            | ("panel.auth", RemoteViewEventKind::Click) => {
                if view.snapshot.is_authenticated() {
                    logout_sidecar()?;
                } else if !view.snapshot.is_pending() {
                    begin_sidecar_login()?;
                }
                sync_sidecar_snapshot(view, false);
                Ok(EventOutcome::Rerender)
            }
            _ => Ok(EventOutcome::Noop),
        }
    }

    fn close_view(&mut self, instance_id: u64) {
        self.views.remove(&instance_id);
    }
}

fn render_titlebar_view(view: &DemoViewState) -> RemoteViewTree {
    let nodes = vec![
        node(
            "titlebar.root",
            None,
            RemoteViewNodeKind::Row,
            [
                ("clickable", "true"),
                ("gap", "4"),
                ("tooltip", "Toggle Codex usage panel"),
            ],
        ),
        node(
            "titlebar.icon",
            Some("titlebar.root"),
            RemoteViewNodeKind::Icon("icons/ai_open_ai.svg".to_string()),
            [],
        ),
        node(
            "titlebar.badge",
            Some("titlebar.root"),
            RemoteViewNodeKind::Badge(view.snapshot.status_badge()),
            [],
        ),
    ];
    RemoteViewTree {
        revision: view.revision,
        root_id: "titlebar.root".to_string(),
        nodes,
    }
}

fn render_footer_view(view: &DemoViewState) -> RemoteViewTree {
    let primary_remaining_percent = view.snapshot.primary_remaining_percent();

    RemoteViewTree {
        revision: view.revision,
        root_id: "footer.root".to_string(),
        nodes: vec![
            node(
                "footer.root",
                None,
                RemoteViewNodeKind::Row,
                [
                    ("clickable", "true"),
                    ("gap", "4"),
                    (
                        "tooltip",
                        view.snapshot.footer_tooltip(view.sidecar_error.as_deref()),
                    ),
                ],
            ),
            node(
                "footer.icon",
                Some("footer.root"),
                RemoteViewNodeKind::Icon("icons/ai_open_ai.svg".to_string()),
                [],
            ),
            node(
                "footer.summary",
                Some("footer.root"),
                RemoteViewNodeKind::Text(view.snapshot.footer_summary()),
                [],
            ),
            node(
                "footer.progress",
                Some("footer.root"),
                RemoteViewNodeKind::ProgressBar(ProgressBarProps {
                    value: primary_remaining_percent,
                    max_value: 100,
                }),
                [],
            ),
        ],
    }
}

fn render_panel_view(view: &DemoViewState) -> RemoteViewTree {
    let mut nodes = vec![node(
        "panel.root",
        None,
        RemoteViewNodeKind::Column,
        [("gap", "8")],
    )];

    if let Some(plan_label) = view.snapshot.panel_plan_label() {
        nodes.push(node(
            "panel.plan",
            Some("panel.root"),
            RemoteViewNodeKind::Text(plan_label),
            [],
        ));
    }

    for usage_limit in view.snapshot.usage_limits.iter().take(5) {
        let section_id = format!("panel.limit.{}", usage_limit.id);
        let title_id = format!("{section_id}.title");
        let value_id = format!("{section_id}.value");
        let progress_id = format!("{section_id}.progress");
        nodes.push(node(
            section_id.clone(),
            Some("panel.root"),
            RemoteViewNodeKind::Column,
            [("gap", "4")],
        ));
        nodes.push(node(
            title_id,
            Some(section_id.as_str()),
            RemoteViewNodeKind::Text(usage_limit.title.clone()),
            [],
        ));
        nodes.push(node(
            value_id,
            Some(section_id.as_str()),
            RemoteViewNodeKind::Text(format!("{}% remaining", usage_limit.remaining_percent)),
            [],
        ));
        nodes.push(node(
            progress_id,
            Some(section_id.as_str()),
            RemoteViewNodeKind::ProgressBar(ProgressBarProps {
                value: usage_limit.remaining_percent,
                max_value: 100,
            }),
            [],
        ));
        if let Some(resets_at_label) = usage_limit.resets_at_label.as_deref() {
            let reset_id = format!("{section_id}.reset");
            nodes.push(node(
                reset_id,
                Some(section_id.as_str()),
                RemoteViewNodeKind::Text(format!("Resets {resets_at_label}")),
                [],
            ));
        }
    }

    nodes.push(node(
        "panel.credits",
        Some("panel.root"),
        RemoteViewNodeKind::Column,
        [("gap", "4")],
    ));
    nodes.push(node(
        "panel.credits.title",
        Some("panel.credits"),
        RemoteViewNodeKind::Text("Credits remaining".to_string()),
        [],
    ));
    nodes.push(node(
        "panel.credits.balance",
        Some("panel.credits"),
        RemoteViewNodeKind::Text(view.snapshot.credits_summary.balance_label.clone()),
        [],
    ));
    nodes.push(node(
        "panel.credits.detail",
        Some("panel.credits"),
        RemoteViewNodeKind::Text(view.snapshot.credits_summary.detail.clone()),
        [],
    ));

    if let Some(sidecar_error) = view.sidecar_error.as_deref() {
        nodes.push(node(
            "panel.error",
            Some("panel.root"),
            RemoteViewNodeKind::Text(sidecar_error.to_string()),
            [],
        ));
    }

    RemoteViewTree {
        revision: view.revision,
        root_id: "panel.root".to_string(),
        nodes,
    }
}

fn sync_sidecar_snapshot(view: &mut DemoViewState, force_refresh: bool) {
    let sync_started_at = Instant::now();
    let (snapshot, sidecar_error) = load_sidecar_snapshot_result(force_refresh);
    view.snapshot = snapshot;
    view.sidecar_error = sidecar_error;
    view.revision = view.revision.saturating_add(1);
    println!(
        "remote-ui-example sync_sidecar_snapshot contribution_id={} force_refresh={} total_ms={} revision={} sidecar_error={}",
        view.contribution_id,
        force_refresh,
        sync_started_at.elapsed().as_millis(),
        view.revision,
        view.sidecar_error.as_deref().unwrap_or("none"),
    );
}

fn begin_sidecar_login() -> zed::Result<(), String> {
    zed::sidecar::call("auth.begin-login", None, Some(5_000)).map(|_| ())
}

fn refresh_sidecar_usage() -> zed::Result<(), String> {
    let _ = load_sidecar_snapshot(true)?;
    Ok(())
}

fn logout_sidecar() -> zed::Result<(), String> {
    zed::sidecar::call("auth.logout", None, Some(5_000)).map(|_| ())
}

fn load_sidecar_snapshot(force_refresh: bool) -> zed::Result<SidecarSnapshot, String> {
    let call_started_at = Instant::now();
    println!(
        "remote-ui-example load_sidecar_snapshot start force_refresh={}",
        force_refresh
    );

    let mut state = load_persisted_auth_state_from_disk()?;
    let snapshot = if state.auth_status == "pending" {
        pending_snapshot()
    } else if state.refresh_token.is_none() {
        state.last_snapshot.clone().unwrap_or_else(|| {
            if state.auth_status == "error" {
                error_snapshot(state.last_error.as_deref())
            } else {
                SidecarSnapshot::signed_out()
            }
        })
    } else if !force_refresh {
        match state.last_snapshot.clone() {
            Some(snapshot) => snapshot,
            None => fetch_and_store_usage_snapshot(&mut state, false)?,
        }
    } else {
        fetch_and_store_usage_snapshot(&mut state, true)?
    };

    println!(
        "remote-ui-example load_sidecar_snapshot done total_ms={} outcome=ok",
        call_started_at.elapsed().as_millis(),
    );
    Ok(snapshot)
}

fn load_sidecar_snapshot_result(force_refresh: bool) -> (SidecarSnapshot, Option<String>) {
    match load_sidecar_snapshot(force_refresh) {
        Ok(snapshot) => (snapshot, None),
        Err(error) => {
            println!(
                "remote-ui-example load_sidecar_snapshot done total_ms=error error={}",
                error
            );
            (SidecarSnapshot::signed_out(), Some(error))
        }
    }
}

fn fetch_and_store_usage_snapshot(
    state: &mut PersistedAuthState,
    force_refresh: bool,
) -> zed::Result<SidecarSnapshot, String> {
    let access_token = ensure_access_token(state, force_refresh)?;

    match fetch_usage_snapshot(state, &access_token) {
        Ok(snapshot) => {
            state.auth_status = "authenticated".to_string();
            state.last_snapshot = Some(snapshot.clone());
            state.last_fetched_at = current_time_millis()?;
            state.last_error = None;
            save_persisted_auth_state(state)?;
            Ok(snapshot)
        }
        Err(error) => {
            state.last_error = Some(error.clone());
            save_persisted_auth_state(state)?;
            Err(error)
        }
    }
}

fn ensure_access_token(
    state: &mut PersistedAuthState,
    force_refresh: bool,
) -> zed::Result<String, String> {
    let now = current_time_millis()?;
    let expires_soon = state.expires_at == 0
        || state.expires_at <= now.saturating_add(TOKEN_REFRESH_SKEW_MILLIS);

    if force_refresh || state.access_token.is_none() || expires_soon {
        refresh_access_token(state)?;
    }

    state
        .access_token
        .clone()
        .ok_or_else(|| "missing access token after refresh".to_string())
}

fn refresh_access_token(state: &mut PersistedAuthState) -> zed::Result<(), String> {
    let refresh_token = state
        .refresh_token
        .clone()
        .ok_or_else(|| "no refresh token is available".to_string())?;
    let response = fetch_json_request(
        HttpMethod::Post,
        TOKEN_URL,
        &[("Content-Type", "application/x-www-form-urlencoded")],
        Some(form_body(&[
            ("grant_type", "refresh_token"),
            ("client_id", CLIENT_ID),
            ("refresh_token", refresh_token.as_str()),
        ])),
    )?;
    let object = response
        .as_object()
        .ok_or_else(|| "token refresh response was not an object".to_string())?;

    state.access_token = Some(required_string(object, "access_token")?);
    state.refresh_token = Some(
        optional_string(object, "refresh_token").unwrap_or(refresh_token),
    );
    state.expires_at = current_time_millis()?.saturating_add(
        optional_u64(object, "expires_in")
            .unwrap_or(DEFAULT_TOKEN_EXPIRY_SECONDS)
            .saturating_mul(1_000),
    );
    state.last_refresh_at = current_time_millis()?;
    state.auth_status = "authenticated".to_string();
    Ok(())
}

fn fetch_usage_snapshot(
    state: &mut PersistedAuthState,
    access_token: &str,
) -> zed::Result<SidecarSnapshot, String> {
    let mut headers = vec![
        ("Accept", "application/json".to_string()),
        ("Authorization", format!("Bearer {access_token}")),
    ];
    if let Some(account_id) = state.account_id.as_deref() {
        headers.push(("ChatGPT-Account-Id", account_id.to_string()));
    }

    let response = fetch_json_request_owned(HttpMethod::Get, USAGE_URL, &headers, None)?;
    normalize_usage_snapshot(&response)
}

fn fetch_json_request(
    method: HttpMethod,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<Vec<u8>>,
) -> zed::Result<zed::serde_json::Value, String> {
    let request = headers.iter().fold(
        HttpRequest::builder().method(method).url(url.to_string()),
        |builder, (name, value)| builder.header((*name).to_string(), (*value).to_string()),
    );
    let request = match body {
        Some(body) => request.body(body).build()?,
        None => request.build()?,
    };
    let response = request.fetch()?;
    zed::serde_json::from_slice(&response.body)
        .map_err(|error| format!("failed to parse JSON response from `{url}`: {error}"))
}

fn fetch_json_request_owned(
    method: HttpMethod,
    url: &str,
    headers: &[(impl AsRef<str>, String)],
    body: Option<Vec<u8>>,
) -> zed::Result<zed::serde_json::Value, String> {
    let request = headers.iter().fold(
        HttpRequest::builder().method(method).url(url.to_string()),
        |builder, (name, value)| builder.header(name.as_ref().to_string(), value.clone()),
    );
    let request = match body {
        Some(body) => request.body(body).build()?,
        None => request.build()?,
    };
    let response = request.fetch()?;
    zed::serde_json::from_slice(&response.body)
        .map_err(|error| format!("failed to parse JSON response from `{url}`: {error}"))
}

fn form_body(parameters: &[(&str, &str)]) -> Vec<u8> {
    parameters
        .iter()
        .map(|(name, value)| format!("{}={}", encode(name), encode(value)))
        .collect::<Vec<_>>()
        .join("&")
        .into_bytes()
}

fn should_force_usage_refresh(reason: RenderReason) -> bool {
    matches!(reason, RenderReason::ExplicitRefresh)
}

fn pending_snapshot() -> SidecarSnapshot {
    SidecarSnapshot {
        auth_status: "pending".to_string(),
        ..SidecarSnapshot::signed_out()
    }
}

fn error_snapshot(message: Option<&str>) -> SidecarSnapshot {
    let mut snapshot = SidecarSnapshot::signed_out();
    snapshot.auth_status = "error".to_string();
    snapshot.credits_summary.detail = message
        .unwrap_or("ChatGPT authentication failed.")
        .to_string();
    snapshot
}

fn render_reason_label(reason: RenderReason) -> &'static str {
    match reason {
        RenderReason::Initial => "initial",
        RenderReason::Event => "event",
        RenderReason::HostContextChanged => "host-context-changed",
        RenderReason::VirtualRangeChanged => "virtual-range-changed",
        RenderReason::ExplicitRefresh => "explicit-refresh",
    }
}

fn toggle_panel(workspace_id: u64) -> zed::Result<(), String> {
    zed::request_host_mutation(workspace_id, &HostMutation::TogglePanel("demo".to_string()))
}

fn parse_sidecar_snapshot(value: zed::serde_json::Value) -> zed::Result<SidecarSnapshot, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "sidecar snapshot must be a JSON object".to_string())?;

    let auth_status = required_string(object, "auth_status")?;

    Ok(SidecarSnapshot {
        auth_status,
        plan_type: optional_string(object, "plan_type"),
        primary_used_percent: optional_u32(object, "primary_used_percent")
            .unwrap_or(0)
            .min(100),
        usage_limits: parse_usage_limits(object.get("usage_limits")),
        credits_summary: parse_credits_summary(object.get("credits_summary")),
    })
}

fn parse_usage_limits(value: Option<&zed::serde_json::Value>) -> Vec<UsageLimitCard> {
    let Some(array) = value.and_then(zed::serde_json::Value::as_array) else {
        return Vec::new();
    };

    array
        .iter()
        .filter_map(zed::serde_json::Value::as_object)
        .filter_map(|object| {
            Some(UsageLimitCard {
                id: required_string(object, "id").ok()?,
                title: required_string(object, "title").ok()?,
                remaining_percent: optional_u32(object, "remaining_percent")
                    .unwrap_or(0)
                    .min(100),
                resets_at_label: optional_string(object, "resets_at_label"),
            })
        })
        .collect()
}

fn parse_credits_summary(value: Option<&zed::serde_json::Value>) -> CreditsSummary {
    let Some(object) = value.and_then(zed::serde_json::Value::as_object) else {
        return CreditsSummary {
            balance_label: "0".to_string(),
            detail: "Use credits to send messages beyond your plan limit.".to_string(),
        };
    };

    CreditsSummary {
        balance_label: optional_string(object, "balance_label").unwrap_or_else(|| "0".to_string()),
        detail: optional_string(object, "detail")
            .unwrap_or_else(|| "Use credits to send messages beyond your plan limit.".to_string()),
    }
}

fn parse_persisted_auth_state(
    value: zed::serde_json::Value,
) -> zed::Result<PersistedAuthState, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "persisted auth state must be a JSON object".to_string())?;
    let access_token = optional_string(object, "access_token");
    let refresh_token = optional_string(object, "refresh_token");
    let auth_status = optional_string(object, "auth_status").unwrap_or_else(|| {
        if refresh_token.is_some() {
            "authenticated".to_string()
        } else if optional_string(object, "last_error").is_some() {
            "error".to_string()
        } else {
            "signed-out".to_string()
        }
    });

    Ok(PersistedAuthState {
        auth_status,
        access_token,
        refresh_token,
        account_id: optional_string(object, "account_id"),
        expires_at: optional_u64(object, "expires_at").unwrap_or(0),
        last_refresh_at: optional_u64(object, "last_refresh_at").unwrap_or(0),
        last_snapshot: object
            .get("last_snapshot")
            .cloned()
            .map(parse_sidecar_snapshot)
            .transpose()?,
        last_fetched_at: optional_u64(object, "last_fetched_at").unwrap_or(0),
        last_error: optional_string(object, "last_error"),
    })
}

fn load_persisted_auth_state_from_disk() -> zed::Result<PersistedAuthState, String> {
    let path = auth_state_path()?;
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PersistedAuthState::signed_out());
        }
        Err(error) => return Err(format!("failed to read auth state `{}`: {error}", path.display())),
    };
    let value = zed::serde_json::from_str(&raw)
        .map_err(|error| format!("failed to parse auth state `{}`: {error}", path.display()))?;
    parse_persisted_auth_state(value)
}

fn save_persisted_auth_state(state: &PersistedAuthState) -> zed::Result<(), String> {
    let path = auth_state_path()?;
    let json = zed::serde_json::json!({
        "auth_status": state.auth_status,
        "access_token": state.access_token,
        "refresh_token": state.refresh_token,
        "account_id": state.account_id,
        "expires_at": state.expires_at,
        "last_refresh_at": state.last_refresh_at,
        "last_snapshot": state.last_snapshot.as_ref().map(sidecar_snapshot_to_json),
        "last_fetched_at": state.last_fetched_at,
        "last_error": state.last_error,
    });
    let bytes = zed::serde_json::to_vec_pretty(&json)
        .map_err(|error| format!("failed to encode auth state `{}`: {error}", path.display()))?;
    fs::write(&path, bytes)
        .map_err(|error| format!("failed to write auth state `{}`: {error}", path.display()))
}

fn auth_state_path() -> zed::Result<PathBuf, String> {
    std::env::current_dir()
        .map(|current_dir| current_dir.join(AUTH_STATE_PATH))
        .map_err(|error| format!("failed to determine extension work directory: {error}"))
}

fn sidecar_snapshot_to_json(snapshot: &SidecarSnapshot) -> zed::serde_json::Value {
    zed::serde_json::json!({
        "auth_status": snapshot.auth_status,
        "plan_type": snapshot.plan_type,
        "primary_used_percent": snapshot.primary_used_percent,
        "usage_limits": snapshot.usage_limits.iter().map(|usage_limit| {
            zed::serde_json::json!({
                "id": usage_limit.id,
                "title": usage_limit.title,
                "remaining_percent": usage_limit.remaining_percent,
                "resets_at_label": usage_limit.resets_at_label,
            })
        }).collect::<Vec<_>>(),
        "credits_summary": {
            "balance_label": snapshot.credits_summary.balance_label,
            "detail": snapshot.credits_summary.detail,
        }
    })
}

fn normalize_usage_snapshot(value: &zed::serde_json::Value) -> zed::Result<SidecarSnapshot, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "usage response was not a JSON object".to_string())?;
    let rate_limit = object.get("rate_limit");
    let code_review_rate_limit = object.get("code_review_rate_limit");
    let additional_rate_limits = object
        .get("additional_rate_limits")
        .and_then(zed::serde_json::Value::as_array);
    let primary_window = rate_limit
        .and_then(zed::serde_json::Value::as_object)
        .and_then(|rate_limit| rate_limit.get("primary_window"));

    let mut usage_limits = normalize_limit_windows(rate_limit, "codex", "5 hour usage limit", "Weekly usage limit");
    usage_limits.extend(normalize_limit_windows(
        code_review_rate_limit,
        "code-review",
        "Code review",
        "Code review weekly usage limit",
    ));

    if let Some(additional_rate_limits) = additional_rate_limits {
        for (index, entry) in additional_rate_limits.iter().enumerate() {
            let limit_name = entry
                .as_object()
                .and_then(|entry| entry.get("limit_name"))
                .and_then(zed::serde_json::Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| format!("Additional limit {}", index + 1));
            usage_limits.extend(normalize_limit_windows(
                entry.as_object().and_then(|entry| entry.get("rate_limit")),
                &format!("additional-{index}"),
                &format!("{limit_name} 5 hour usage limit"),
                &format!("{limit_name} Weekly usage limit"),
            ));
        }
    }

    Ok(SidecarSnapshot {
        auth_status: "authenticated".to_string(),
        plan_type: optional_string(object, "plan_type"),
        primary_used_percent: primary_window
            .and_then(zed::serde_json::Value::as_object)
            .and_then(|window| window.get("used_percent"))
            .and_then(read_percent)
            .unwrap_or(0),
        usage_limits,
        credits_summary: normalize_credits_summary(object.get("credits_summary")),
    })
}

fn normalize_limit_windows(
    rate_limit: Option<&zed::serde_json::Value>,
    prefix: &str,
    primary_title: &str,
    secondary_title: &str,
) -> Vec<UsageLimitCard> {
    let Some(rate_limit) = rate_limit.and_then(zed::serde_json::Value::as_object) else {
        return Vec::new();
    };
    let mut windows = Vec::new();
    if let Some(window) = normalize_usage_window(
        rate_limit.get("primary_window"),
        &format!("{prefix}-primary"),
        primary_title,
    ) {
        windows.push(window);
    }
    if let Some(window) = normalize_usage_window(
        rate_limit.get("secondary_window"),
        &format!("{prefix}-secondary"),
        secondary_title,
    ) {
        windows.push(window);
    }
    windows
}

fn normalize_usage_window(
    window: Option<&zed::serde_json::Value>,
    id: &str,
    title: &str,
) -> Option<UsageLimitCard> {
    let window = window?.as_object()?;
    if window.get("limit_window_seconds").is_none()
        && window.get("reset_at").is_none()
        && window.get("reset_after_seconds").is_none()
    {
        return None;
    }

    let used_percent = window
        .get("used_percent")
        .and_then(read_percent)
        .unwrap_or(0);
    Some(UsageLimitCard {
        id: id.to_string(),
        title: title.to_string(),
        remaining_percent: 100_u32.saturating_sub(used_percent),
        resets_at_label: None,
    })
}

fn normalize_credits_summary(value: Option<&zed::serde_json::Value>) -> CreditsSummary {
    let Some(object) = value.and_then(zed::serde_json::Value::as_object) else {
        return CreditsSummary {
            balance_label: "0".to_string(),
            detail: "Use credits to send messages beyond your plan limit.".to_string(),
        };
    };

    if object
        .get("unlimited")
        .and_then(zed::serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        return CreditsSummary {
            balance_label: "Unlimited".to_string(),
            detail: "Credits are unlimited.".to_string(),
        };
    }

    CreditsSummary {
        balance_label: object
            .get("balance")
            .and_then(read_number)
            .unwrap_or(0.0)
            .to_string(),
        detail: "Use credits to send messages beyond your plan limit.".to_string(),
    }
}

fn read_number(value: &zed::serde_json::Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_u64().map(|value| value as f64))
        .or_else(|| {
            value.as_str().and_then(|value| {
                value
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite())
            })
        })
}

fn read_percent(value: &zed::serde_json::Value) -> Option<u32> {
    read_number(value).map(|value| value.round().clamp(0.0, 100.0) as u32)
}

fn required_string(
    object: &zed::serde_json::Map<String, zed::serde_json::Value>,
    field_name: &str,
) -> zed::Result<String, String> {
    object
        .get(field_name)
        .and_then(zed::serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("sidecar snapshot is missing `{field_name}`"))
}

fn optional_string(
    object: &zed::serde_json::Map<String, zed::serde_json::Value>,
    field_name: &str,
) -> Option<String> {
    object
        .get(field_name)
        .and_then(zed::serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

fn optional_u32(
    object: &zed::serde_json::Map<String, zed::serde_json::Value>,
    field_name: &str,
) -> Option<u32> {
    object
        .get(field_name)
        .and_then(zed::serde_json::Value::as_u64)
        .map(|value| value.min(u32::MAX as u64) as u32)
}

fn optional_u64(
    object: &zed::serde_json::Map<String, zed::serde_json::Value>,
    field_name: &str,
) -> Option<u64> {
    object
        .get(field_name)
        .and_then(zed::serde_json::Value::as_u64)
}

fn current_time_millis() -> zed::Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .map_err(|error| format!("system clock is before the Unix epoch: {error}"))
}

fn capitalize_plan_word(word: &str) -> String {
    let mut characters = word.chars();
    let Some(first_character) = characters.next() else {
        return String::new();
    };

    first_character
        .to_uppercase()
        .chain(characters.flat_map(|character| character.to_lowercase()))
        .collect()
}

fn node(
    node_id: impl Into<String>,
    parent_id: Option<&str>,
    kind: RemoteViewNodeKind,
    properties: impl IntoIterator<Item = (&'static str, &'static str)>,
) -> RemoteViewNode {
    RemoteViewNode {
        node_id: node_id.into(),
        parent_id: parent_id.map(ToOwned::to_owned),
        kind,
        properties: properties
            .into_iter()
            .map(|(name, value)| RemoteViewProperty {
                name: name.to_string(),
                value: value.to_string(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo_view_state() -> DemoViewState {
        DemoViewState {
            contribution_id: "demo.panel".to_string(),
            revision: 1,
            snapshot: SidecarSnapshot {
                auth_status: "authenticated".to_string(),
                plan_type: Some("pro".to_string()),
                primary_used_percent: 6,
                usage_limits: vec![
                    UsageLimitCard {
                        id: "codex-primary".to_string(),
                        title: "5 hour usage limit".to_string(),
                        remaining_percent: 94,
                        resets_at_label: Some("1:12 PM".to_string()),
                    },
                    UsageLimitCard {
                        id: "codex-secondary".to_string(),
                        title: "Weekly usage limit".to_string(),
                        remaining_percent: 29,
                        resets_at_label: Some("5:25 PM".to_string()),
                    },
                    UsageLimitCard {
                        id: "spark-primary".to_string(),
                        title: "GPT-5.3-Codex-Spark 5 hour usage limit".to_string(),
                        remaining_percent: 100,
                        resets_at_label: None,
                    },
                    UsageLimitCard {
                        id: "spark-secondary".to_string(),
                        title: "GPT-5.3-Codex-Spark Weekly usage limit".to_string(),
                        remaining_percent: 100,
                        resets_at_label: None,
                    },
                    UsageLimitCard {
                        id: "code-review-primary".to_string(),
                        title: "Code review".to_string(),
                        remaining_percent: 100,
                        resets_at_label: None,
                    },
                ],
                credits_summary: CreditsSummary {
                    balance_label: "0".to_string(),
                    detail: "Use credits to send messages beyond your plan limit.".to_string(),
                },
            },
            sidecar_error: None,
        }
    }

    #[test]
    fn parse_sidecar_snapshot_reads_expected_usage_fields() {
        let snapshot = parse_sidecar_snapshot(zed::serde_json::json!({
            "auth_status": "authenticated",
            "plan_type": "pro",
            "primary_used_percent": 106,
            "usage_limits": [
                {
                    "id": "codex-primary",
                    "title": "5 hour usage limit",
                    "used_percent": 6,
                    "remaining_percent": 94,
                    "resets_at_label": "1:12 PM"
                },
                {
                    "id": "codex-secondary",
                    "title": "Weekly usage limit",
                    "used_percent": 71,
                    "remaining_percent": 29,
                    "resets_at_label": "5:25 PM"
                }
            ],
            "credits_summary": {
                "balance_label": "0",
                "detail": "Use credits to send messages beyond your plan limit."
            }
        }))
        .expect("snapshot should parse");

        assert_eq!(snapshot.auth_status, "authenticated");
        assert_eq!(snapshot.plan_type.as_deref(), Some("pro"));
        assert_eq!(snapshot.primary_used_percent, 100);
        assert_eq!(snapshot.usage_limits.len(), 2);
        assert_eq!(snapshot.usage_limits[0].remaining_percent, 94);
        assert_eq!(snapshot.credits_summary.balance_label, "0");
    }

    #[test]
    fn titlebar_view_stays_compact() {
        let view = demo_view_state();

        let tree = render_titlebar_view(&view);
        let node_ids = tree
            .nodes
            .iter()
            .map(|node| node.node_id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(tree.root_id, "titlebar.root");
        assert_eq!(
            node_ids,
            vec!["titlebar.root", "titlebar.icon", "titlebar.badge"]
        );
    }

    #[test]
    fn panel_view_renders_usage_cards_and_compact_credits_card() {
        let mut view = demo_view_state();
        view.sidecar_error = Some("usage request failed with HTTP 401".to_string());

        let tree = render_panel_view(&view);
        let node_ids = tree
            .nodes
            .iter()
            .map(|node| node.node_id.as_str())
            .collect::<Vec<_>>();

        assert!(node_ids.contains(&"panel.limit.codex-primary"));
        assert!(node_ids.contains(&"panel.limit.codex-secondary"));
        assert!(node_ids.contains(&"panel.limit.spark-primary"));
        assert!(node_ids.contains(&"panel.limit.spark-secondary"));
        assert!(node_ids.contains(&"panel.limit.code-review-primary"));
        assert!(node_ids.contains(&"panel.plan"));
        assert!(node_ids.contains(&"panel.credits"));
        assert!(node_ids.contains(&"panel.credits.balance"));
        assert!(node_ids.contains(&"panel.error"));
    }

    #[test]
    fn footer_view_shows_primary_usage_progress() {
        let view = demo_view_state();

        let tree = render_footer_view(&view);
        let node_ids = tree
            .nodes
            .iter()
            .map(|node| node.node_id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(tree.root_id, "footer.root");
        assert_eq!(
            node_ids,
            vec![
                "footer.root",
                "footer.icon",
                "footer.summary",
                "footer.progress",
            ]
        );

        let progress = tree
            .nodes
            .iter()
            .find(|node| node.node_id == "footer.progress")
            .expect("footer progress node should exist");
        let summary = tree
            .nodes
            .iter()
            .find(|node| node.node_id == "footer.summary")
            .expect("footer summary node should exist");

        assert!(matches!(
            progress.kind,
            RemoteViewNodeKind::ProgressBar(ProgressBarProps {
                value: 94,
                max_value: 100,
            })
        ));
        assert!(matches!(
            summary.kind,
            RemoteViewNodeKind::Text(ref label) if label == "94% left"
        ));
    }

    #[test]
    fn explicit_refresh_forces_a_usage_reload() {
        assert!(!should_force_usage_refresh(RenderReason::Initial));
        assert!(!should_force_usage_refresh(RenderReason::Event));
        assert!(should_force_usage_refresh(RenderReason::ExplicitRefresh));
    }

    #[test]
    fn persisted_auth_state_reads_pending_status_and_cached_snapshot() {
        let state = parse_persisted_auth_state(zed::serde_json::json!({
            "auth_status": "pending",
            "last_snapshot": {
                "auth_status": "authenticated",
                "plan_type": "pro",
                "primary_used_percent": 40,
                "usage_limits": [],
                "credits_summary": {
                    "balance_label": "12",
                    "detail": "Credits remain available."
                }
            },
            "last_error": null,
            "last_fetched_at": 55
        }))
        .expect("state should parse");

        assert_eq!(state.auth_status, "pending");
        assert_eq!(state.last_fetched_at, 55);
        assert_eq!(
            state
                .last_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.plan_type.as_deref()),
            Some("pro")
        );
    }
}

zed::register_extension!(RemoteUiExampleExtension);
