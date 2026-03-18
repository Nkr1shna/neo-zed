use std::collections::HashMap;

use zed_extension_api::{
    self as zed, CommandContext, EventOutcome, HostMutation, MountContext, ProgressBarProps,
    RemoteViewEvent, RemoteViewEventKind, RemoteViewNode, RemoteViewNodeKind, RemoteViewProperty,
    RemoteViewTree, RenderReason,
};

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

#[derive(Clone)]
struct SidecarSnapshot {
    auth_status: String,
    status_label: String,
    detail: String,
    plan_type: Option<String>,
    account_label: Option<String>,
    primary_window_label: String,
    secondary_window_label: String,
    primary_used_percent: u32,
    secondary_used_percent: u32,
    busy: bool,
}

impl SidecarSnapshot {
    fn signed_out() -> Self {
        Self {
            auth_status: "signed-out".to_string(),
            status_label: "Sign in to ChatGPT".to_string(),
            detail: "Start the OAuth flow to read your Codex usage windows.".to_string(),
            plan_type: None,
            account_label: None,
            primary_window_label: "5h window".to_string(),
            secondary_window_label: "7d window".to_string(),
            primary_used_percent: 0,
            secondary_used_percent: 0,
            busy: false,
        }
    }

    fn status_badge(&self) -> String {
        format!("{}%", self.primary_used_percent)
    }

    fn plan_badge(&self) -> Option<&str> {
        match self.plan_type.as_deref() {
            Some(plan_type) if !plan_type.is_empty() => Some(plan_type),
            _ => None,
        }
    }

    fn footer_state_label(&self, sidecar_error: Option<&str>) -> &'static str {
        if sidecar_error.is_some() {
            "Retry"
        } else if self.is_pending() {
            "Waiting"
        } else if self.is_authenticated() {
            "Weekly"
        } else {
            "Log in"
        }
    }

    fn auth_button_label(&self, sidecar_error: Option<&str>) -> &'static str {
        if sidecar_error.is_some() {
            "Retry"
        } else if self.is_authenticated() {
            "Sign Out"
        } else if self.is_pending() {
            "Waiting"
        } else {
            "Log in"
        }
    }

    fn is_authenticated(&self) -> bool {
        self.auth_status == "authenticated"
    }

    fn is_pending(&self) -> bool {
        self.auth_status == "pending"
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
            "demo-open-panel" => zed::request_host_mutation(
                context.workspace_id,
                &HostMutation::OpenPanel("demo".to_string()),
            ),
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
                let instance_id = self.next_instance_id;
                self.next_instance_id += 1;
                let (snapshot, sidecar_error) = load_sidecar_snapshot(false);
                self.views.insert(
                    instance_id,
                    DemoViewState {
                        contribution_id,
                        revision: 1,
                        snapshot,
                        sidecar_error,
                    },
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
        _reason: RenderReason,
    ) -> zed::Result<RemoteViewTree, String> {
        let Some(view) = self.views.get_mut(&instance_id) else {
            return Err(format!("unknown view instance `{instance_id}`"));
        };
        sync_sidecar_snapshot(view, false);

        match view.contribution_id.as_str() {
            "demo.titlebar" => Ok(render_titlebar_view(view)),
            "demo.footer" => Ok(render_footer_view(view)),
            "demo.panel" => Ok(render_panel_view(view)),
            unknown => Err(format!("unknown view `{unknown}`")),
        }
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
            | ("footer.sync", RemoteViewEventKind::Click)
            | ("panel.refresh", RemoteViewEventKind::Click) => {
                refresh_sidecar_usage()?;
                sync_sidecar_snapshot(view, true);
                Ok(EventOutcome::Rerender)
            }
            ("footer.status", RemoteViewEventKind::Click) => {
                if view.snapshot.is_authenticated() {
                    zed::request_host_mutation(
                        context.workspace_id,
                        &HostMutation::OpenPanel("demo".to_string()),
                    )?;
                    Ok(EventOutcome::Noop)
                } else if !view.snapshot.is_pending() {
                    begin_sidecar_login()?;
                    sync_sidecar_snapshot(view, false);
                    Ok(EventOutcome::Rerender)
                } else {
                    Ok(EventOutcome::Noop)
                }
            }
            ("titlebar.icon", RemoteViewEventKind::Click)
            | ("titlebar.badge", RemoteViewEventKind::Click)
            | ("titlebar.open-panel", RemoteViewEventKind::Click) => {
                zed::request_host_mutation(
                    context.workspace_id,
                    &HostMutation::OpenPanel("demo".to_string()),
                )?;
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
            ("footer.open-panel", RemoteViewEventKind::Click) => {
                zed::request_host_mutation(
                    context.workspace_id,
                    &HostMutation::OpenPanel("demo".to_string()),
                )?;
                Ok(EventOutcome::Noop)
            }
            ("panel.copy-status", RemoteViewEventKind::Click) => {
                zed::request_host_mutation(
                    context.workspace_id,
                    &HostMutation::CopyToClipboard(snapshot_status_text(
                        &view.snapshot,
                        view.sidecar_error.as_deref(),
                    )),
                )?;
                Ok(EventOutcome::Noop)
            }
            ("panel.open-url", RemoteViewEventKind::Click) => {
                zed::request_host_mutation(
                    context.workspace_id,
                    &HostMutation::OpenExternalUrl("https://chatgpt.com".to_string()),
                )?;
                Ok(EventOutcome::Noop)
            }
            _ => Ok(EventOutcome::Noop),
        }
    }

    fn close_view(&mut self, instance_id: u64) {
        self.views.remove(&instance_id);
    }
}

fn render_titlebar_view(view: &DemoViewState) -> RemoteViewTree {
    let mut nodes = vec![
        node(
            "titlebar.root",
            None,
            RemoteViewNodeKind::Row,
            [("clickable", "true"), ("gap", "4")],
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

    if let Some(plan_badge) = view.snapshot.plan_badge() {
        nodes.push(node(
            "titlebar.plan",
            Some("titlebar.root"),
            RemoteViewNodeKind::Badge(plan_badge.to_string()),
            [],
        ));
    }
    RemoteViewTree {
        revision: view.revision,
        root_id: "titlebar.root".to_string(),
        nodes,
    }
}

fn render_footer_view(view: &DemoViewState) -> RemoteViewTree {
    RemoteViewTree {
        revision: view.revision,
        root_id: "footer.root".to_string(),
        nodes: vec![
            node("footer.root", None, RemoteViewNodeKind::Row, []),
            node(
                "footer.icon",
                Some("footer.root"),
                RemoteViewNodeKind::Icon("icons/ai_open_ai.svg".to_string()),
                [],
            ),
            node(
                "footer.status",
                Some("footer.root"),
                RemoteViewNodeKind::Button(
                    view.snapshot
                        .footer_state_label(view.sidecar_error.as_deref())
                        .to_string(),
                ),
                [],
            ),
            node(
                "footer.badge",
                Some("footer.root"),
                RemoteViewNodeKind::Badge(format!("7d {}", view.snapshot.secondary_used_percent)),
                [],
            ),
            node(
                "footer.progress",
                Some("footer.root"),
                RemoteViewNodeKind::ProgressBar(ProgressBarProps {
                    value: view.snapshot.secondary_used_percent,
                    max_value: 100,
                }),
                [],
            ),
            node(
                "footer.sync",
                Some("footer.root"),
                RemoteViewNodeKind::Button("Sync".to_string()),
                [],
            ),
            node(
                "footer.auth",
                Some("footer.root"),
                RemoteViewNodeKind::Button(
                    view.snapshot
                        .auth_button_label(view.sidecar_error.as_deref())
                        .to_string(),
                ),
                [],
            ),
            node(
                "footer.open-panel",
                Some("footer.root"),
                RemoteViewNodeKind::Button("Open".to_string()),
                [],
            ),
        ],
    }
}

fn render_panel_view(view: &DemoViewState) -> RemoteViewTree {
    let mut nodes = vec![
        node("panel.root", None, RemoteViewNodeKind::Column, []),
        node(
            "panel.header",
            Some("panel.root"),
            RemoteViewNodeKind::Row,
            [],
        ),
        node(
            "panel.icon",
            Some("panel.header"),
            RemoteViewNodeKind::Icon("icons/ai_open_ai.svg".to_string()),
            [],
        ),
        node(
            "panel.title",
            Some("panel.header"),
            RemoteViewNodeKind::Text("ChatGPT Codex Usage".to_string()),
            [],
        ),
        node(
            "panel.badge",
            Some("panel.header"),
            RemoteViewNodeKind::Badge(view.snapshot.status_badge()),
            [],
        ),
        node(
            "panel.fill",
            Some("panel.header"),
            RemoteViewNodeKind::Spacer,
            [],
        ),
        node(
            "panel.refresh",
            Some("panel.header"),
            RemoteViewNodeKind::Button("Refresh".to_string()),
            [],
        ),
        node(
            "panel.auth",
            Some("panel.header"),
            RemoteViewNodeKind::Button(
                view.snapshot
                    .auth_button_label(view.sidecar_error.as_deref())
                    .to_string(),
            ),
            [],
        ),
        node(
            "panel.copy-status",
            Some("panel.header"),
            RemoteViewNodeKind::Button("Copy".to_string()),
            [],
        ),
        node(
            "panel.open-url",
            Some("panel.header"),
            RemoteViewNodeKind::Button("ChatGPT".to_string()),
            [],
        ),
        node(
            "panel.summary",
            Some("panel.root"),
            RemoteViewNodeKind::Text(snapshot_status_text(
                &view.snapshot,
                view.sidecar_error.as_deref(),
            )),
            [],
        ),
        node(
            "panel.primary.label",
            Some("panel.root"),
            RemoteViewNodeKind::Text(view.snapshot.primary_window_label.clone()),
            [],
        ),
        node(
            "panel.primary.progress",
            Some("panel.root"),
            RemoteViewNodeKind::ProgressBar(ProgressBarProps {
                value: view.snapshot.primary_used_percent,
                max_value: 100,
            }),
            [],
        ),
        node(
            "panel.secondary.label",
            Some("panel.root"),
            RemoteViewNodeKind::Text(view.snapshot.secondary_window_label.clone()),
            [],
        ),
        node(
            "panel.secondary.progress",
            Some("panel.root"),
            RemoteViewNodeKind::ProgressBar(ProgressBarProps {
                value: view.snapshot.secondary_used_percent,
                max_value: 100,
            }),
            [],
        ),
        node(
            "panel.rule",
            Some("panel.root"),
            RemoteViewNodeKind::Divider,
            [],
        ),
        node(
            "panel.scroll",
            Some("panel.root"),
            RemoteViewNodeKind::ScrollView,
            [],
        ),
        node(
            "panel.scroll.body",
            Some("panel.scroll"),
            RemoteViewNodeKind::Column,
            [],
        ),
        node(
            "panel.status",
            Some("panel.scroll.body"),
            RemoteViewNodeKind::Text(view.snapshot.status_label.clone()),
            [],
        ),
        node(
            "panel.detail",
            Some("panel.scroll.body"),
            RemoteViewNodeKind::Text(view.snapshot.detail.clone()),
            [],
        ),
    ];

    if let Some(account_label) = &view.snapshot.account_label {
        nodes.push(node(
            "panel.account",
            Some("panel.scroll.body"),
            RemoteViewNodeKind::Text(account_label.clone()),
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
    let (snapshot, sidecar_error) = load_sidecar_snapshot(force_refresh);
    view.snapshot = snapshot;
    view.sidecar_error = sidecar_error;
    view.revision = view.revision.saturating_add(1);
}

fn begin_sidecar_login() -> zed::Result<(), String> {
    zed::sidecar::call("auth.begin-login", None, Some(5_000)).map(|_| ())
}

fn refresh_sidecar_usage() -> zed::Result<(), String> {
    zed::sidecar::call("usage.refresh", None, Some(15_000)).map(|_| ())
}

fn logout_sidecar() -> zed::Result<(), String> {
    zed::sidecar::call("auth.logout", None, Some(5_000)).map(|_| ())
}

fn load_sidecar_snapshot(force_refresh: bool) -> (SidecarSnapshot, Option<String>) {
    let method = if force_refresh {
        "usage.refresh"
    } else {
        "usage.snapshot"
    };

    match zed::sidecar::call(method, None, Some(15_000)).and_then(parse_sidecar_snapshot) {
        Ok(snapshot) => (snapshot, None),
        Err(error) => (SidecarSnapshot::signed_out(), Some(error)),
    }
}

fn parse_sidecar_snapshot(value: zed::serde_json::Value) -> zed::Result<SidecarSnapshot, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "sidecar snapshot must be a JSON object".to_string())?;

    let auth_status = required_string(object, "auth_status")?;
    let status_label = required_string(object, "status_label")?;
    let detail = required_string(object, "detail")?;
    let primary_window_label = required_string(object, "primary_window_label")?;
    let secondary_window_label = required_string(object, "secondary_window_label")?;

    Ok(SidecarSnapshot {
        auth_status,
        status_label,
        detail,
        plan_type: optional_string(object, "plan_type"),
        account_label: optional_string(object, "account_label"),
        primary_window_label,
        secondary_window_label,
        primary_used_percent: optional_u32(object, "primary_used_percent")
            .unwrap_or(0)
            .min(100),
        secondary_used_percent: optional_u32(object, "secondary_used_percent")
            .unwrap_or(0)
            .min(100),
        busy: optional_bool(object, "busy").unwrap_or(false),
    })
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

fn optional_bool(
    object: &zed::serde_json::Map<String, zed::serde_json::Value>,
    field_name: &str,
) -> Option<bool> {
    object
        .get(field_name)
        .and_then(zed::serde_json::Value::as_bool)
}

fn snapshot_status_text(snapshot: &SidecarSnapshot, sidecar_error: Option<&str>) -> String {
    let mut status = format!(
        "{} · {}% / {}%",
        snapshot.status_label, snapshot.primary_used_percent, snapshot.secondary_used_percent
    );

    if let Some(plan_type) = snapshot.plan_type.as_deref() {
        status.push_str(&format!(" · {plan_type}"));
    }

    if snapshot.busy {
        status.push_str(" · syncing");
    }

    if let Some(sidecar_error) = sidecar_error {
        status.push_str(&format!(" · {sidecar_error}"));
    }

    status
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

zed::register_extension!(RemoteUiExampleExtension);
