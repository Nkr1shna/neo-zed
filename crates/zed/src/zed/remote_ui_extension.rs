use std::{fs, sync::Arc, time::Duration};

use anyhow::{Context as _, Result, anyhow, bail};
use collections::HashMap;
use command_palette::{
    CommandInterceptItem, CommandInterceptResult, GlobalCommandPaletteInterceptor,
    normalize_action_query,
};
use editor::{Editor, EditorContextMenuHooks};
use extension::{
    CommandContext, ContextActionTarget, DockSide, EventOutcome, Extension, ExtensionHostProxy,
    ExtensionMenuManifestEntry, ExtensionPanelManifestEntry, ExtensionRemoteUiProxy,
    FooterWidgetManifestEntry, FooterWidgetZone, HostMutation, MenuLocation, MountContext,
    MountKind, RemoteUiManifest, RemoteViewEvent, RemoteViewEventKind, RemoteViewNode,
    RemoteViewNodeKind, RemoteViewTree, RenderReason, TitlebarWidgetManifestEntry, WidgetSide,
    WidgetSize,
};
use extension_host::ExtensionStore;
use gpui::{
    Action, AnyElement, App, AppContext as _, BorrowAppContext as _, ClipboardItem, Context,
    Entity, EventEmitter, FocusHandle, Focusable, Global, InteractiveElement, IntoElement,
    MouseButton, Pixels, Render, SharedString, Styled, Task, WeakEntity, Window, prelude::*, px,
};
use project_panel::ProjectPanelContextMenuHooks;
use schemars::JsonSchema;
use serde::Deserialize;
use theme::ActiveTheme;
use title_bar::TitleBar;
use ui::{
    Button, ButtonCommon, Clickable, ContextMenu, ContextMenuEntry, ContextMenuItem, Divider, Icon,
    IconButton, IconName, IconSize, PopoverMenu, ProgressBar, Tooltip,
    utils::platform_title_bar_height, v_flex,
};
use util::ResultExt as _;
use workspace::dock::{DockPosition, PanelEvent, PanelHandle};
use workspace::notifications::NotificationId;
use workspace::{
    AppState, ItemHandle, ItemTabContextMenuHooks, Panel, PanelOverflowContextMenuHooks,
    StatusItemView, Toast, Workspace, WorkspaceId,
};

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema, gpui::Action)]
#[action(namespace = extensions)]
pub struct RunRegisteredCommand {
    pub command_id: String,
    pub input_json: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema, gpui::Action)]
#[action(namespace = extensions)]
pub struct ToggleRemoteUiPanel {
    pub panel_id: String,
}

#[derive(Default)]
struct GlobalRemoteUiRegistry(RemoteUiRegistry);

impl Global for GlobalRemoteUiRegistry {}

#[derive(Default)]
struct RemoteUiRegistry {
    next_panel_activation_priority: u32,
    extensions: HashMap<Arc<str>, RegisteredRemoteUiExtension>,
    commands: HashMap<Arc<str>, RegisteredRemoteUiCommand>,
    command_palette_items: Vec<RegisteredRemoteUiPaletteItem>,
    panels: HashMap<Arc<str>, RegisteredRemoteUiPanel>,
    mounted_panels: HashMap<(u64, Arc<str>), WeakEntity<RemoteUiPanel>>,
    mounted_widget_strips: HashMap<WidgetStripKey, WeakEntity<RemoteUiWidgetStrip>>,
}

#[derive(Clone)]
struct RegisteredRemoteUiExtension {
    remote_ui: RemoteUiManifest,
    qualified_command_ids: Vec<Arc<str>>,
    qualified_panel_ids: Vec<Arc<str>>,
}

#[derive(Clone)]
struct RegisteredRemoteUiCommand {
    extension_id: Arc<str>,
    local_command_id: Arc<str>,
    title: SharedString,
}

#[derive(Clone)]
struct RegisteredRemoteUiPaletteItem {
    command_id: Arc<str>,
    title: SharedString,
    search_text: Arc<str>,
}

#[derive(Clone)]
struct RegisteredRemoteUiContextMenuEntry {
    command_id: Arc<str>,
    title: SharedString,
    priority: u32,
}

#[derive(Clone)]
struct RegisteredRemoteUiPanel {
    qualified_panel_id: Arc<str>,
    extension_id: Arc<str>,
    root_view: Arc<str>,
    title: SharedString,
    default_dock: DockPosition,
    activation_priority: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum WidgetSurface {
    Titlebar,
    Footer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum StripSide {
    Left,
    Center,
    Right,
}

#[derive(Clone)]
struct RegisteredRemoteUiWidget {
    qualified_widget_id: Arc<str>,
    extension_id: Arc<str>,
    root_view: Arc<str>,
    surface: WidgetSurface,
    side: StripSide,
    size: WidgetSize,
    priority: u32,
    min_width: Option<u32>,
    max_width: Option<u32>,
    refresh_interval_seconds: Option<u32>,
}

#[derive(Clone)]
struct MountedRemoteUiWidget {
    descriptor: RegisteredRemoteUiWidget,
    entity: Entity<RemoteUiWidget>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct WidgetStripKey {
    workspace_id: u64,
    surface: WidgetSurface,
    side: StripSide,
}

struct RemoteUiProxy;

struct RemoteUiCommandError;

fn remote_ui_snapshot_key(workspace_id: u64, qualified_panel_id: &str) -> String {
    let sanitized_panel_id = qualified_panel_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("{workspace_id}_{sanitized_panel_id}.json")
}

fn remote_ui_panel_snapshot_path(
    workspace_id: u64,
    qualified_panel_id: &str,
) -> std::path::PathBuf {
    paths::data_dir()
        .join("remote_ui_snapshots")
        .join(remote_ui_snapshot_key(workspace_id, qualified_panel_id))
}

fn load_remote_ui_panel_snapshot(
    workspace_id: u64,
    qualified_panel_id: &str,
) -> Option<RemoteViewTree> {
    let snapshot_path = remote_ui_panel_snapshot_path(workspace_id, qualified_panel_id);
    let snapshot_json = match fs::read_to_string(&snapshot_path) {
        Ok(snapshot_json) => snapshot_json,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            Err(error)
                .with_context(|| {
                    format!(
                        "reading remote ui snapshot from {}",
                        snapshot_path.display()
                    )
                })
                .log_err()?;
            return None;
        }
    };
    serde_json::from_str(&snapshot_json).log_err()
}

fn save_remote_ui_panel_snapshot(
    workspace_id: u64,
    qualified_panel_id: Arc<str>,
    tree: RemoteViewTree,
) -> anyhow::Result<()> {
    let snapshot_path = remote_ui_panel_snapshot_path(workspace_id, qualified_panel_id.as_ref());
    let parent_dir = snapshot_path
        .parent()
        .context("remote ui snapshot path is missing a parent directory")?;
    fs::create_dir_all(parent_dir).context("creating remote ui snapshot directory")?;
    let snapshot_json = serde_json::to_string(&tree).context("serializing remote ui snapshot")?;
    fs::write(&snapshot_path, snapshot_json)
        .with_context(|| format!("writing remote ui snapshot to {}", snapshot_path.display()))?;
    Ok(())
}

pub fn init(cx: &mut App) {
    let proxy = ExtensionHostProxy::default_global(cx);
    proxy.register_remote_ui_proxy(RemoteUiProxy);
    EditorContextMenuHooks::set_named(cx, "remote-ui", remote_ui_editor_context_menu_hook);
    ItemTabContextMenuHooks::set_named(cx, "remote-ui", remote_ui_item_tab_context_menu_hook);
    ProjectPanelContextMenuHooks::set_named(
        cx,
        "remote-ui",
        remote_ui_project_panel_context_menu_hook,
    );
    PanelOverflowContextMenuHooks::set_named(cx, "remote-ui", remote_ui_panel_overflow_hook);
    GlobalCommandPaletteInterceptor::set_named(
        cx,
        "remote-ui",
        remote_ui_command_palette_interceptor,
    );

    cx.observe_new(|workspace: &mut Workspace, window, cx| {
        let Some(window) = window else {
            return;
        };

        mount_registered_panels_for_workspace(workspace, window, cx);
        mount_remote_ui_widget_strips_for_workspace(workspace, window, cx);

        workspace.register_action(|workspace, action: &RunRegisteredCommand, _window, cx| {
            let Some(workspace_id) = workspace.database_id().and_then(WorkspaceId::to_u64) else {
                let message = String::from(
                    "Failed to run extension command: workspace is missing a persisted identifier",
                );
                workspace.show_toast(
                    Toast::new(
                        NotificationId::unique::<RemoteUiCommandError>(),
                        message.clone(),
                    )
                    .autohide(),
                    cx,
                );
                log::error!("{message}");
                return;
            };

            if let Err(error) = dispatch_registered_command_from_workspace(
                cx.entity(),
                workspace_id,
                action.command_id.as_str(),
                action.input_json.clone(),
                cx,
            ) {
                let message = format!("Failed to run extension command: {error}");
                workspace.show_toast(
                    Toast::new(
                        NotificationId::unique::<RemoteUiCommandError>(),
                        message.clone(),
                    )
                    .autohide(),
                    cx,
                );
                log::error!("{message}");
            }
        });

        workspace.register_action(|workspace, action: &ToggleRemoteUiPanel, window, cx| {
            let Some(workspace_id) = workspace.database_id().and_then(WorkspaceId::to_u64) else {
                return;
            };

            if let Err(error) = toggle_panel_visibility(
                workspace,
                window,
                workspace_id,
                action.panel_id.as_str(),
                cx,
            ) {
                let message = format!("Failed to toggle extension panel: {error}");
                workspace.show_toast(
                    Toast::new(
                        NotificationId::unique::<RemoteUiCommandError>(),
                        message.clone(),
                    )
                    .autohide(),
                    cx,
                );
                log::error!("{message}");
            }
        });
    })
    .detach();
}

impl RemoteUiRegistry {
    fn read_global(cx: &App) -> Option<&Self> {
        cx.try_global::<GlobalRemoteUiRegistry>()
            .map(|registry| &registry.0)
    }

    fn update_global<T>(cx: &mut App, update: impl FnOnce(&mut Self) -> T) -> T {
        cx.update_default_global(|registry: &mut GlobalRemoteUiRegistry, _cx| {
            update(&mut registry.0)
        })
    }

    fn register_extension(
        &mut self,
        extension_id: Arc<str>,
        remote_ui: RemoteUiManifest,
    ) -> Vec<RegisteredRemoteUiPanel> {
        self.unregister_extension(extension_id.as_ref());

        let mut qualified_command_ids = Vec::with_capacity(remote_ui.commands.len());
        for (command_id, command) in &remote_ui.commands {
            let qualified_command_id = qualified_remote_ui_id(extension_id.as_ref(), command_id);
            self.commands.insert(
                qualified_command_id.clone(),
                RegisteredRemoteUiCommand {
                    extension_id: extension_id.clone(),
                    local_command_id: command_id.clone(),
                    title: command.title.clone().into(),
                },
            );
            qualified_command_ids.push(qualified_command_id);
        }

        let mut qualified_panel_ids = Vec::with_capacity(remote_ui.panels.len());
        let mut registered_panels = Vec::with_capacity(remote_ui.panels.len());
        for (panel_id, panel) in &remote_ui.panels {
            let qualified_panel_id = qualified_remote_ui_id(extension_id.as_ref(), panel_id);
            if panel.default_size.is_some() {
                log::warn!(
                    "extension panel `{qualified_panel_id}` uses deprecated `default_size`; dock sizing is host-owned and the value is ignored"
                );
            }
            let registered_panel = RegisteredRemoteUiPanel {
                qualified_panel_id: qualified_panel_id.clone(),
                extension_id: extension_id.clone(),
                root_view: panel.root_view.clone().into(),
                title: panel.title.clone().into(),
                default_dock: dock_position_for_panel(panel),
                activation_priority: self.next_panel_activation_priority,
            };
            self.next_panel_activation_priority = self
                .next_panel_activation_priority
                .checked_add(1)
                .unwrap_or(1_000_000);

            self.panels
                .insert(qualified_panel_id.clone(), registered_panel.clone());
            qualified_panel_ids.push(qualified_panel_id);
            registered_panels.push(registered_panel);
        }

        self.extensions.insert(
            extension_id,
            RegisteredRemoteUiExtension {
                remote_ui,
                qualified_command_ids,
                qualified_panel_ids,
            },
        );

        self.rebuild_command_palette_items();

        registered_panels
    }

    fn unregister_extension(&mut self, extension_id: &str) -> Vec<Arc<str>> {
        let Some(extension) = self.extensions.remove(extension_id) else {
            return Vec::new();
        };

        for qualified_command_id in extension.qualified_command_ids {
            self.commands.remove(&qualified_command_id);
        }

        for qualified_panel_id in &extension.qualified_panel_ids {
            self.panels.remove(qualified_panel_id);
        }

        self.rebuild_command_palette_items();

        extension.qualified_panel_ids
    }

    fn rebuild_command_palette_items(&mut self) {
        let mut command_palette_items = Vec::new();

        for (extension_id, extension) in &self.extensions {
            for (command_id, command) in &extension.remote_ui.commands {
                if command.palette {
                    command_palette_items.push(RegisteredRemoteUiPaletteItem {
                        command_id: qualified_remote_ui_id(extension_id, command_id),
                        title: command.title.clone().into(),
                        search_text: palette_search_text(
                            extension_id,
                            command_id,
                            &command.title,
                            Some(command.description.as_str()),
                        ),
                    });
                }
            }

            for menu in extension
                .remote_ui
                .menus
                .iter()
                .filter(|menu| menu.location == MenuLocation::CommandPalette)
            {
                if let Some(item) = palette_item_for_menu(extension_id, &extension.remote_ui, menu)
                {
                    command_palette_items.push(item);
                }
            }
        }

        command_palette_items.sort_by(|left, right| left.title.cmp(&right.title));
        self.command_palette_items = command_palette_items;
    }

    fn remember_mounted_panel(
        &mut self,
        workspace_id: u64,
        panel_id: Arc<str>,
        panel: WeakEntity<RemoteUiPanel>,
    ) {
        self.mounted_panels.insert((workspace_id, panel_id), panel);
    }

    fn forget_mounted_panel(&mut self, workspace_id: u64, panel_id: &str) {
        self.mounted_panels
            .remove(&(workspace_id, Arc::from(panel_id)));
    }

    fn widget_descriptors(
        &self,
        surface: WidgetSurface,
        side: StripSide,
    ) -> Vec<RegisteredRemoteUiWidget> {
        let mut descriptors = Vec::new();

        for (extension_id, extension) in &self.extensions {
            match surface {
                WidgetSurface::Titlebar => {
                    for widget in extension
                        .remote_ui
                        .titlebar_widgets
                        .iter()
                        .filter(|widget| strip_side_for_titlebar_widget(widget.side) == side)
                    {
                        descriptors.push(registered_titlebar_widget(extension_id, widget));
                    }
                }
                WidgetSurface::Footer => {
                    for widget in extension
                        .remote_ui
                        .footer_widgets
                        .iter()
                        .filter(|widget| strip_side_for_footer_widget(widget.zone) == side)
                    {
                        descriptors.push(registered_footer_widget(extension_id, widget));
                    }
                }
            }
        }

        descriptors.sort_by_key(|descriptor| descriptor.priority);
        descriptors
    }

    fn context_menu_entries(
        &self,
        menu_location: MenuLocation,
        action_target: ContextActionTarget,
        current_panel_qualified_id: Option<&str>,
    ) -> Vec<RegisteredRemoteUiContextMenuEntry> {
        let mut entries = Vec::new();

        for (extension_id, extension) in &self.extensions {
            for menu in extension
                .remote_ui
                .menus
                .iter()
                .filter(|menu| menu.location == menu_location)
            {
                if let Some(panel_id) = &menu.panel {
                    let qualified_panel_id = qualified_remote_ui_id(extension_id, panel_id);
                    if current_panel_qualified_id != Some(qualified_panel_id.as_ref()) {
                        continue;
                    }
                }

                entries.push(RegisteredRemoteUiContextMenuEntry {
                    command_id: qualified_remote_ui_id(extension_id, &menu.command),
                    title: menu.title.clone().into(),
                    priority: menu.priority,
                });
            }

            for action in extension
                .remote_ui
                .context_actions
                .iter()
                .filter(|action| action.target == action_target)
            {
                entries.push(RegisteredRemoteUiContextMenuEntry {
                    command_id: qualified_remote_ui_id(extension_id, &action.command),
                    title: action.title.clone().into(),
                    priority: action.priority,
                });
            }
        }

        entries.sort_by(|left, right| {
            left.priority
                .cmp(&right.priority)
                .then_with(|| left.title.cmp(&right.title))
        });
        entries
    }

    fn remember_widget_strip(
        &mut self,
        key: WidgetStripKey,
        strip: WeakEntity<RemoteUiWidgetStrip>,
    ) {
        self.mounted_widget_strips.insert(key, strip);
    }

    fn mounted_widget_strip(&self, key: WidgetStripKey) -> Option<WeakEntity<RemoteUiWidgetStrip>> {
        self.mounted_widget_strips.get(&key).cloned()
    }
}

impl ExtensionRemoteUiProxy for RemoteUiProxy {
    fn register_remote_ui_extension(
        &self,
        extension_id: Arc<str>,
        remote_ui: RemoteUiManifest,
        cx: &mut App,
    ) {
        let panels = RemoteUiRegistry::update_global(cx, |registry| {
            registry.register_extension(extension_id, remote_ui)
        });
        cx.defer(move |cx| {
            mount_remote_ui_panels_in_all_workspaces(panels, cx);
            refresh_remote_ui_widget_strips_in_all_workspaces(cx);
        });
    }

    fn unregister_remote_ui_extension(&self, extension_id: Arc<str>, cx: &mut App) {
        let removed_panel_ids = RemoteUiRegistry::update_global(cx, |registry| {
            registry.unregister_extension(extension_id.as_ref())
        });
        cx.defer(move |cx| {
            unmount_remote_ui_panels_in_all_workspaces(&removed_panel_ids, cx);
            refresh_remote_ui_widget_strips_in_all_workspaces(cx);
        });
    }

    fn dispatch_remote_ui_workspace_action(
        &self,
        workspace_id: u64,
        action_id: &str,
        payload_json: Option<&str>,
        cx: &mut App,
    ) -> Result<()> {
        let (window_handle, _) = workspace_for_id(workspace_id, cx)?;
        let action_payload: Option<serde_json::Value> = payload_json
            .map(serde_json::from_str)
            .transpose()
            .context("invalid workspace action payload")?;
        let action = cx
            .build_action(action_id, action_payload)
            .map_err(|error| anyhow!("{error}"))?;

        window_handle
            .update(cx, |_, window, cx| {
                window.dispatch_action(action, cx);
            })
            .map_err(|error| anyhow!("{error}"))?;

        Ok(())
    }

    fn dispatch_remote_ui_command(
        &self,
        workspace_id: u64,
        command_id: &str,
        input_json: Option<&str>,
        cx: &mut App,
    ) -> Result<()> {
        let (_, workspace) = workspace_for_id(workspace_id, cx)?;
        dispatch_registered_command_from_workspace(
            workspace,
            workspace_id,
            command_id,
            input_json.map(ToOwned::to_owned),
            cx,
        )
    }

    fn request_remote_ui_host_mutation(
        &self,
        extension_id: &str,
        workspace_id: u64,
        mutation: HostMutation,
        cx: &mut App,
    ) -> Result<()> {
        let (window_handle, workspace) = workspace_for_id(workspace_id, cx)?;
        match mutation {
            HostMutation::ShowToast(message) => {
                workspace.update(cx, |workspace, cx| {
                    workspace.show_toast(
                        Toast::new(NotificationId::unique::<RemoteUiCommandError>(), message)
                            .autohide(),
                        cx,
                    );
                });
            }
            HostMutation::CopyToClipboard(contents) => {
                cx.write_to_clipboard(ClipboardItem::new_string(contents));
            }
            HostMutation::OpenExternalUrl(url) => {
                cx.open_url(&url);
            }
            HostMutation::OpenPanel(panel_id) => {
                let qualified_panel_id = qualified_remote_ui_id(extension_id, panel_id.as_str());
                set_panel_open_state(
                    window_handle,
                    workspace,
                    workspace_id,
                    qualified_panel_id.as_ref(),
                    true,
                    cx,
                )?;
            }
            HostMutation::TogglePanel(panel_id) => {
                let qualified_panel_id = qualified_remote_ui_id(extension_id, panel_id.as_str());
                let _ = window_handle.update(cx, |_, window, cx| {
                    workspace.update(cx, |workspace, cx| {
                        toggle_panel_visibility(
                            workspace,
                            window,
                            workspace_id,
                            qualified_panel_id.as_ref(),
                            cx,
                        )
                    })
                })?;
            }
            HostMutation::ClosePanel(panel_id) => {
                let qualified_panel_id = qualified_remote_ui_id(extension_id, panel_id.as_str());
                set_panel_open_state(
                    window_handle,
                    workspace,
                    workspace_id,
                    qualified_panel_id.as_ref(),
                    false,
                    cx,
                )?;
            }
        }

        Ok(())
    }
}

struct RemoteUiPanel {
    descriptor: RegisteredRemoteUiPanel,
    workspace: WeakEntity<Workspace>,
    workspace_id: u64,
    focus_handle: FocusHandle,
    instance_id: Option<u64>,
    tree: Option<RemoteViewTree>,
    loading: bool,
    error_message: Option<SharedString>,
    position: DockPosition,
}

impl EventEmitter<PanelEvent> for RemoteUiPanel {}

impl RemoteUiPanel {
    fn new(
        descriptor: RegisteredRemoteUiPanel,
        workspace: WeakEntity<Workspace>,
        workspace_id: u64,
        cx: &mut Context<Self>,
    ) -> Self {
        let qualified_panel_id = descriptor.qualified_panel_id.clone();
        let mut this = Self {
            position: descriptor.default_dock,
            descriptor,
            workspace,
            workspace_id,
            focus_handle: cx.focus_handle(),
            instance_id: None,
            tree: load_remote_ui_panel_snapshot(workspace_id, qualified_panel_id.as_ref()),
            loading: false,
            error_message: None,
        };
        this.reload(RenderReason::Initial, cx);
        this
    }

    fn mount_context(&self) -> MountContext {
        MountContext {
            workspace_id: self.workspace_id,
            mount_kind: MountKind::Panel,
            trusted: false,
            active_item_kind: None,
            appearance: None,
        }
    }

    fn reload(&mut self, reason: RenderReason, cx: &mut Context<Self>) {
        let descriptor = self.descriptor.clone();
        let extension = match extension_for_id(descriptor.extension_id.as_ref(), cx) {
            Ok(extension) => extension,
            Err(error) => {
                self.loading = false;
                if is_extension_not_loaded_error(&error) {
                    self.error_message = None;
                } else {
                    let message = format!(
                        "Extension panel `{}` reload failed: {error}",
                        descriptor.qualified_panel_id
                    );
                    log::error!("{message}");
                    self.error_message = Some(message.into());
                }
                cx.notify();
                return;
            }
        };
        let mount_context = self.mount_context();
        let existing_instance_id = self.instance_id;

        self.loading = true;
        self.error_message = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = async {
                let instance_id = match existing_instance_id {
                    Some(instance_id) => instance_id,
                    None => {
                        extension
                            .open_view(descriptor.root_view.clone(), mount_context.clone())
                            .await?
                    }
                };
                let tree = extension
                    .render_view(instance_id, mount_context, reason)
                    .await?;
                Ok::<_, anyhow::Error>((instance_id, tree))
            }
            .await;

            this.update(cx, |panel, cx| {
                panel.loading = false;
                match result {
                    Ok((instance_id, tree)) => {
                        panel.instance_id = Some(instance_id);
                        panel.tree = Some(tree.clone());
                        panel.error_message = None;
                        let workspace_id = panel.workspace_id;
                        let qualified_panel_id = panel.descriptor.qualified_panel_id.clone();
                        cx.background_spawn(async move {
                            save_remote_ui_panel_snapshot(workspace_id, qualified_panel_id, tree)
                                .log_err();
                        })
                        .detach();
                    }
                    Err(error) => {
                        let message = format!(
                            "Extension panel `{}` reload failed: {error}",
                            descriptor.qualified_panel_id
                        );
                        log::error!("{message}");
                        panel.error_message = Some(message.into());
                    }
                }
                cx.notify();
            })?;

            Ok::<(), anyhow::Error>(())
        })
        .detach_and_log_err(cx);
    }

    fn handle_remote_event(
        &mut self,
        event: RemoteViewEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(instance_id) = self.instance_id else {
            return;
        };

        let descriptor = self.descriptor.clone();
        let extension = match extension_for_id(descriptor.extension_id.as_ref(), cx) {
            Ok(extension) => extension,
            Err(error) => {
                let message = format!(
                    "Extension panel `{}` event setup failed: {error}",
                    descriptor.qualified_panel_id
                );
                log::error!("{message}");
                self.error_message = Some(message.into());
                cx.notify();
                return;
            }
        };
        let mount_context = self.mount_context();
        let workspace = self.workspace.clone();

        cx.spawn_in(window, async move |this, cx| {
            let result = extension
                .handle_view_event(instance_id, mount_context.clone(), event)
                .await;

            match result {
                Ok(EventOutcome::Noop) => {}
                Ok(EventOutcome::Rerender) | Ok(EventOutcome::RerenderVirtualRange(_)) => {
                    this.update(cx, |panel, cx| {
                        panel.reload(RenderReason::Event, cx);
                    })?;
                }
                Ok(EventOutcome::ShowError(message)) => {
                    workspace
                        .update(cx, |workspace, cx| {
                            workspace.show_toast(
                                Toast::new(
                                    NotificationId::unique::<RemoteUiCommandError>(),
                                    message,
                                )
                                .autohide(),
                                cx,
                            );
                        })
                        .ok();
                }
                Err(error) => {
                    let message = format!(
                        "Extension panel `{}` event failed: {error}",
                        descriptor.title
                    );
                    log::error!("{message}");
                    this.update(cx, |panel, cx| {
                        panel.error_message = Some(message.clone().into());
                        cx.notify();
                    })?;
                }
            }

            Ok::<(), anyhow::Error>(())
        })
        .detach_and_log_err(cx);
    }

    fn render_content(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let body = if let Some(error_message) = &self.error_message {
            v_flex()
                .gap_2()
                .p_2()
                .child(error_message.clone())
                .into_any_element()
        } else if self.loading && self.tree.is_none() {
            v_flex()
                .gap_2()
                .p_2()
                .child("Loading extension panel...")
                .into_any_element()
        } else {
            let Some(tree) = self.tree.clone() else {
                return self.render_panel_frame(
                    "Extension panel has no content.".into_any_element(),
                    window,
                    cx,
                );
            };

            let mut nodes = HashMap::default();
            let mut children_by_parent: HashMap<Option<String>, Vec<String>> = HashMap::default();
            for node in tree.nodes {
                let node_id = node.node_id.clone();
                children_by_parent
                    .entry(node.parent_id.clone())
                    .or_default()
                    .push(node_id.clone());
                nodes.insert(node_id, node);
            }

            self.render_remote_view_node(
                tree.root_id.as_str(),
                &nodes,
                &children_by_parent,
                false,
                window,
                cx,
            )
        };

        self.render_panel_frame(body, window, cx)
    }

    fn render_panel_frame(
        &mut self,
        body: AnyElement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let menu = build_remote_ui_panel_overflow_menu(
            self.workspace.clone(),
            self.descriptor.qualified_panel_id.clone(),
            window,
            cx,
        );

        v_flex()
            .size_full()
            .child(
                ui::h_flex()
                    .justify_between()
                    .items_center()
                    .px_2()
                    .py_1()
                    .child(self.descriptor.title.clone())
                    .children(menu.into_iter().map(|_| {
                        let workspace = self.workspace.clone();
                        let panel_id = self.descriptor.qualified_panel_id.clone();
                        let popover_id = format!("remote-ui-panel-menu-{}", panel_id);
                        let button_id = format!("remote-ui-panel-menu-button-{}", panel_id);
                        PopoverMenu::new(popover_id)
                            .menu(move |window, cx| {
                                build_remote_ui_panel_overflow_menu(
                                    workspace.clone(),
                                    panel_id.clone(),
                                    window,
                                    cx,
                                )
                            })
                            .trigger_with_tooltip(
                                IconButton::new(button_id, IconName::Ellipsis)
                                    .icon_size(IconSize::Small),
                                Tooltip::text("Panel actions"),
                            )
                    })),
            )
            .child(Divider::horizontal())
            .child(body)
            .into_any_element()
    }

    fn render_remote_view_node(
        &mut self,
        node_id: &str,
        nodes: &HashMap<String, RemoteViewNode>,
        children_by_parent: &HashMap<Option<String>, Vec<String>>,
        ancestor_is_clickable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(node) = nodes.get(node_id) else {
            return "Missing remote UI node".into_any_element();
        };

        let rendered_children = children_by_parent
            .get(&Some(node.node_id.clone()))
            .into_iter()
            .flatten()
            .map(|child_id| {
                self.render_remote_view_node(
                    child_id.as_str(),
                    nodes,
                    children_by_parent,
                    ancestor_is_clickable || node_is_clickable(node),
                    window,
                    cx,
                )
            })
            .collect::<Vec<_>>();

        match &node.kind {
            RemoteViewNodeKind::Row => {
                let node_id = node.node_id.clone();
                let gap = node_gap(node, 8.);
                let tooltip = node_tooltip(node);
                if node_is_clickable(node) {
                    ui::h_flex()
                        .id(node_id.clone())
                        .gap(gap)
                        .children(rendered_children)
                        .rounded_sm()
                        .cursor_pointer()
                        .hover(|style| style.bg(cx.theme().colors().ghost_element_hover))
                        .active(|style| style.bg(cx.theme().colors().ghost_element_active))
                        .when_some(tooltip, |this, tooltip| {
                            this.tooltip(Tooltip::text(tooltip))
                        })
                        .on_click(cx.listener(move |panel, _, window, cx| {
                            panel.handle_remote_event(
                                RemoteViewEvent {
                                    node_id: node_id.clone(),
                                    kind: RemoteViewEventKind::Click,
                                    payload_json: None,
                                },
                                window,
                                cx,
                            );
                        }))
                        .into_any_element()
                } else {
                    ui::h_flex()
                        .gap(gap)
                        .children(rendered_children)
                        .into_any_element()
                }
            }
            RemoteViewNodeKind::Column => {
                let node_id = node.node_id.clone();
                let gap = node_gap(node, 8.);
                if node_is_clickable(node) {
                    v_flex()
                        .id(node_id.clone())
                        .gap(gap)
                        .children(rendered_children)
                        .rounded_sm()
                        .cursor_pointer()
                        .hover(|style| style.bg(cx.theme().colors().ghost_element_hover))
                        .active(|style| style.bg(cx.theme().colors().ghost_element_active))
                        .on_click(cx.listener(move |panel, _, window, cx| {
                            panel.handle_remote_event(
                                RemoteViewEvent {
                                    node_id: node_id.clone(),
                                    kind: RemoteViewEventKind::Click,
                                    payload_json: None,
                                },
                                window,
                                cx,
                            );
                        }))
                        .into_any_element()
                } else {
                    v_flex()
                        .gap(gap)
                        .children(rendered_children)
                        .into_any_element()
                }
            }
            RemoteViewNodeKind::Stack => {
                let node_id = node.node_id.clone();
                let gap = node_gap(node, 4.);
                if node_is_clickable(node) {
                    v_flex()
                        .id(node_id.clone())
                        .gap(gap)
                        .children(rendered_children)
                        .rounded_sm()
                        .cursor_pointer()
                        .hover(|style| style.bg(cx.theme().colors().ghost_element_hover))
                        .active(|style| style.bg(cx.theme().colors().ghost_element_active))
                        .on_click(cx.listener(move |panel, _, window, cx| {
                            panel.handle_remote_event(
                                RemoteViewEvent {
                                    node_id: node_id.clone(),
                                    kind: RemoteViewEventKind::Click,
                                    payload_json: None,
                                },
                                window,
                                cx,
                            );
                        }))
                        .into_any_element()
                } else {
                    v_flex()
                        .gap(gap)
                        .children(rendered_children)
                        .into_any_element()
                }
            }
            RemoteViewNodeKind::Text(text) => text.clone().into_any_element(),
            RemoteViewNodeKind::Icon(icon) => {
                let node_id = node.node_id.clone();
                ui::ButtonLike::new(node_id.clone())
                    .style(ui::ButtonStyle::Transparent)
                    .size(ui::ButtonSize::Compact)
                    .child(Icon::from_path(icon.clone()))
                    .on_click(cx.listener(move |panel, _, window, cx| {
                        panel.handle_remote_event(
                            RemoteViewEvent {
                                node_id: node_id.clone(),
                                kind: RemoteViewEventKind::Click,
                                payload_json: None,
                            },
                            window,
                            cx,
                        );
                    }))
                    .into_any_element()
            }
            RemoteViewNodeKind::Button(label) => {
                let node_id = node.node_id.clone();
                Button::new(node_id.clone(), label.clone())
                    .on_click(cx.listener(move |panel, _, window, cx| {
                        panel.handle_remote_event(
                            RemoteViewEvent {
                                node_id: node_id.clone(),
                                kind: RemoteViewEventKind::Click,
                                payload_json: None,
                            },
                            window,
                            cx,
                        );
                    }))
                    .into_any_element()
            }
            RemoteViewNodeKind::Toggle(value) => {
                format!("Toggle: {}", if *value { "on" } else { "off" }).into_any_element()
            }
            RemoteViewNodeKind::Checkbox(value) => {
                format!("Checkbox: {}", if *value { "checked" } else { "unchecked" })
                    .into_any_element()
            }
            RemoteViewNodeKind::TextInput(value) => value.clone().into_any_element(),
            RemoteViewNodeKind::Badge(value) => {
                let node_id = node.node_id.clone();
                ui::ButtonLike::new(node_id.clone())
                    .style(ui::ButtonStyle::Transparent)
                    .size(ui::ButtonSize::Compact)
                    .child(value.clone())
                    .on_click(cx.listener(move |panel, _, window, cx| {
                        panel.handle_remote_event(
                            RemoteViewEvent {
                                node_id: node_id.clone(),
                                kind: RemoteViewEventKind::Click,
                                payload_json: None,
                            },
                            window,
                            cx,
                        );
                    }))
                    .into_any_element()
            }
            RemoteViewNodeKind::ProgressBar(props) => ProgressBar::new(
                node.node_id.clone(),
                props.value as f32,
                props.max_value.max(1) as f32,
                cx,
            )
            .into_any_element(),
            RemoteViewNodeKind::Divider => Divider::horizontal().into_any_element(),
            RemoteViewNodeKind::Spacer => gpui::div().flex_grow().into_any_element(),
            RemoteViewNodeKind::ScrollView => {
                gpui::div().children(rendered_children).into_any_element()
            }
            RemoteViewNodeKind::VirtualList(_) => {
                "Virtual list is not implemented yet".into_any_element()
            }
        }
    }
}

impl Focusable for RemoteUiPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for RemoteUiPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        v_flex()
            .size_full()
            .track_focus(&self.focus_handle(cx))
            .p_2()
            .child(self.render_content(window, cx))
    }
}

impl Panel for RemoteUiPanel {
    fn persistent_name() -> &'static str {
        "RemoteUiPanel"
    }

    fn panel_key() -> &'static str {
        "RemoteUiPanel"
    }

    fn position(&self, _window: &Window, _cx: &App) -> DockPosition {
        self.position
    }

    fn position_is_valid(&self, _position: DockPosition) -> bool {
        true
    }

    fn set_position(
        &mut self,
        position: DockPosition,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.position = position;
        cx.notify();
    }

    fn icon(&self, _window: &Window, _cx: &App) -> Option<ui::IconName> {
        None
    }

    fn icon_tooltip(&self, _window: &Window, _cx: &App) -> Option<&'static str> {
        None
    }

    fn toggle_action(&self) -> Box<dyn gpui::Action> {
        ToggleRemoteUiPanel {
            panel_id: self.descriptor.qualified_panel_id.to_string(),
        }
        .boxed_clone()
    }

    fn activation_priority(&self) -> u32 {
        self.descriptor.activation_priority
    }
}

struct RemoteUiWidget {
    descriptor: RegisteredRemoteUiWidget,
    workspace: WeakEntity<Workspace>,
    workspace_id: u64,
    instance_id: Option<u64>,
    tree: Option<RemoteViewTree>,
    loading: bool,
    error_message: Option<SharedString>,
    refresh_task: Option<Task<anyhow::Result<()>>>,
}

impl RemoteUiWidget {
    fn new(
        descriptor: RegisteredRemoteUiWidget,
        workspace: WeakEntity<Workspace>,
        workspace_id: u64,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            descriptor,
            workspace,
            workspace_id,
            instance_id: None,
            tree: None,
            loading: false,
            error_message: None,
            refresh_task: None,
        };
        this.reload(RenderReason::Initial, cx);
        this.schedule_auto_refresh(cx);
        this
    }

    fn mount_context(&self) -> MountContext {
        MountContext {
            workspace_id: self.workspace_id,
            mount_kind: match self.descriptor.surface {
                WidgetSurface::Titlebar => MountKind::TitlebarWidget,
                WidgetSurface::Footer => MountKind::FooterWidget,
            },
            trusted: false,
            active_item_kind: None,
            appearance: None,
        }
    }

    fn reload(&mut self, reason: RenderReason, cx: &mut Context<Self>) {
        let descriptor = self.descriptor.clone();
        let extension = match extension_for_id(descriptor.extension_id.as_ref(), cx) {
            Ok(extension) => extension,
            Err(error) => {
                self.loading = false;
                if is_extension_not_loaded_error(&error) {
                    self.error_message = None;
                } else {
                    let message = format!(
                        "Extension widget `{}` reload failed: {error}",
                        descriptor.qualified_widget_id
                    );
                    log::error!("{message}");
                    self.error_message = Some(message.into());
                }
                cx.notify();
                return;
            }
        };
        let mount_context = self.mount_context();
        let existing_instance_id = self.instance_id;

        self.loading = true;
        self.error_message = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = async {
                let instance_id = match existing_instance_id {
                    Some(instance_id) => instance_id,
                    None => {
                        extension
                            .open_view(descriptor.root_view.clone(), mount_context.clone())
                            .await?
                    }
                };
                let tree = extension
                    .render_view(instance_id, mount_context, reason)
                    .await?;
                Ok::<_, anyhow::Error>((instance_id, tree))
            }
            .await;

            this.update(cx, |widget, cx| {
                widget.loading = false;
                match result {
                    Ok((instance_id, tree)) => {
                        widget.instance_id = Some(instance_id);
                        match validate_widget_tree(&widget.descriptor, &tree) {
                            Ok(()) => {
                                widget.tree = Some(tree);
                                widget.error_message = None;
                            }
                            Err(error) => {
                                let message = format!(
                                    "Extension widget `{}` validation failed: {error}",
                                    widget.descriptor.qualified_widget_id
                                );
                                log::error!("{message}");
                                widget.tree = None;
                                widget.error_message = Some(message.into());
                            }
                        }
                    }
                    Err(error) => {
                        let message = format!(
                            "Extension widget `{}` reload failed: {error}",
                            descriptor.qualified_widget_id
                        );
                        log::error!("{message}");
                        widget.error_message = Some(message.into());
                    }
                }
                cx.notify();
            })?;

            Ok::<(), anyhow::Error>(())
        })
        .detach_and_log_err(cx);
    }

    fn schedule_auto_refresh(&mut self, cx: &mut Context<Self>) {
        let Some(refresh_interval_seconds) = self.descriptor.refresh_interval_seconds else {
            return;
        };
        let interval = Duration::from_secs(refresh_interval_seconds as u64);
        self.refresh_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(interval).await;
                let Some(this) = this.upgrade() else {
                    break;
                };
                this.update(cx, |widget, cx| {
                    if !widget.loading {
                        widget.reload(RenderReason::ExplicitRefresh, cx);
                    }
                });
            }

            Ok::<(), anyhow::Error>(())
        }));
    }

    fn handle_remote_event(
        &mut self,
        event: RemoteViewEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        log::info!(
            "remote ui widget event widget_id={} workspace_id={} node_id={} kind={:?}",
            self.descriptor.qualified_widget_id,
            self.workspace_id,
            event.node_id,
            event.kind
        );
        let Some(instance_id) = self.instance_id else {
            return;
        };

        let descriptor = self.descriptor.clone();
        let extension = match extension_for_id(descriptor.extension_id.as_ref(), cx) {
            Ok(extension) => extension,
            Err(error) => {
                let message = format!(
                    "Extension widget `{}` event setup failed: {error}",
                    descriptor.qualified_widget_id
                );
                log::error!("{message}");
                self.error_message = Some(message.into());
                cx.notify();
                return;
            }
        };
        let mount_context = self.mount_context();
        let workspace = self.workspace.clone();

        cx.spawn_in(window, async move |this, cx| {
            let result = extension
                .handle_view_event(instance_id, mount_context.clone(), event)
                .await;

            match result {
                Ok(EventOutcome::Noop) => {}
                Ok(EventOutcome::Rerender) | Ok(EventOutcome::RerenderVirtualRange(_)) => {
                    this.update(cx, |widget, cx| {
                        widget.reload(RenderReason::Event, cx);
                    })?;
                }
                Ok(EventOutcome::ShowError(message)) => {
                    workspace
                        .update(cx, |workspace, cx| {
                            workspace.show_toast(
                                Toast::new(
                                    NotificationId::unique::<RemoteUiCommandError>(),
                                    message,
                                )
                                .autohide(),
                                cx,
                            );
                        })
                        .ok();
                }
                Err(error) => {
                    let message = format!(
                        "Extension widget `{}` event failed: {error}",
                        descriptor.qualified_widget_id
                    );
                    log::error!("{message}");
                    this.update(cx, |widget, cx| {
                        widget.error_message = Some(message.clone().into());
                        cx.notify();
                    })?;
                }
            }

            Ok::<(), anyhow::Error>(())
        })
        .detach_and_log_err(cx);
    }

    fn render_content(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if let Some(error_message) = &self.error_message {
            return error_message.clone().into_any_element();
        }

        if self.loading && self.tree.is_none() {
            return "…".into_any_element();
        }

        let Some(tree) = self.tree.clone() else {
            return gpui::div().into_any_element();
        };

        let mut nodes = HashMap::default();
        let mut children_by_parent: HashMap<Option<String>, Vec<String>> = HashMap::default();
        for node in tree.nodes {
            let node_id = node.node_id.clone();
            children_by_parent
                .entry(node.parent_id.clone())
                .or_default()
                .push(node_id.clone());
            nodes.insert(node_id, node);
        }

        self.render_remote_view_node(
            tree.root_id.as_str(),
            &nodes,
            &children_by_parent,
            false,
            window,
            cx,
        )
    }

    fn render_remote_view_node(
        &mut self,
        node_id: &str,
        nodes: &HashMap<String, RemoteViewNode>,
        children_by_parent: &HashMap<Option<String>, Vec<String>>,
        ancestor_is_clickable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(node) = nodes.get(node_id) else {
            return "Missing remote UI node".into_any_element();
        };

        let rendered_children = children_by_parent
            .get(&Some(node.node_id.clone()))
            .into_iter()
            .flatten()
            .map(|child_id| {
                self.render_remote_view_node(
                    child_id.as_str(),
                    nodes,
                    children_by_parent,
                    ancestor_is_clickable || node_is_clickable(node),
                    window,
                    cx,
                )
            })
            .collect::<Vec<_>>();

        match &node.kind {
            RemoteViewNodeKind::Row => {
                let node_id = node.node_id.clone();
                let gap = node_gap(node, 8.);
                let tooltip = node_tooltip(node);
                if node_is_clickable(node) {
                    ui::h_flex()
                        .id(node_id.clone())
                        .gap(gap)
                        .children(rendered_children)
                        .px_1()
                        .rounded_sm()
                        .cursor_pointer()
                        .hover(|style| style.bg(cx.theme().colors().ghost_element_hover))
                        .active(|style| style.bg(cx.theme().colors().ghost_element_active))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .when_some(tooltip, |this, tooltip| {
                            this.tooltip(Tooltip::text(tooltip))
                        })
                        .on_click(cx.listener(move |widget, _, window, cx| {
                            widget.handle_remote_event(
                                RemoteViewEvent {
                                    node_id: node_id.clone(),
                                    kind: RemoteViewEventKind::Click,
                                    payload_json: None,
                                },
                                window,
                                cx,
                            );
                        }))
                        .into_any_element()
                } else {
                    ui::h_flex()
                        .gap(gap)
                        .children(rendered_children)
                        .into_any_element()
                }
            }
            RemoteViewNodeKind::Column => {
                let node_id = node.node_id.clone();
                let gap = node_gap(node, 8.);
                let tooltip = node_tooltip(node);
                if node_is_clickable(node) {
                    v_flex()
                        .id(node_id.clone())
                        .gap(gap)
                        .children(rendered_children)
                        .px_1()
                        .rounded_sm()
                        .cursor_pointer()
                        .hover(|style| style.bg(cx.theme().colors().ghost_element_hover))
                        .active(|style| style.bg(cx.theme().colors().ghost_element_active))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .when_some(tooltip, |this, tooltip| {
                            this.tooltip(Tooltip::text(tooltip))
                        })
                        .on_click(cx.listener(move |widget, _, window, cx| {
                            widget.handle_remote_event(
                                RemoteViewEvent {
                                    node_id: node_id.clone(),
                                    kind: RemoteViewEventKind::Click,
                                    payload_json: None,
                                },
                                window,
                                cx,
                            );
                        }))
                        .into_any_element()
                } else {
                    v_flex()
                        .gap(gap)
                        .children(rendered_children)
                        .into_any_element()
                }
            }
            RemoteViewNodeKind::Stack => {
                let node_id = node.node_id.clone();
                let gap = node_gap(node, 4.);
                let tooltip = node_tooltip(node);
                if node_is_clickable(node) {
                    v_flex()
                        .id(node_id.clone())
                        .gap(gap)
                        .children(rendered_children)
                        .px_1()
                        .rounded_sm()
                        .cursor_pointer()
                        .hover(|style| style.bg(cx.theme().colors().ghost_element_hover))
                        .active(|style| style.bg(cx.theme().colors().ghost_element_active))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .when_some(tooltip, |this, tooltip| {
                            this.tooltip(Tooltip::text(tooltip))
                        })
                        .on_click(cx.listener(move |widget, _, window, cx| {
                            widget.handle_remote_event(
                                RemoteViewEvent {
                                    node_id: node_id.clone(),
                                    kind: RemoteViewEventKind::Click,
                                    payload_json: None,
                                },
                                window,
                                cx,
                            );
                        }))
                        .into_any_element()
                } else {
                    v_flex()
                        .gap(gap)
                        .children(rendered_children)
                        .into_any_element()
                }
            }
            RemoteViewNodeKind::Text(text) => text.clone().into_any_element(),
            RemoteViewNodeKind::Icon(icon) => {
                let node_id = node.node_id.clone();
                let tooltip = node_tooltip(node);
                if ancestor_is_clickable {
                    Icon::from_path(icon.clone()).into_any_element()
                } else {
                    ui::ButtonLike::new(node_id.clone())
                        .style(ui::ButtonStyle::Transparent)
                        .size(ui::ButtonSize::Compact)
                        .when_some(tooltip, |this, tooltip| {
                            this.tooltip(Tooltip::text(tooltip))
                        })
                        .child(Icon::from_path(icon.clone()))
                        .on_click(cx.listener(move |widget, _, window, cx| {
                            widget.handle_remote_event(
                                RemoteViewEvent {
                                    node_id: node_id.clone(),
                                    kind: RemoteViewEventKind::Click,
                                    payload_json: None,
                                },
                                window,
                                cx,
                            );
                        }))
                        .into_any_element()
                }
            }
            RemoteViewNodeKind::Button(label) => {
                let node_id = node.node_id.clone();
                let tooltip = node_tooltip(node);
                Button::new(node_id.clone(), label.clone())
                    .when_some(tooltip, |this, tooltip| {
                        this.tooltip(Tooltip::text(tooltip))
                    })
                    .on_click(cx.listener(move |widget, _, window, cx| {
                        widget.handle_remote_event(
                            RemoteViewEvent {
                                node_id: node_id.clone(),
                                kind: RemoteViewEventKind::Click,
                                payload_json: None,
                            },
                            window,
                            cx,
                        );
                    }))
                    .into_any_element()
            }
            RemoteViewNodeKind::Toggle(value) => {
                format!("Toggle: {}", if *value { "on" } else { "off" }).into_any_element()
            }
            RemoteViewNodeKind::Checkbox(value) => {
                format!("Checkbox: {}", if *value { "checked" } else { "unchecked" })
                    .into_any_element()
            }
            RemoteViewNodeKind::TextInput(value) => value.clone().into_any_element(),
            RemoteViewNodeKind::Badge(value) => {
                let node_id = node.node_id.clone();
                let tooltip = node_tooltip(node);
                if ancestor_is_clickable {
                    value.clone().into_any_element()
                } else {
                    ui::ButtonLike::new(node_id.clone())
                        .style(ui::ButtonStyle::Transparent)
                        .size(ui::ButtonSize::Compact)
                        .when_some(tooltip, |this, tooltip| {
                            this.tooltip(Tooltip::text(tooltip))
                        })
                        .child(value.clone())
                        .on_click(cx.listener(move |widget, _, window, cx| {
                            widget.handle_remote_event(
                                RemoteViewEvent {
                                    node_id: node_id.clone(),
                                    kind: RemoteViewEventKind::Click,
                                    payload_json: None,
                                },
                                window,
                                cx,
                            );
                        }))
                        .into_any_element()
                }
            }
            RemoteViewNodeKind::ProgressBar(props) => ProgressBar::new(
                node.node_id.clone(),
                props.value as f32,
                props.max_value.max(1) as f32,
                cx,
            )
            .into_any_element(),
            RemoteViewNodeKind::Divider => Divider::horizontal().into_any_element(),
            RemoteViewNodeKind::Spacer => gpui::div().flex_grow().into_any_element(),
            RemoteViewNodeKind::ScrollView => {
                gpui::div().children(rendered_children).into_any_element()
            }
            RemoteViewNodeKind::VirtualList(_) => {
                "Virtual list is not implemented yet".into_any_element()
            }
        }
    }
}

impl Render for RemoteUiWidget {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        ui::h_flex()
            .h_full()
            .flex_none()
            .items_center()
            .overflow_hidden()
            .child(self.render_content(window, cx))
    }
}

struct RemoteUiWidgetStrip {
    surface: WidgetSurface,
    side: StripSide,
    workspace: WeakEntity<Workspace>,
    workspace_id: u64,
    items: Vec<MountedRemoteUiWidget>,
}

impl RemoteUiWidgetStrip {
    fn new(
        surface: WidgetSurface,
        side: StripSide,
        workspace: WeakEntity<Workspace>,
        workspace_id: u64,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            surface,
            side,
            workspace,
            workspace_id,
            items: Vec::new(),
        };
        this.refresh(cx);
        this
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let descriptors = RemoteUiRegistry::read_global(cx)
            .map(|registry| registry.widget_descriptors(self.surface, self.side))
            .unwrap_or_default();
        let mut existing = std::mem::take(&mut self.items)
            .into_iter()
            .map(|item| (item.descriptor.qualified_widget_id.clone(), item))
            .collect::<HashMap<_, _>>();
        let mut next_items = Vec::with_capacity(descriptors.len());

        for descriptor in descriptors {
            let mounted = if let Some(item) = existing.remove(&descriptor.qualified_widget_id) {
                let entity = item.entity.clone();
                entity.update(cx, |widget, cx| {
                    let root_view_changed = widget.descriptor.root_view != descriptor.root_view;
                    widget.descriptor = descriptor.clone();
                    if root_view_changed {
                        widget.instance_id = None;
                        widget.tree = None;
                        widget.error_message = None;
                    }
                    if !widget.loading
                        && (root_view_changed
                            || widget.instance_id.is_none()
                            || widget.tree.is_none()
                            || widget.error_message.is_some())
                    {
                        widget.reload(RenderReason::Initial, cx);
                    }
                });
                MountedRemoteUiWidget {
                    descriptor: descriptor.clone(),
                    entity,
                }
            } else {
                let entity = cx.new(|cx| {
                    RemoteUiWidget::new(
                        descriptor.clone(),
                        self.workspace.clone(),
                        self.workspace_id,
                        cx,
                    )
                });
                MountedRemoteUiWidget {
                    descriptor: descriptor.clone(),
                    entity,
                }
            };
            next_items.push(mounted);
        }

        for removed_widget in existing.into_values() {
            close_remote_ui_widget(removed_widget.entity, cx);
        }

        self.items = next_items;
        cx.notify();
    }
}

fn is_extension_not_loaded_error(error: &anyhow::Error) -> bool {
    error.to_string().contains("is not loaded")
}

fn node_is_clickable(node: &RemoteViewNode) -> bool {
    node.properties
        .iter()
        .any(|property| property.name == "clickable" && property.value == "true")
}

fn node_gap(node: &RemoteViewNode, default_gap_pixels: f32) -> Pixels {
    let gap_pixels = node
        .properties
        .iter()
        .find(|property| property.name == "gap")
        .and_then(|property| property.value.parse::<f32>().ok())
        .unwrap_or(default_gap_pixels);
    px(gap_pixels)
}

fn node_tooltip(node: &RemoteViewNode) -> Option<SharedString> {
    node.properties
        .iter()
        .find(|property| property.name == "tooltip")
        .map(|property| SharedString::from(property.value.clone()))
}

fn widget_width_bounds(widget: &RegisteredRemoteUiWidget) -> (Option<u32>, Option<u32>) {
    let host_max_width = match (widget.surface, widget.side, widget.size) {
        (WidgetSurface::Titlebar, _, WidgetSize::Small) => Some(24),
        (WidgetSurface::Titlebar, _, WidgetSize::Medium) => Some(64),
        (WidgetSurface::Titlebar, _, WidgetSize::Large) => Some(112),
        (WidgetSurface::Footer, StripSide::Left | StripSide::Right, _) => Some(24),
        (WidgetSurface::Footer, StripSide::Center, WidgetSize::Small) => Some(24),
        (WidgetSurface::Footer, StripSide::Center, WidgetSize::Medium) => Some(96),
        (WidgetSurface::Footer, StripSide::Center, WidgetSize::Large) => Some(180),
    };

    let min_width = widget.min_width;
    let max_width = match (widget.max_width, host_max_width) {
        (Some(widget_max_width), Some(host_max_width)) => {
            Some(widget_max_width.min(host_max_width))
        }
        (Some(widget_max_width), None) => Some(widget_max_width),
        (None, host_max_width) => host_max_width,
    };

    (min_width, max_width)
}

fn validate_widget_tree(widget: &RegisteredRemoteUiWidget, tree: &RemoteViewTree) -> Result<()> {
    let mut icon_count = 0usize;
    let mut badge_count = 0usize;
    let mut content_unit_count = 0usize;

    for node in &tree.nodes {
        match (widget.surface, widget.side, &node.kind) {
            (
                _,
                _,
                RemoteViewNodeKind::Row | RemoteViewNodeKind::Column | RemoteViewNodeKind::Stack,
            ) => {}
            (WidgetSurface::Titlebar, _, RemoteViewNodeKind::Icon(_)) => {
                icon_count += 1;
                content_unit_count += 1;
            }
            (WidgetSurface::Titlebar, _, RemoteViewNodeKind::Text(_)) => {
                content_unit_count += 1;
            }
            (WidgetSurface::Titlebar, _, RemoteViewNodeKind::Badge(_)) => {
                badge_count += 1;
                content_unit_count += 1;
            }
            (
                WidgetSurface::Footer,
                StripSide::Left | StripSide::Right,
                RemoteViewNodeKind::Icon(_),
            ) => {
                icon_count += 1;
                content_unit_count += 1;
            }
            (
                WidgetSurface::Footer,
                StripSide::Center,
                RemoteViewNodeKind::Icon(_)
                | RemoteViewNodeKind::Text(_)
                | RemoteViewNodeKind::Badge(_)
                | RemoteViewNodeKind::Button(_)
                | RemoteViewNodeKind::ProgressBar(_),
            ) => {
                content_unit_count += 1;
            }
            _ => {
                bail!(
                    "widget `{}` uses unsupported element {:?} for this slot",
                    widget.qualified_widget_id,
                    node.kind
                );
            }
        }
    }

    if matches!(
        (widget.surface, widget.side),
        (WidgetSurface::Footer, StripSide::Left | StripSide::Right)
    ) && icon_count != 1
    {
        bail!(
            "widget `{}` must render exactly one icon in footer edge slots",
            widget.qualified_widget_id
        );
    }

    if widget.surface == WidgetSurface::Titlebar
        && widget.size == WidgetSize::Small
        && content_unit_count == 1
        && icon_count == 0
        && badge_count == 0
    {
        bail!(
            "widget `{}` size `s` must render an icon or badge only in the titlebar",
            widget.qualified_widget_id
        );
    }

    let max_content_units = match (widget.surface, widget.side, widget.size) {
        (WidgetSurface::Titlebar, _, WidgetSize::Small) => 1,
        (WidgetSurface::Titlebar, _, WidgetSize::Medium) => 2,
        (WidgetSurface::Titlebar, _, WidgetSize::Large) => 4,
        (WidgetSurface::Footer, StripSide::Left | StripSide::Right, _) => 1,
        (WidgetSurface::Footer, StripSide::Center, WidgetSize::Small) => 1,
        (WidgetSurface::Footer, StripSide::Center, WidgetSize::Medium) => 2,
        (WidgetSurface::Footer, StripSide::Center, WidgetSize::Large) => 4,
    };
    if content_unit_count > max_content_units {
        bail!(
            "widget `{}` exceeds the `{}` content budget for this slot",
            widget.qualified_widget_id,
            match widget.size {
                WidgetSize::Small => "s",
                WidgetSize::Medium => "m",
                WidgetSize::Large => "l",
            }
        );
    }

    Ok(())
}

impl Render for RemoteUiWidgetStrip {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let host_height = match self.surface {
            WidgetSurface::Titlebar => platform_title_bar_height(window),
            WidgetSurface::Footer => window.line_height(),
        };

        ui::h_flex()
            .gap_1()
            .children(self.items.iter().map(|item| {
                let (min_width, max_width) = widget_width_bounds(&item.descriptor);
                ui::h_flex()
                    .h_full()
                    .flex_none()
                    .items_center()
                    .overflow_hidden()
                    .when_some(min_width, |div, min_width| div.min_w(px(min_width as f32)))
                    .when_some(max_width, |div, max_width| div.max_w(px(max_width as f32)))
                    .child(item.entity.clone())
            }))
            .h(host_height)
            .overflow_hidden()
    }
}

impl StatusItemView for RemoteUiWidgetStrip {
    fn set_active_pane_item(
        &mut self,
        _active_pane_item: Option<&dyn workspace::ItemHandle>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
    }
}

fn dispatch_registered_command_from_workspace(
    workspace: Entity<Workspace>,
    workspace_id: u64,
    command_id: &str,
    input_json: Option<String>,
    cx: &mut App,
) -> Result<()> {
    let command = RemoteUiRegistry::read_global(cx)
        .and_then(|registry| registry.commands.get(command_id).cloned())
        .with_context(|| format!("unknown extension command `{command_id}`"))?;
    let extension = extension_for_id(command.extension_id.as_ref(), cx)?;
    let command_context = CommandContext {
        workspace_id,
        trusted: false,
        active_item_kind: None,
    };

    cx.spawn(async move |cx| {
        if let Err(error) = extension
            .run_command(
                command.local_command_id.clone(),
                command_context,
                input_json,
            )
            .await
        {
            let message = format!("Extension command `{}` failed: {error}", command.title);
            log::error!("{message}");
            workspace.update(cx, |workspace, cx| {
                workspace.show_toast(
                    Toast::new(
                        NotificationId::unique::<RemoteUiCommandError>(),
                        message.clone(),
                    )
                    .autohide(),
                    cx,
                );
            });
        }
        Ok::<(), anyhow::Error>(())
    })
    .detach_and_log_err(cx);

    Ok(())
}

fn extension_for_id(extension_id: &str, cx: &App) -> Result<Arc<dyn Extension>> {
    let store = ExtensionStore::try_global(cx).context("missing extension store")?;
    store
        .read_with(cx, |store, _cx| {
            store
                .wasm_extensions
                .iter()
                .find(|(manifest, _)| manifest.id.as_ref() == extension_id)
                .map(|(_, extension)| Arc::new(extension.clone()) as Arc<dyn Extension>)
        })
        .with_context(|| format!("extension `{extension_id}` is not loaded"))
}

fn workspace_for_id(
    workspace_id: u64,
    cx: &App,
) -> Result<(gpui::AnyWindowHandle, Entity<Workspace>)> {
    let app_state = AppState::try_global(cx)
        .and_then(|app_state| app_state.upgrade())
        .context("missing app state")?;
    let target_workspace_id =
        WorkspaceId::from_i64(i64::try_from(workspace_id).context("workspace id overflow")?);

    app_state
        .workspace_store
        .read_with(cx, |workspace_store, cx| {
            workspace_store
                .workspaces_with_windows()
                .filter_map(|(window_handle, weak_workspace)| {
                    Some((window_handle, weak_workspace.upgrade()?))
                })
                .find(|(_, workspace)| {
                    workspace.read(cx).database_id() == Some(target_workspace_id)
                })
        })
        .with_context(|| format!("workspace `{workspace_id}` is not available"))
}

fn mount_registered_panels_for_workspace(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let Some(workspace_id) = workspace.database_id().and_then(WorkspaceId::to_u64) else {
        return;
    };
    let panel_descriptors = RemoteUiRegistry::read_global(cx)
        .map(|registry| registry.panels.values().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    mount_remote_ui_panels_for_workspace(workspace, window, workspace_id, panel_descriptors, cx);
}

fn mount_remote_ui_widget_strips_for_workspace(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let Some(workspace_id) = workspace.database_id().and_then(WorkspaceId::to_u64) else {
        return;
    };
    let workspace_handle = cx.entity().downgrade();

    for side in [StripSide::Left, StripSide::Center, StripSide::Right] {
        let footer_key = WidgetStripKey {
            workspace_id,
            surface: WidgetSurface::Footer,
            side,
        };
        let footer_strip = RemoteUiRegistry::read_global(cx)
            .and_then(|registry| registry.mounted_widget_strip(footer_key))
            .and_then(|strip| strip.upgrade())
            .unwrap_or_else(|| {
                let strip = cx.new(|cx| {
                    RemoteUiWidgetStrip::new(
                        WidgetSurface::Footer,
                        side,
                        workspace_handle.clone(),
                        workspace_id,
                        cx,
                    )
                });
                workspace
                    .status_bar()
                    .update(cx, |status_bar, cx| match side {
                        StripSide::Left => status_bar.add_left_item(strip.clone(), window, cx),
                        StripSide::Center => status_bar.add_center_item(strip.clone(), window, cx),
                        StripSide::Right => status_bar.add_right_item(strip.clone(), window, cx),
                    });
                RemoteUiRegistry::update_global(cx, |registry| {
                    registry.remember_widget_strip(footer_key, strip.downgrade());
                });
                strip
            });
        footer_strip.update(cx, |strip, cx| strip.refresh(cx));

        if side == StripSide::Center {
            continue;
        }

        let titlebar_key = WidgetStripKey {
            workspace_id,
            surface: WidgetSurface::Titlebar,
            side,
        };
        let titlebar_strip = RemoteUiRegistry::read_global(cx)
            .and_then(|registry| registry.mounted_widget_strip(titlebar_key))
            .and_then(|strip| strip.upgrade())
            .unwrap_or_else(|| {
                let strip = cx.new(|cx| {
                    RemoteUiWidgetStrip::new(
                        WidgetSurface::Titlebar,
                        side,
                        workspace_handle.clone(),
                        workspace_id,
                        cx,
                    )
                });
                RemoteUiRegistry::update_global(cx, |registry| {
                    registry.remember_widget_strip(titlebar_key, strip.downgrade());
                });
                strip
            });
        titlebar_strip.update(cx, |strip, cx| strip.refresh(cx));
    }

    update_workspace_titlebar_widget_strips(workspace, cx);
}

fn refresh_remote_ui_widget_strips_in_all_workspaces(cx: &mut App) {
    let Some(app_state) = AppState::try_global(cx).and_then(|app_state| app_state.upgrade()) else {
        return;
    };

    let workspaces = app_state
        .workspace_store
        .read_with(cx, |workspace_store, _cx| {
            workspace_store
                .workspaces_with_windows()
                .filter_map(|(window_handle, weak_workspace)| {
                    Some((window_handle, weak_workspace.upgrade()?))
                })
                .collect::<Vec<_>>()
        });

    for (window_handle, workspace) in workspaces {
        let _ = window_handle.update(cx, |_, window, cx| {
            let _ = workspace.update(cx, |workspace, cx| {
                mount_remote_ui_widget_strips_for_workspace(workspace, window, cx);
            });
        });
    }
}

fn mount_remote_ui_panels_in_all_workspaces(
    panel_descriptors: Vec<RegisteredRemoteUiPanel>,
    cx: &mut App,
) {
    if panel_descriptors.is_empty() {
        return;
    }

    let Some(app_state) = AppState::try_global(cx).and_then(|app_state| app_state.upgrade()) else {
        return;
    };

    let workspaces = app_state
        .workspace_store
        .read_with(cx, |workspace_store, _cx| {
            workspace_store
                .workspaces_with_windows()
                .filter_map(|(window_handle, weak_workspace)| {
                    Some((window_handle, weak_workspace.upgrade()?))
                })
                .collect::<Vec<_>>()
        });

    for (window_handle, workspace) in workspaces {
        let panel_descriptors = panel_descriptors.clone();
        let _ = window_handle.update(cx, |_, window, cx| {
            let _ = workspace.update(cx, |workspace, cx| {
                let Some(workspace_id) = workspace.database_id().and_then(WorkspaceId::to_u64)
                else {
                    return;
                };
                mount_remote_ui_panels_for_workspace(
                    workspace,
                    window,
                    workspace_id,
                    panel_descriptors,
                    cx,
                );
            });
        });
    }
}

fn mount_remote_ui_panels_for_workspace(
    workspace: &mut Workspace,
    window: &mut Window,
    workspace_id: u64,
    panel_descriptors: Vec<RegisteredRemoteUiPanel>,
    cx: &mut Context<Workspace>,
) {
    let workspace_handle = cx.entity().downgrade();

    for descriptor in panel_descriptors {
        let existing_panel = RemoteUiRegistry::read_global(cx)
            .and_then(|registry| {
                registry
                    .mounted_panels
                    .get(&(workspace_id, descriptor.qualified_panel_id.clone()))
                    .cloned()
            })
            .and_then(|panel| panel.upgrade());
        if let Some(panel) = existing_panel {
            panel.update(cx, |panel, cx| {
                if panel.descriptor.root_view != descriptor.root_view {
                    panel.descriptor = descriptor.clone();
                    panel.instance_id = None;
                    panel.tree = None;
                    panel.error_message = None;
                }
                if !panel.loading
                    && (panel.instance_id.is_none()
                        || panel.tree.is_none()
                        || panel.error_message.is_some())
                {
                    panel.reload(RenderReason::Initial, cx);
                }
            });
            continue;
        }

        let panel = cx.new(|cx| {
            RemoteUiPanel::new(
                descriptor.clone(),
                workspace_handle.clone(),
                workspace_id,
                cx,
            )
        });
        workspace.add_panel(panel.clone(), window, cx);
        RemoteUiRegistry::update_global(cx, |registry| {
            registry.remember_mounted_panel(
                workspace_id,
                descriptor.qualified_panel_id.clone(),
                panel.downgrade(),
            );
        });
    }
}

fn unmount_remote_ui_panels_in_all_workspaces(panel_ids: &[Arc<str>], cx: &mut App) {
    if panel_ids.is_empty() {
        return;
    }

    let Some(app_state) = AppState::try_global(cx).and_then(|app_state| app_state.upgrade()) else {
        return;
    };

    let workspaces = app_state
        .workspace_store
        .read_with(cx, |workspace_store, _cx| {
            workspace_store
                .workspaces_with_windows()
                .filter_map(|(window_handle, weak_workspace)| {
                    Some((window_handle, weak_workspace.upgrade()?))
                })
                .collect::<Vec<_>>()
        });

    for (window_handle, workspace) in workspaces {
        let panel_ids = panel_ids.to_vec();
        let _ = window_handle.update(cx, |_, window, cx| {
            let _ = workspace.update(cx, |workspace, cx| {
                let Some(workspace_id) = workspace.database_id().and_then(WorkspaceId::to_u64)
                else {
                    return;
                };

                for panel_id in &panel_ids {
                    let panel = RemoteUiRegistry::read_global(cx)
                        .and_then(|registry| {
                            registry
                                .mounted_panels
                                .get(&(workspace_id, panel_id.clone()))
                                .cloned()
                        })
                        .and_then(|panel| panel.upgrade());
                    let Some(panel) = panel else {
                        RemoteUiRegistry::update_global(cx, |registry| {
                            registry.forget_mounted_panel(workspace_id, panel_id.as_ref());
                        });
                        continue;
                    };

                    close_remote_ui_panel(panel.clone(), cx);
                    workspace.remove_panel(&panel, window, cx);
                    RemoteUiRegistry::update_global(cx, |registry| {
                        registry.forget_mounted_panel(workspace_id, panel_id.as_ref());
                    });
                }
            });
        });
    }
}

fn set_panel_open_state(
    window_handle: gpui::AnyWindowHandle,
    workspace: Entity<Workspace>,
    workspace_id: u64,
    panel_id: &str,
    open: bool,
    cx: &mut App,
) -> Result<()> {
    log::info!(
        "remote ui set_panel_open_state workspace_id={} panel_id={} open={}",
        workspace_id,
        panel_id,
        open
    );
    let panel = RemoteUiRegistry::read_global(cx)
        .and_then(|registry| {
            registry
                .mounted_panels
                .get(&(workspace_id, Arc::from(panel_id)))
                .cloned()
        })
        .and_then(|panel| panel.upgrade())
        .with_context(|| format!("unknown extension panel `{panel_id}`"))?;

    let _ = window_handle.update(cx, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            if open {
                panel.update(cx, |_, cx| cx.emit(PanelEvent::Activate));
            } else {
                panel.update(cx, |_, cx| cx.emit(PanelEvent::Close));
                if panel.focus_handle(cx).contains_focused(window, cx) {
                    workspace.active_pane().update(cx, |pane, cx| {
                        window.focus(&pane.focus_handle(cx), cx);
                    });
                }
            }
        });
    });

    Ok(())
}

fn close_remote_ui_panel(panel: Entity<RemoteUiPanel>, cx: &mut App) {
    let (extension_id, instance_id) = panel.read_with(cx, |panel, _cx| {
        (panel.descriptor.extension_id.clone(), panel.instance_id)
    });
    let Some(instance_id) = instance_id else {
        return;
    };
    let extension = match extension_for_id(extension_id.as_ref(), cx) {
        Ok(extension) => extension,
        Err(error) => {
            log::error!("Failed to close extension panel view: {error}");
            return;
        }
    };

    cx.spawn(async move |_cx| extension.close_view(instance_id).await)
        .detach_and_log_err(cx);
}

fn close_remote_ui_widget(widget: Entity<RemoteUiWidget>, cx: &mut App) {
    let (extension_id, instance_id) = widget.read_with(cx, |widget, _cx| {
        (widget.descriptor.extension_id.clone(), widget.instance_id)
    });
    let Some(instance_id) = instance_id else {
        return;
    };
    let extension = match extension_for_id(extension_id.as_ref(), cx) {
        Ok(extension) => extension,
        Err(error) => {
            log::error!("Failed to close extension widget view: {error}");
            return;
        }
    };

    cx.spawn(async move |_cx| extension.close_view(instance_id).await)
        .detach_and_log_err(cx);
}

fn update_workspace_titlebar_widget_strips(workspace: &mut Workspace, cx: &mut Context<Workspace>) {
    let Some(titlebar) = workspace
        .titlebar_item()
        .and_then(|item| item.downcast::<TitleBar>().ok())
    else {
        return;
    };
    let Some(workspace_id) = workspace.database_id().and_then(WorkspaceId::to_u64) else {
        return;
    };

    let left_strip = RemoteUiRegistry::read_global(cx)
        .and_then(|registry| {
            registry.mounted_widget_strip(WidgetStripKey {
                workspace_id,
                surface: WidgetSurface::Titlebar,
                side: StripSide::Left,
            })
        })
        .and_then(|strip| strip.upgrade())
        .map(Into::into);
    let right_strip = RemoteUiRegistry::read_global(cx)
        .and_then(|registry| {
            registry.mounted_widget_strip(WidgetStripKey {
                workspace_id,
                surface: WidgetSurface::Titlebar,
                side: StripSide::Right,
            })
        })
        .and_then(|strip| strip.upgrade())
        .map(Into::into);

    titlebar.update(cx, |titlebar, cx| {
        titlebar.set_extension_items(left_strip, right_strip, cx);
    });
}

fn toggle_panel_visibility(
    workspace: &mut Workspace,
    window: &mut Window,
    workspace_id: u64,
    panel_id: &str,
    cx: &mut Context<Workspace>,
) -> Result<()> {
    let panel = RemoteUiRegistry::read_global(cx)
        .and_then(|registry| {
            registry
                .mounted_panels
                .get(&(workspace_id, Arc::from(panel_id)))
                .cloned()
        })
        .and_then(|panel| panel.upgrade())
        .with_context(|| format!("unknown extension panel `{panel_id}`"))?;

    let position = panel.read(cx).position(window, cx);
    let (is_visible, is_focused) = {
        let dock = workspace.dock_at_position(position).read(cx);
        let is_visible = dock
            .visible_panel()
            .is_some_and(|visible_panel| visible_panel.panel_id() == panel.entity_id());
        let is_focused = panel.focus_handle(cx).contains_focused(window, cx);
        (is_visible, is_focused)
    };

    if is_visible && is_focused {
        panel.update(cx, |_, cx| cx.emit(PanelEvent::Close));
        workspace.active_pane().update(cx, |pane, cx| {
            window.focus(&pane.focus_handle(cx), cx);
        });
    } else {
        panel.update(cx, |_, cx| cx.emit(PanelEvent::Activate));
    }

    Ok(())
}

fn dock_position_for_panel(panel: &ExtensionPanelManifestEntry) -> DockPosition {
    match panel.default_dock.unwrap_or(DockSide::Right) {
        DockSide::Left => DockPosition::Left,
        DockSide::Bottom => DockPosition::Bottom,
        DockSide::Right => DockPosition::Right,
    }
}

fn strip_side_for_titlebar_widget(side: WidgetSide) -> StripSide {
    match side {
        WidgetSide::Left => StripSide::Left,
        WidgetSide::Right => StripSide::Right,
    }
}

fn strip_side_for_footer_widget(zone: FooterWidgetZone) -> StripSide {
    match zone {
        FooterWidgetZone::Left => StripSide::Left,
        FooterWidgetZone::Center => StripSide::Center,
        FooterWidgetZone::Right => StripSide::Right,
    }
}

fn registered_titlebar_widget(
    extension_id: &str,
    widget: &TitlebarWidgetManifestEntry,
) -> RegisteredRemoteUiWidget {
    RegisteredRemoteUiWidget {
        qualified_widget_id: qualified_remote_ui_id(extension_id, widget.id.as_str()),
        extension_id: Arc::from(extension_id),
        root_view: widget.root_view.clone().into(),
        surface: WidgetSurface::Titlebar,
        side: strip_side_for_titlebar_widget(widget.side),
        size: widget.size,
        priority: widget.priority,
        min_width: widget.min_width,
        max_width: widget.max_width,
        refresh_interval_seconds: widget.refresh_interval_seconds,
    }
}

fn registered_footer_widget(
    extension_id: &str,
    widget: &FooterWidgetManifestEntry,
) -> RegisteredRemoteUiWidget {
    RegisteredRemoteUiWidget {
        qualified_widget_id: qualified_remote_ui_id(extension_id, widget.id.as_str()),
        extension_id: Arc::from(extension_id),
        root_view: widget.root_view.clone().into(),
        surface: WidgetSurface::Footer,
        side: strip_side_for_footer_widget(widget.zone),
        size: widget.size,
        priority: widget.priority,
        min_width: widget.min_width,
        max_width: widget.max_width,
        refresh_interval_seconds: widget.refresh_interval_seconds,
    }
}

fn qualified_remote_ui_id(extension_id: &str, local_id: &str) -> Arc<str> {
    format!("{extension_id}::{local_id}").into()
}

fn palette_item_for_menu(
    extension_id: &str,
    remote_ui: &RemoteUiManifest,
    menu: &ExtensionMenuManifestEntry,
) -> Option<RegisteredRemoteUiPaletteItem> {
    let command = remote_ui.commands.get(menu.command.as_str())?;
    Some(RegisteredRemoteUiPaletteItem {
        command_id: qualified_remote_ui_id(extension_id, &menu.command),
        title: menu.title.clone().into(),
        search_text: palette_search_text(
            extension_id,
            &menu.command,
            &menu.title,
            Some(command.description.as_str()),
        ),
    })
}

fn palette_search_text(
    extension_id: &str,
    command_id: &str,
    title: &str,
    description: Option<&str>,
) -> Arc<str> {
    let qualified_command_id = qualified_remote_ui_id(extension_id, command_id);
    match description {
        Some(description) if !description.is_empty() => {
            format!("{title} {description} {extension_id} {qualified_command_id}").into()
        }
        _ => format!("{title} {extension_id} {qualified_command_id}").into(),
    }
}

fn remote_ui_item_tab_context_menu_hook(
    menu: ContextMenu,
    workspace: WeakEntity<Workspace>,
    _item: &dyn ItemHandle,
    window: &mut Window,
    cx: &mut App,
) -> ContextMenu {
    append_remote_ui_context_menu_entries(
        menu,
        workspace,
        MenuLocation::ItemTabContext,
        ContextActionTarget::ItemTab,
        None,
        window,
        cx,
    )
}

fn remote_ui_editor_context_menu_hook(
    menu: ContextMenu,
    workspace: WeakEntity<Workspace>,
    _editor: WeakEntity<Editor>,
    _point: editor::DisplayPoint,
    window: &mut Window,
    cx: &mut App,
) -> ContextMenu {
    append_remote_ui_context_menu_entries(
        menu,
        workspace,
        MenuLocation::EditorContext,
        ContextActionTarget::Editor,
        None,
        window,
        cx,
    )
}

fn remote_ui_panel_overflow_hook(
    menu: ContextMenu,
    workspace: WeakEntity<Workspace>,
    panel: &dyn PanelHandle,
    window: &mut Window,
    cx: &mut App,
) -> ContextMenu {
    let current_panel_qualified_id = workspace.upgrade().and_then(|workspace| {
        let workspace_id = workspace
            .read(cx)
            .database_id()
            .and_then(WorkspaceId::to_u64)?;
        qualified_panel_id_for_handle(workspace_id, panel.panel_id(), cx)
    });

    append_remote_ui_context_menu_entries(
        menu,
        workspace,
        MenuLocation::PanelOverflow,
        ContextActionTarget::Panel,
        current_panel_qualified_id.as_deref(),
        window,
        cx,
    )
}

fn remote_ui_project_panel_context_menu_hook(
    menu: ContextMenu,
    workspace: WeakEntity<Workspace>,
    _target: &project_panel::ProjectPanelContextMenuTarget,
    window: &mut Window,
    cx: &mut App,
) -> ContextMenu {
    append_remote_ui_context_menu_entries(
        menu,
        workspace,
        MenuLocation::ProjectPanelContext,
        ContextActionTarget::ProjectPanel,
        None,
        window,
        cx,
    )
}

fn append_remote_ui_context_menu_entries(
    mut menu: ContextMenu,
    workspace: WeakEntity<Workspace>,
    menu_location: MenuLocation,
    action_target: ContextActionTarget,
    current_panel_qualified_id: Option<&str>,
    window: &mut Window,
    cx: &mut App,
) -> ContextMenu {
    let entries = remote_ui_context_menu_entries(
        menu_location,
        action_target,
        current_panel_qualified_id,
        cx,
    );
    if entries.is_empty() {
        return menu;
    }

    menu = menu.separator();
    append_registered_remote_ui_context_menu_entries(menu, workspace, entries, window, cx)
}

fn build_remote_ui_panel_overflow_menu(
    workspace: WeakEntity<Workspace>,
    panel_qualified_id: Arc<str>,
    window: &mut Window,
    cx: &mut App,
) -> Option<Entity<ContextMenu>> {
    let entries = remote_ui_context_menu_entries(
        MenuLocation::PanelOverflow,
        ContextActionTarget::Panel,
        Some(panel_qualified_id.as_ref()),
        cx,
    );
    if entries.is_empty() {
        return None;
    }

    Some(ContextMenu::build(window, cx, move |menu, window, cx| {
        append_registered_remote_ui_context_menu_entries(
            menu,
            workspace.clone(),
            entries.clone(),
            window,
            cx,
        )
    }))
}

fn remote_ui_context_menu_entries(
    menu_location: MenuLocation,
    action_target: ContextActionTarget,
    current_panel_qualified_id: Option<&str>,
    cx: &App,
) -> Vec<RegisteredRemoteUiContextMenuEntry> {
    RemoteUiRegistry::read_global(cx)
        .map(|registry| {
            registry.context_menu_entries(menu_location, action_target, current_panel_qualified_id)
        })
        .unwrap_or_default()
}

fn append_registered_remote_ui_context_menu_entries(
    mut menu: ContextMenu,
    workspace: WeakEntity<Workspace>,
    entries: Vec<RegisteredRemoteUiContextMenuEntry>,
    window: &mut Window,
    _cx: &mut App,
) -> ContextMenu {
    let Some(workspace) = workspace.upgrade() else {
        return menu;
    };

    for entry in entries {
        let command_id = entry.command_id.clone();
        let title = entry.title.clone();
        menu = menu.item(ContextMenuItem::Entry(
            ContextMenuEntry::new(title.clone()).handler(window.handler_for(
                &workspace,
                move |workspace, _window, cx| {
                    let Some(workspace_id) = workspace.database_id().and_then(WorkspaceId::to_u64)
                    else {
                        return;
                    };

                    if let Err(error) = dispatch_registered_command_from_workspace(
                        cx.entity(),
                        workspace_id,
                        command_id.as_ref(),
                        None,
                        cx,
                    ) {
                        let message = format!("Failed to run extension command `{title}`: {error}");
                        workspace.show_toast(
                            Toast::new(
                                NotificationId::unique::<RemoteUiCommandError>(),
                                message.clone(),
                            )
                            .autohide(),
                            cx,
                        );
                        log::error!("{message}");
                    }
                },
            )),
        ));
    }

    menu
}

fn qualified_panel_id_for_handle(
    workspace_id: u64,
    panel_entity_id: gpui::EntityId,
    cx: &App,
) -> Option<Arc<str>> {
    RemoteUiRegistry::read_global(cx).and_then(|registry| {
        registry
            .mounted_panels
            .iter()
            .find_map(|((mounted_workspace_id, panel_id), panel)| {
                if *mounted_workspace_id != workspace_id {
                    return None;
                }

                let panel = panel.upgrade()?;
                (panel.entity_id() == panel_entity_id).then(|| panel_id.clone())
            })
    })
}

fn remote_ui_command_palette_interceptor(
    query: &str,
    _workspace: WeakEntity<Workspace>,
    cx: &mut App,
) -> Task<CommandInterceptResult> {
    let query = normalize_action_query(query);
    let items = RemoteUiRegistry::read_global(cx)
        .map(|registry| registry.command_palette_items.clone())
        .unwrap_or_default();

    Task::ready(CommandInterceptResult {
        results: remote_ui_command_palette_results(items, &query),
        exclusive: false,
    })
}

fn remote_ui_command_palette_results(
    items: Vec<RegisteredRemoteUiPaletteItem>,
    query: &str,
) -> Vec<CommandInterceptItem> {
    let normalized_query = query.to_ascii_lowercase();
    let mut results = items
        .into_iter()
        .filter_map(|item| {
            let title = item.title.to_string();
            if normalized_query.is_empty() {
                return Some(CommandInterceptItem {
                    action: RunRegisteredCommand {
                        command_id: item.command_id.to_string(),
                        input_json: None,
                    }
                    .boxed_clone(),
                    string: title,
                    positions: vec![],
                });
            }

            let search_text =
                normalize_action_query(item.search_text.as_ref()).to_ascii_lowercase();
            if !search_text.contains(&normalized_query) {
                return None;
            }

            Some(CommandInterceptItem {
                action: RunRegisteredCommand {
                    command_id: item.command_id.to_string(),
                    input_json: None,
                }
                .boxed_clone(),
                string: title.clone(),
                positions: substring_match_positions(&title, query),
            })
        })
        .collect::<Vec<_>>();

    results.sort_by(|left, right| left.string.cmp(&right.string));
    results
}

fn substring_match_positions(haystack: &str, query: &str) -> Vec<usize> {
    if query.is_empty() {
        return Vec::new();
    }

    let lowercase_haystack = haystack.to_ascii_lowercase();
    let lowercase_query = query.to_ascii_lowercase();
    let Some(start) = lowercase_haystack.find(&lowercase_query) else {
        return Vec::new();
    };

    let end = start + lowercase_query.len();
    (start..end).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualified_ids_are_namespaced() {
        assert_eq!(
            qualified_remote_ui_id("remote-ui", "sample-open").as_ref(),
            "remote-ui::sample-open"
        );
    }

    #[test]
    fn unregistering_an_extension_removes_its_commands_and_panels() {
        let mut registry = RemoteUiRegistry::default();
        let mut remote_ui = RemoteUiManifest::default();
        remote_ui.commands.insert(
            "sample-open".into(),
            extension::ExtensionCommandManifestEntry {
                title: "Open Sample".into(),
                description: "Open the sample panel".into(),
                palette: true,
                input_schema: None,
                when: None,
            },
        );
        remote_ui.panels.insert(
            "sample-panel".into(),
            ExtensionPanelManifestEntry {
                title: "Sample".into(),
                icon: None,
                default_dock: None,
                default_size: None,
                root_view: "sample-root".into(),
                toggle_command: None,
            },
        );

        registry.register_extension("remote-ui".into(), remote_ui);
        assert!(registry.commands.contains_key("remote-ui::sample-open"));
        assert!(registry.panels.contains_key("remote-ui::sample-panel"));

        registry.unregister_extension("remote-ui");
        assert!(registry.commands.is_empty());
        assert!(registry.panels.is_empty());
    }

    #[test]
    fn command_palette_items_include_palette_commands_and_palette_menus() {
        let mut registry = RemoteUiRegistry::default();
        let mut remote_ui = RemoteUiManifest::default();
        remote_ui.commands.insert(
            "sample-open".into(),
            extension::ExtensionCommandManifestEntry {
                title: "Open Sample".into(),
                description: "Open the sample panel".into(),
                palette: true,
                input_schema: None,
                when: None,
            },
        );
        remote_ui.commands.insert(
            "sample-hidden".into(),
            extension::ExtensionCommandManifestEntry {
                title: "Hidden Sample".into(),
                description: "Shown only through menu wiring".into(),
                palette: false,
                input_schema: None,
                when: None,
            },
        );
        remote_ui.menus.push(ExtensionMenuManifestEntry {
            id: "sample-hidden-menu".into(),
            location: MenuLocation::CommandPalette,
            title: "Open Hidden Sample".into(),
            command: "sample-hidden".into(),
            panel: None,
            group: None,
            priority: 10,
            when: None,
        });

        registry.register_extension("remote-ui".into(), remote_ui);

        let titles = registry
            .command_palette_items
            .iter()
            .map(|item| item.title.to_string())
            .collect::<Vec<_>>();

        assert_eq!(titles, vec!["Open Hidden Sample", "Open Sample"]);
    }

    #[test]
    fn command_palette_search_matches_command_metadata() {
        let results = remote_ui_command_palette_results(
            vec![RegisteredRemoteUiPaletteItem {
                command_id: "remote-ui::sample-open".into(),
                title: "Open Sample".into(),
                search_text: palette_search_text(
                    "remote-ui",
                    "sample-open",
                    "Open Sample",
                    Some("Open the sample panel"),
                ),
            }],
            "sample panel",
        );

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].string, "Open Sample");
    }

    #[test]
    fn widget_descriptors_filter_by_surface_and_side_and_sort_by_priority() {
        let mut registry = RemoteUiRegistry::default();
        let mut remote_ui = RemoteUiManifest::default();
        remote_ui
            .titlebar_widgets
            .push(TitlebarWidgetManifestEntry {
                id: "right-late".into(),
                root_view: "titlebar-right-late".into(),
                side: WidgetSide::Right,
                size: WidgetSize::Large,
                priority: 30,
                min_width: None,
                max_width: None,
                refresh_interval_seconds: None,
                when: None,
            });
        remote_ui
            .titlebar_widgets
            .push(TitlebarWidgetManifestEntry {
                id: "right-early".into(),
                root_view: "titlebar-right-early".into(),
                side: WidgetSide::Right,
                size: WidgetSize::Medium,
                priority: 10,
                min_width: Some(120),
                max_width: Some(240),
                refresh_interval_seconds: Some(300),
                when: None,
            });
        remote_ui
            .titlebar_widgets
            .push(TitlebarWidgetManifestEntry {
                id: "left-only".into(),
                root_view: "titlebar-left-only".into(),
                side: WidgetSide::Left,
                size: WidgetSize::Small,
                priority: 20,
                min_width: None,
                max_width: None,
                refresh_interval_seconds: None,
                when: None,
            });
        remote_ui.footer_widgets.push(FooterWidgetManifestEntry {
            id: "footer-right".into(),
            root_view: "footer-right".into(),
            zone: FooterWidgetZone::Right,
            size: WidgetSize::Small,
            priority: 15,
            min_width: Some(80),
            max_width: Some(160),
            refresh_interval_seconds: None,
            when: None,
        });

        registry.register_extension("remote-ui".into(), remote_ui);

        let titlebar_right = registry.widget_descriptors(WidgetSurface::Titlebar, StripSide::Right);
        let titlebar_left = registry.widget_descriptors(WidgetSurface::Titlebar, StripSide::Left);
        let footer_right = registry.widget_descriptors(WidgetSurface::Footer, StripSide::Right);

        assert_eq!(
            titlebar_right
                .iter()
                .map(|descriptor| descriptor.qualified_widget_id.as_ref())
                .collect::<Vec<_>>(),
            vec!["remote-ui::right-early", "remote-ui::right-late"]
        );
        assert_eq!(titlebar_right[0].min_width, Some(120));
        assert_eq!(titlebar_right[0].max_width, Some(240));
        assert_eq!(titlebar_right[0].refresh_interval_seconds, Some(300));

        assert_eq!(
            titlebar_left
                .iter()
                .map(|descriptor| descriptor.qualified_widget_id.as_ref())
                .collect::<Vec<_>>(),
            vec!["remote-ui::left-only"]
        );
        assert_eq!(
            footer_right
                .iter()
                .map(|descriptor| descriptor.qualified_widget_id.as_ref())
                .collect::<Vec<_>>(),
            vec!["remote-ui::footer-right"]
        );
        assert_eq!(footer_right[0].surface, WidgetSurface::Footer);
        assert_eq!(footer_right[0].side, StripSide::Right);
    }

    #[test]
    fn titlebar_widget_validation_enforces_size_budget() {
        let widget = RegisteredRemoteUiWidget {
            qualified_widget_id: "remote-ui::titlebar".into(),
            extension_id: "remote-ui".into(),
            root_view: "titlebar".into(),
            surface: WidgetSurface::Titlebar,
            side: StripSide::Right,
            size: WidgetSize::Medium,
            priority: 10,
            min_width: None,
            max_width: None,
            refresh_interval_seconds: None,
        };
        let tree = RemoteViewTree {
            revision: 1,
            root_id: "root".into(),
            nodes: vec![
                test_remote_view_node("root", None, RemoteViewNodeKind::Row),
                test_remote_view_node(
                    "icon",
                    Some("root"),
                    RemoteViewNodeKind::Icon("icons/sample.svg".into()),
                ),
                test_remote_view_node(
                    "badge",
                    Some("root"),
                    RemoteViewNodeKind::Badge("42%".into()),
                ),
                test_remote_view_node(
                    "text",
                    Some("root"),
                    RemoteViewNodeKind::Text("extra".into()),
                ),
            ],
        };

        let error = validate_widget_tree(&widget, &tree)
            .unwrap_err()
            .to_string();
        assert!(error.contains("exceeds the `m` content budget"));
    }

    #[test]
    fn footer_edge_widget_validation_requires_single_icon() {
        let widget = RegisteredRemoteUiWidget {
            qualified_widget_id: "remote-ui::footer-edge".into(),
            extension_id: "remote-ui".into(),
            root_view: "footer-edge".into(),
            surface: WidgetSurface::Footer,
            side: StripSide::Left,
            size: WidgetSize::Small,
            priority: 10,
            min_width: None,
            max_width: None,
            refresh_interval_seconds: None,
        };
        let tree = RemoteViewTree {
            revision: 1,
            root_id: "root".into(),
            nodes: vec![
                test_remote_view_node("root", None, RemoteViewNodeKind::Row),
                test_remote_view_node(
                    "icon",
                    Some("root"),
                    RemoteViewNodeKind::Icon("icons/sample.svg".into()),
                ),
                test_remote_view_node(
                    "badge",
                    Some("root"),
                    RemoteViewNodeKind::Badge("42%".into()),
                ),
            ],
        };

        let error = validate_widget_tree(&widget, &tree)
            .unwrap_err()
            .to_string();
        assert!(error.contains("unsupported element"));
    }

    #[test]
    fn footer_center_widget_validation_allows_large_status_cluster() {
        let widget = RegisteredRemoteUiWidget {
            qualified_widget_id: "remote-ui::footer-center".into(),
            extension_id: "remote-ui".into(),
            root_view: "footer-center".into(),
            surface: WidgetSurface::Footer,
            side: StripSide::Center,
            size: WidgetSize::Large,
            priority: 10,
            min_width: None,
            max_width: None,
            refresh_interval_seconds: None,
        };
        let tree = RemoteViewTree {
            revision: 1,
            root_id: "root".into(),
            nodes: vec![
                test_remote_view_node("root", None, RemoteViewNodeKind::Row),
                test_remote_view_node(
                    "icon",
                    Some("root"),
                    RemoteViewNodeKind::Icon("icons/sample.svg".into()),
                ),
                test_remote_view_node(
                    "status",
                    Some("root"),
                    RemoteViewNodeKind::Button("Status".into()),
                ),
                test_remote_view_node(
                    "badge",
                    Some("root"),
                    RemoteViewNodeKind::Badge("7d 20".into()),
                ),
                test_remote_view_node(
                    "progress",
                    Some("root"),
                    RemoteViewNodeKind::ProgressBar(extension::ProgressBarProps {
                        value: 20,
                        max_value: 100,
                    }),
                ),
            ],
        };

        validate_widget_tree(&widget, &tree).expect("footer center cluster should be valid");
    }

    fn test_remote_view_node(
        node_id: &str,
        parent_id: Option<&str>,
        kind: RemoteViewNodeKind,
    ) -> RemoteViewNode {
        RemoteViewNode {
            node_id: node_id.into(),
            parent_id: parent_id.map(ToOwned::to_owned),
            kind,
            properties: Vec::new(),
        }
    }

    #[test]
    fn context_menu_entries_include_item_tab_menus_and_actions() {
        let mut registry = RemoteUiRegistry::default();
        let mut remote_ui = RemoteUiManifest::default();
        remote_ui.commands.insert(
            "pin-tab".into(),
            extension::ExtensionCommandManifestEntry {
                title: "Pin Tab".into(),
                description: "Pin the current tab".into(),
                palette: false,
                input_schema: None,
                when: None,
            },
        );
        remote_ui.commands.insert(
            "copy-path".into(),
            extension::ExtensionCommandManifestEntry {
                title: "Copy Path".into(),
                description: "Copy the active tab path".into(),
                palette: false,
                input_schema: None,
                when: None,
            },
        );
        remote_ui.menus.push(ExtensionMenuManifestEntry {
            id: "pin-tab-menu".into(),
            location: MenuLocation::ItemTabContext,
            title: "Pin From Extension".into(),
            command: "pin-tab".into(),
            panel: None,
            group: None,
            priority: 20,
            when: None,
        });
        remote_ui
            .context_actions
            .push(extension::ExtensionContextActionManifestEntry {
                id: "copy-path-action".into(),
                target: ContextActionTarget::ItemTab,
                title: "Copy Path From Extension".into(),
                command: "copy-path".into(),
                group: None,
                priority: 10,
                when: None,
            });

        registry.register_extension("remote-ui".into(), remote_ui);

        let entries = registry.context_menu_entries(
            MenuLocation::ItemTabContext,
            ContextActionTarget::ItemTab,
            None,
        );

        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.title.to_string())
                .collect::<Vec<_>>(),
            vec!["Copy Path From Extension", "Pin From Extension"]
        );
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.command_id.as_ref())
                .collect::<Vec<_>>(),
            vec!["remote-ui::copy-path", "remote-ui::pin-tab"]
        );
    }

    #[test]
    fn context_menu_entries_include_editor_menus_and_actions() {
        let mut registry = RemoteUiRegistry::default();
        let mut remote_ui = RemoteUiManifest::default();
        remote_ui.commands.insert(
            "explain-selection".into(),
            extension::ExtensionCommandManifestEntry {
                title: "Explain Selection".into(),
                description: "Explain the selected code".into(),
                palette: false,
                input_schema: None,
                when: None,
            },
        );
        remote_ui.commands.insert(
            "copy-symbol".into(),
            extension::ExtensionCommandManifestEntry {
                title: "Copy Symbol".into(),
                description: "Copy the selected symbol".into(),
                palette: false,
                input_schema: None,
                when: None,
            },
        );
        remote_ui.menus.push(ExtensionMenuManifestEntry {
            id: "editor-explain-menu".into(),
            location: MenuLocation::EditorContext,
            title: "Explain From Extension".into(),
            command: "explain-selection".into(),
            panel: None,
            group: None,
            priority: 20,
            when: None,
        });
        remote_ui
            .context_actions
            .push(extension::ExtensionContextActionManifestEntry {
                id: "editor-copy-action".into(),
                target: ContextActionTarget::Editor,
                title: "Copy Symbol From Extension".into(),
                command: "copy-symbol".into(),
                group: None,
                priority: 10,
                when: None,
            });

        registry.register_extension("remote-ui".into(), remote_ui);

        let entries = registry.context_menu_entries(
            MenuLocation::EditorContext,
            ContextActionTarget::Editor,
            None,
        );

        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.title.to_string())
                .collect::<Vec<_>>(),
            vec!["Copy Symbol From Extension", "Explain From Extension"]
        );
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.command_id.as_ref())
                .collect::<Vec<_>>(),
            vec!["remote-ui::copy-symbol", "remote-ui::explain-selection"]
        );
    }

    #[test]
    fn context_menu_entries_include_project_panel_menus_and_actions() {
        let mut registry = RemoteUiRegistry::default();
        let mut remote_ui = RemoteUiManifest::default();
        remote_ui.commands.insert(
            "reveal-children".into(),
            extension::ExtensionCommandManifestEntry {
                title: "Reveal Children".into(),
                description: "Reveal descendant entries".into(),
                palette: false,
                input_schema: None,
                when: None,
            },
        );
        remote_ui.commands.insert(
            "copy-entry-path".into(),
            extension::ExtensionCommandManifestEntry {
                title: "Copy Entry Path".into(),
                description: "Copy the selected project entry path".into(),
                palette: false,
                input_schema: None,
                when: None,
            },
        );
        remote_ui.menus.push(ExtensionMenuManifestEntry {
            id: "project-panel-reveal-menu".into(),
            location: MenuLocation::ProjectPanelContext,
            title: "Reveal Children From Extension".into(),
            command: "reveal-children".into(),
            panel: None,
            group: None,
            priority: 20,
            when: None,
        });
        remote_ui
            .context_actions
            .push(extension::ExtensionContextActionManifestEntry {
                id: "project-panel-copy-action".into(),
                target: ContextActionTarget::ProjectPanel,
                title: "Copy Entry Path From Extension".into(),
                command: "copy-entry-path".into(),
                group: None,
                priority: 10,
                when: None,
            });

        registry.register_extension("remote-ui".into(), remote_ui);

        let entries = registry.context_menu_entries(
            MenuLocation::ProjectPanelContext,
            ContextActionTarget::ProjectPanel,
            None,
        );

        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.title.to_string())
                .collect::<Vec<_>>(),
            vec![
                "Copy Entry Path From Extension",
                "Reveal Children From Extension",
            ]
        );
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.command_id.as_ref())
                .collect::<Vec<_>>(),
            vec!["remote-ui::copy-entry-path", "remote-ui::reveal-children"]
        );
    }

    #[test]
    fn context_menu_entries_filter_panel_overflow_entries_by_target_panel() {
        let mut registry = RemoteUiRegistry::default();
        let mut remote_ui = RemoteUiManifest::default();
        remote_ui.commands.insert(
            "open-panel-settings".into(),
            extension::ExtensionCommandManifestEntry {
                title: "Panel Settings".into(),
                description: "Open settings for this panel".into(),
                palette: false,
                input_schema: None,
                when: None,
            },
        );
        remote_ui.commands.insert(
            "open-other-panel-settings".into(),
            extension::ExtensionCommandManifestEntry {
                title: "Other Panel Settings".into(),
                description: "Open settings for another panel".into(),
                palette: false,
                input_schema: None,
                when: None,
            },
        );
        remote_ui.menus.push(ExtensionMenuManifestEntry {
            id: "current-panel-menu".into(),
            location: MenuLocation::PanelOverflow,
            title: "Current Panel Entry".into(),
            command: "open-panel-settings".into(),
            panel: Some("sample-panel".into()),
            group: None,
            priority: 10,
            when: None,
        });
        remote_ui.menus.push(ExtensionMenuManifestEntry {
            id: "other-panel-menu".into(),
            location: MenuLocation::PanelOverflow,
            title: "Other Panel Entry".into(),
            command: "open-other-panel-settings".into(),
            panel: Some("other-panel".into()),
            group: None,
            priority: 20,
            when: None,
        });

        registry.register_extension("remote-ui".into(), remote_ui);

        let entries = registry.context_menu_entries(
            MenuLocation::PanelOverflow,
            ContextActionTarget::Panel,
            Some("remote-ui::sample-panel"),
        );

        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.title.to_string())
                .collect::<Vec<_>>(),
            vec!["Current Panel Entry"]
        );
    }
}
