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
    plan_type: Option<String>,
    primary_used_percent: u32,
    secondary_used_percent: u32,
    usage_limits: Vec<UsageLimitCard>,
    credits_summary: CreditsSummary,
}

#[derive(Clone)]
struct UsageLimitCard {
    id: String,
    title: String,
    remaining_percent: u32,
    resets_at_label: Option<String>,
}

#[derive(Clone)]
struct CreditsSummary {
    balance_label: String,
    detail: String,
}

impl SidecarSnapshot {
    fn signed_out() -> Self {
        Self {
            auth_status: "signed-out".to_string(),
            plan_type: None,
            primary_used_percent: 0,
            secondary_used_percent: 0,
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
        secondary_used_percent: optional_u32(object, "secondary_used_percent")
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
                secondary_used_percent: 71,
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
            "secondary_used_percent": 71,
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
        assert_eq!(snapshot.secondary_used_percent, 71);
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
}

zed::register_extension!(RemoteUiExampleExtension);
