mod components;

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use editor::{Editor, EditorElement, EditorStyle};
use gpui::{
    Action, AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyContext,
    ParentElement, Render, SharedString, Styled, TextStyle, UniformListScrollHandle, Window,
    actions, point, prelude::*, px, uniform_list,
};
use project::DirectoryLister;
use ui::{
    ButtonLink, Chip, ScrollableHandle, TintColor, ToggleButtonGroup, ToggleButtonGroupSize,
    ToggleButtonGroupStyle, ToggleButtonSimple, WithScrollbar, prelude::*,
};
use workspace::{
    Workspace,
    item::{Item, ItemEvent},
};
use zed_actions::Plugins;

use components::PluginCard;

actions!(
    plugins_ui,
    [
        /// Installs a plugin from a local directory for development.
        InstallDevPlugin
    ]
);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginSource {
    Registry,
    Development { path: PathBuf },
}

impl PluginSource {
    pub fn label(&self) -> SharedString {
        match self {
            Self::Registry => "Registry plugin".into(),
            Self::Development { path } => {
                SharedString::from(format!("Development: {}", path.display()))
            }
        }
    }

    pub fn summary_label(&self) -> SharedString {
        match self {
            Self::Registry => "Registry".into(),
            Self::Development { .. } => "Development override".into(),
        }
    }

    pub fn development_path(&self) -> Option<&PathBuf> {
        match self {
            Self::Development { path } => Some(path),
            Self::Registry => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginStatus {
    NotInstalled,
    Installing,
    Installed,
    Removing,
    Failed(SharedString),
}

impl PluginStatus {
    pub fn label(&self) -> SharedString {
        match self {
            Self::NotInstalled => "Not installed".into(),
            Self::Installing => "Installing".into(),
            Self::Installed => "Installed".into(),
            Self::Removing => "Removing".into(),
            Self::Failed(message) => SharedString::from(format!("Error: {message}")),
        }
    }

    pub fn is_loading(&self) -> bool {
        matches!(self, Self::Installing | Self::Removing)
    }

    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Installing | Self::Removing)
    }

    pub fn is_installed(&self) -> bool {
        matches!(self, Self::Installed)
    }

    pub fn error_message(&self) -> Option<SharedString> {
        match self {
            Self::Failed(message) => Some(message.clone()),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginPanelRecord {
    pub id: Arc<str>,
    pub title: SharedString,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginRecord {
    pub id: Arc<str>,
    pub name: SharedString,
    pub version: SharedString,
    pub description: Option<SharedString>,
    pub authors: Vec<SharedString>,
    pub repository_url: Option<SharedString>,
    pub homepage_url: Option<SharedString>,
    pub source: PluginSource,
    pub status: PluginStatus,
    pub panels: Vec<PluginPanelRecord>,
}

impl PluginRecord {
    pub fn panel_count(&self) -> usize {
        self.panels.len()
    }

    pub fn panel_count_label(&self) -> SharedString {
        match self.panel_count() {
            0 => "No panels".into(),
            1 => "1 panel".into(),
            count => SharedString::from(format!("{count} panels")),
        }
    }

    pub fn source_badge_label(&self) -> SharedString {
        self.source.summary_label()
    }

    pub fn author_label(&self) -> Option<SharedString> {
        (!self.authors.is_empty()).then(|| {
            SharedString::from(
                self.authors
                    .iter()
                    .map(|author| author.as_ref())
                    .collect::<Vec<_>>()
                    .join(", "),
            )
        })
    }

    pub fn has_development_source(&self) -> bool {
        matches!(self.source, PluginSource::Development { .. })
    }

    pub fn action_label(&self) -> SharedString {
        match self.status {
            PluginStatus::NotInstalled | PluginStatus::Failed(_) => match self.source {
                PluginSource::Registry => "Install".into(),
                PluginSource::Development { .. } => "Install Dev".into(),
            },
            PluginStatus::Installing => match self.source {
                PluginSource::Registry => "Installing".into(),
                PluginSource::Development { .. } => "Installing Dev".into(),
            },
            PluginStatus::Installed => "Remove".into(),
            PluginStatus::Removing => "Removing".into(),
        }
    }

    pub fn action_is_disabled(&self) -> bool {
        self.status.is_busy()
    }

    pub fn action_is_loading(&self) -> bool {
        self.status.is_loading()
    }
}

pub trait PluginStoreApi: 'static {
    fn list_plugins(&self) -> Result<Vec<PluginRecord>>;
    fn install_plugin(&self, plugin_id: &str, cx: &mut App) -> Result<()>;
    fn remove_plugin(&self, plugin_id: &str, cx: &mut App) -> Result<()>;
    fn install_development_plugin(&self, source_directory: &Path, cx: &mut App) -> Result<()>;
    fn open_panel(
        &self,
        plugin_id: &str,
        panel_id: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<()>;
}

pub trait PluginStoreProvider: 'static {
    fn plugin_store(&self, workspace: &Workspace, cx: &App) -> Arc<dyn PluginStoreApi>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PluginFilter {
    All,
    Installed,
    Development,
}

struct NoopPluginStore;

impl PluginStoreApi for NoopPluginStore {
    fn list_plugins(&self) -> Result<Vec<PluginRecord>> {
        Ok(Vec::new())
    }

    fn install_plugin(&self, plugin_id: &str, _cx: &mut App) -> Result<()> {
        anyhow::bail!("plugin store is not configured; cannot install `{plugin_id}`")
    }

    fn remove_plugin(&self, plugin_id: &str, _cx: &mut App) -> Result<()> {
        anyhow::bail!("plugin store is not configured; cannot remove `{plugin_id}`")
    }

    fn install_development_plugin(&self, source_directory: &Path, _cx: &mut App) -> Result<()> {
        anyhow::bail!(
            "plugin store is not configured; cannot install development plugin from `{}`",
            source_directory.display()
        )
    }

    fn open_panel(
        &self,
        plugin_id: &str,
        panel_id: &str,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Result<()> {
        anyhow::bail!(
            "plugin store is not configured; cannot open panel `{panel_id}` for `{plugin_id}`"
        )
    }
}

struct NoopPluginStoreProvider;

impl PluginStoreProvider for NoopPluginStoreProvider {
    fn plugin_store(&self, _workspace: &Workspace, _cx: &App) -> Arc<dyn PluginStoreApi> {
        Arc::new(NoopPluginStore)
    }
}

pub fn init(cx: &mut App) {
    init_with_provider(Arc::new(NoopPluginStoreProvider), cx);
}

pub fn init_with_provider(provider: Arc<dyn PluginStoreProvider>, cx: &mut App) {
    cx.observe_new(move |workspace: &mut Workspace, window, _cx| {
        let Some(_window) = window else {
            return;
        };

        workspace.register_action({
            let provider = provider.clone();
            move |workspace, action: &Plugins, window, cx| {
                let existing = workspace
                    .active_pane()
                    .read(cx)
                    .items()
                    .find_map(|item| item.downcast::<PluginsPage>());

                if let Some(existing) = existing {
                    existing.update(cx, |plugins_page, cx| {
                        if let Some(id) = action.id.as_ref() {
                            plugins_page.focus_plugin(id.clone());
                        }
                        plugins_page.refresh(cx);
                    });

                    workspace.activate_item(&existing, true, true, window, cx);
                } else {
                    let store = provider.plugin_store(workspace, cx);
                    let plugins_page = cx.new(|cx| {
                        let mut plugins_page = PluginsPage::new(store);
                        plugins_page.initialize_query_editor(window, cx);
                        if let Some(id) = action.id.as_ref() {
                            plugins_page.focus_plugin(id.clone());
                            plugins_page.sync_query_editor(window, cx);
                        }
                        plugins_page.refresh(cx);
                        plugins_page
                    });

                    workspace.add_item_to_active_pane(
                        Box::new(plugins_page),
                        None,
                        true,
                        window,
                        cx,
                    );
                }
            }
        });

        workspace.register_action({
            let provider = provider.clone();
            move |workspace, _: &InstallDevPlugin, window, cx| {
                let store = provider.plugin_store(workspace, cx);
                let prompt = workspace.prompt_for_open_path(
                    gpui::PathPromptOptions {
                        files: false,
                        directories: true,
                        multiple: false,
                        prompt: None,
                    },
                    DirectoryLister::Local(
                        workspace.project().clone(),
                        workspace.app_state().fs.clone(),
                    ),
                    window,
                    cx,
                );

                let workspace_handle = cx.entity().downgrade();
                window
                    .spawn(cx, async move |cx| {
                        let plugin_path = match prompt.await.map_err(anyhow::Error::from) {
                            Ok(Some(mut paths)) => paths.pop()?,
                            Ok(None) => return None,
                            Err(error) => {
                                workspace_handle
                                    .update(cx, |workspace, cx| {
                                        workspace.show_portal_error(error.to_string(), cx);
                                    })
                                    .ok();
                                return None;
                            }
                        };

                        match workspace_handle.update(cx, |workspace, cx| {
                            store.install_development_plugin(&plugin_path, cx)?;
                            let plugins_page = workspace
                                .active_pane()
                                .read(cx)
                                .items()
                                .find_map(|item| item.downcast::<PluginsPage>());

                            if let Some(plugins_page) = plugins_page {
                                plugins_page.update(cx, |plugins_page, cx| {
                                    plugins_page.refresh(cx);
                                });
                            }

                            Ok::<(), anyhow::Error>(())
                        }) {
                            Ok(_) => {}
                            Err(error) => {
                                workspace_handle
                                    .update(cx, |workspace, cx| {
                                        workspace.show_error(
                                            &format!(
                                                "Failed to install development plugin: {error}"
                                            ),
                                            cx,
                                        );
                                    })
                                    .ok();
                            }
                        }

                        Some(())
                    })
                    .detach();
            }
        });
    })
    .detach();
}

pub struct PluginsPage {
    store: Arc<dyn PluginStoreApi>,
    list: UniformListScrollHandle,
    plugins: Vec<PluginRecord>,
    filtered_plugin_indices: Vec<usize>,
    query_editor: Option<Entity<Editor>>,
    search_query: String,
    filter: PluginFilter,
    is_loading_plugins: bool,
    has_loaded_plugins: bool,
    load_error: Option<SharedString>,
    focused_plugin_id: Option<Arc<str>>,
}

impl PluginsPage {
    pub fn new(store: Arc<dyn PluginStoreApi>) -> Self {
        Self {
            store,
            list: UniformListScrollHandle::new(),
            plugins: Vec::new(),
            filtered_plugin_indices: Vec::new(),
            query_editor: None,
            search_query: String::new(),
            filter: PluginFilter::All,
            is_loading_plugins: false,
            has_loaded_plugins: false,
            load_error: None,
            focused_plugin_id: None,
        }
    }

    fn initialize_query_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.query_editor.is_some() {
            return;
        }

        let initial_query = if self.search_query.trim().is_empty() {
            self.focused_plugin_id
                .as_ref()
                .map(|plugin_id| format!("id:{plugin_id}"))
        } else {
            Some(self.search_query.clone())
        };

        let query_editor = cx.new(|cx| {
            let mut input = Editor::single_line(window, cx);
            input.set_placeholder_text("Search plugins...", window, cx);
            if let Some(initial_query) = initial_query.as_ref() {
                input.set_text(initial_query.clone(), window, cx);
            }
            input
        });
        cx.subscribe(&query_editor, Self::on_query_change).detach();
        self.query_editor = Some(query_editor);
        self.search_query = initial_query.unwrap_or_default();
        self.refilter_plugins();
    }

    pub fn reload_from_store(&mut self) -> Result<()> {
        self.plugins = self.store.list_plugins()?;
        self.plugins.sort_by(|left, right| {
            left.name
                .as_ref()
                .to_lowercase()
                .cmp(&right.name.as_ref().to_lowercase())
                .then_with(|| {
                    left.id
                        .as_ref()
                        .to_lowercase()
                        .cmp(&right.id.as_ref().to_lowercase())
                })
        });
        self.refilter_plugins();
        self.load_error = None;
        self.has_loaded_plugins = true;
        Ok(())
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.is_loading_plugins = true;
        if let Err(error) = self.reload_from_store() {
            self.plugins.clear();
            self.filtered_plugin_indices.clear();
            self.has_loaded_plugins = true;
            self.load_error = Some(SharedString::from(error.to_string()));
        }
        self.is_loading_plugins = false;
        cx.notify();
    }

    pub fn install_plugin(&mut self, plugin_id: &str, cx: &mut App) -> Result<()> {
        self.store.install_plugin(plugin_id, cx)?;
        self.reload_from_store()
    }

    pub fn remove_plugin(&mut self, plugin_id: &str, cx: &mut App) -> Result<()> {
        self.store.remove_plugin(plugin_id, cx)?;
        self.reload_from_store()
    }

    pub fn install_development_plugin(
        &mut self,
        source_directory: &Path,
        cx: &mut App,
    ) -> Result<()> {
        self.store
            .install_development_plugin(source_directory, cx)?;
        self.reload_from_store()
    }

    pub fn open_panel(
        &mut self,
        plugin_id: &str,
        panel_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.store.open_panel(plugin_id, panel_id, window, cx)?;
        self.reload_from_store()
    }

    pub fn focus_plugin(&mut self, plugin_id: impl Into<Arc<str>>) {
        let plugin_id = plugin_id.into();
        self.search_query = format!("id:{plugin_id}");
        self.focused_plugin_id = Some(plugin_id);
        self.refilter_plugins();
    }

    fn sync_query_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(query_editor) = self.query_editor.as_ref() else {
            return;
        };

        let current_text = query_editor.read(cx).text(cx);
        if current_text != self.search_query {
            query_editor.update(cx, |query_editor, cx| {
                query_editor.set_text(self.search_query.clone(), window, cx);
            });
        }
    }

    pub fn focused_plugin_id(&self) -> Option<&str> {
        self.focused_plugin_id.as_deref()
    }

    pub fn plugin_records(&self) -> &[PluginRecord] {
        &self.plugins
    }

    fn search_query(&self) -> Option<&str> {
        let search = self.search_query.trim();
        if search.is_empty() {
            None
        } else {
            Some(search)
        }
    }

    fn refilter_plugins(&mut self) {
        let search_query = self.search_query();
        let filter = self.filter;

        self.filtered_plugin_indices = self
            .plugins
            .iter()
            .enumerate()
            .filter(|(_, plugin)| {
                Self::matches_filter(plugin, filter) && Self::matches_search(plugin, search_query)
            })
            .map(|(index, _)| index)
            .collect();
    }

    fn matches_filter(plugin: &PluginRecord, filter: PluginFilter) -> bool {
        match filter {
            PluginFilter::All => true,
            PluginFilter::Installed => plugin.status.is_installed(),
            PluginFilter::Development => {
                matches!(plugin.source, PluginSource::Development { .. })
            }
        }
    }

    fn matches_search(plugin: &PluginRecord, search_query: Option<&str>) -> bool {
        let Some(search_query) = search_query else {
            return true;
        };

        if let Some(plugin_id) = search_query.strip_prefix("id:") {
            return plugin.id.as_ref().eq_ignore_ascii_case(plugin_id.trim());
        }

        let query = search_query.to_lowercase();
        let search_terms = query
            .split_whitespace()
            .map(str::trim)
            .filter(|term| !term.is_empty())
            .collect::<Vec<_>>();

        let mut haystack = vec![
            plugin.id.to_string(),
            plugin.name.to_string(),
            plugin.version.to_string(),
            plugin.source.label().to_string(),
            plugin.source.summary_label().to_string(),
            plugin.status.label().to_string(),
            plugin.panel_count_label().to_string(),
        ];
        if let Some(description) = plugin.description.as_ref() {
            haystack.push(description.to_string());
        }
        if let Some(path) = plugin.source.development_path() {
            haystack.push(path.display().to_string());
        }
        haystack.extend(plugin.panels.iter().map(|panel| panel.title.to_string()));
        haystack.extend(plugin.authors.iter().map(ToString::to_string));
        if let Some(repository_url) = plugin.repository_url.as_ref() {
            haystack.push(repository_url.to_string());
        }
        if let Some(homepage_url) = plugin.homepage_url.as_ref() {
            haystack.push(homepage_url.to_string());
        }

        let haystack = haystack.join(" ").to_lowercase();
        search_terms
            .into_iter()
            .all(|search_term| haystack.contains(search_term))
    }

    fn scroll_to_top(&mut self, cx: &mut Context<Self>) {
        self.list.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }

    fn on_query_change(
        &mut self,
        query_editor: Entity<Editor>,
        event: &editor::EditorEvent,
        cx: &mut Context<Self>,
    ) {
        if let editor::EditorEvent::Edited { .. } = event {
            self.search_query = query_editor.read(cx).text(cx);
            self.refilter_plugins();
            self.scroll_to_top(cx);
        }
    }

    fn render_search(&self, cx: &mut Context<Self>) -> Div {
        let Some(query_editor) = self.query_editor.as_ref() else {
            return h_flex()
                .h_8()
                .flex_1()
                .pl_1p5()
                .pr_2()
                .gap_2()
                .border_1()
                .border_color(cx.theme().colors().border)
                .rounded_md()
                .child(Icon::new(IconName::MagnifyingGlass).color(Color::Muted))
                .child(
                    Label::new("Search plugins...")
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                );
        };

        let mut key_context = KeyContext::new_with_defaults();
        key_context.add("BufferSearchBar");

        h_flex()
            .key_context(key_context)
            .h_8()
            .min_w(rems_from_px(384.))
            .flex_1()
            .pl_1p5()
            .pr_2()
            .gap_2()
            .border_1()
            .border_color(cx.theme().colors().border)
            .rounded_md()
            .child(Icon::new(IconName::MagnifyingGlass).color(Color::Muted))
            .child(self.render_text_input(query_editor, cx))
    }

    fn render_text_input(
        &self,
        editor: &Entity<Editor>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let text_style = TextStyle {
            color: if editor.read(cx).read_only(cx) {
                cx.theme().colors().text_disabled
            } else {
                cx.theme().colors().text
            },
            ..Default::default()
        };

        EditorElement::new(
            editor,
            EditorStyle {
                background: cx.theme().colors().editor_background,
                local_player: cx.theme().players().local(),
                text: text_style,
                ..Default::default()
            },
        )
    }

    fn render_empty_state(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        let has_search = self.search_query().is_some();

        let message = if self.is_loading_plugins || !self.has_loaded_plugins {
            "Loading plugins..."
        } else if self.load_error.is_some() {
            "Failed to load plugins. Please try again."
        } else {
            match self.filter {
                PluginFilter::All => {
                    if has_search {
                        "No plugins match your search."
                    } else {
                        "No plugins available."
                    }
                }
                PluginFilter::Installed => {
                    if has_search {
                        "No installed plugins match your search."
                    } else {
                        "No installed plugins."
                    }
                }
                PluginFilter::Development => {
                    if has_search {
                        "No development plugins match your search."
                    } else {
                        "No development plugins."
                    }
                }
            }
        };

        h_flex()
            .py_4()
            .gap_1p5()
            .when(self.load_error.is_some(), |this| {
                this.child(
                    Icon::new(IconName::Warning)
                        .size(IconSize::Small)
                        .color(Color::Warning),
                )
            })
            .child(Label::new(message))
    }

    fn render_plugins(
        &mut self,
        range: Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        range
            .map(|index| {
                let Some(plugin_index) = self.filtered_plugin_indices.get(index).copied() else {
                    return self.render_missing_plugin();
                };
                let Some(plugin) = self.plugins.get(plugin_index) else {
                    return self.render_missing_plugin();
                };
                self.render_plugin_card(plugin, cx).into_any_element()
            })
            .collect()
    }

    fn render_missing_plugin(&self) -> AnyElement {
        PluginCard::new()
            .child(
                Label::new("Missing plugin entry.")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .into_any_element()
    }

    fn render_plugin_card(
        &self,
        plugin: &PluginRecord,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let plugin_id = plugin.id.clone();
        let action_label = plugin.action_label();
        let action_is_disabled = plugin.action_is_disabled();
        let action_is_loading = plugin.action_is_loading();
        let action_is_destructive = plugin.status.is_installed();
        let status_label = plugin.status.label();
        let error_message = plugin.status.error_message();
        let development_path = plugin.source.development_path().cloned();
        let description = plugin.description.clone();
        let panels = plugin.panels.clone();
        let title = plugin.name.clone();
        let version = plugin.version.clone();
        let author_label = plugin.author_label();
        let repository_url = plugin.repository_url.clone();
        let homepage_url = plugin.homepage_url.clone();
        let source = plugin.source.clone();
        let status = plugin.status.clone();
        let is_installed = plugin.status.is_installed();
        let panel_count_label = plugin.panel_count_label();
        let source_badge_label = plugin.source_badge_label();
        let plugin_id_for_action = plugin_id.clone();

        let card = if plugin.has_development_source() {
            PluginCard::development()
        } else {
            PluginCard::new()
        };

        card.child(
            h_flex()
                .justify_between()
                .gap_3()
                .child(
                    v_flex()
                        .min_w_0()
                        .gap_1()
                        .child(
                            h_flex()
                                .min_w_0()
                                .gap_2()
                                .items_end()
                                .child(Headline::new(title).size(HeadlineSize::Small))
                                .child(
                                    Label::new(version)
                                        .size(LabelSize::Small)
                                        .color(Color::Muted),
                                ),
                        )
                        .child(
                            h_flex().gap_1().children([
                                Chip::new(source_badge_label)
                                    .label_size(LabelSize::XSmall)
                                    .bg_color(if plugin.has_development_source() {
                                        cx.theme().status().warning_background.opacity(0.15)
                                    } else {
                                        cx.theme().colors().element_background
                                    })
                                    .border_color(if plugin.has_development_source() {
                                        cx.theme().status().warning_border
                                    } else {
                                        cx.theme().colors().border
                                    }),
                                Chip::new(status_label).label_size(LabelSize::XSmall),
                                Chip::new(panel_count_label).label_size(LabelSize::XSmall),
                            ]),
                        ),
                )
                .child(
                    Button::new(
                        SharedString::from(format!("plugin-action-{}", plugin_id_for_action)),
                        action_label,
                    )
                    .style(if action_is_destructive {
                        ButtonStyle::OutlinedGhost
                    } else {
                        ButtonStyle::Tinted(TintColor::Accent)
                    })
                    .disabled(action_is_disabled)
                    .loading(action_is_loading)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let result = this.handle_plugin_action(
                            plugin_id_for_action.clone(),
                            source.clone(),
                            status.clone(),
                            cx,
                        );

                        if let Err(error) = result {
                            this.load_error = Some(SharedString::from(error.to_string()));
                        }

                        cx.notify();
                    })),
                ),
        )
        .child(
            h_flex()
                .min_w_0()
                .gap_2()
                .justify_between()
                .children(description.map(|description| {
                    Label::new(description)
                        .size(LabelSize::Small)
                        .color(Color::Muted)
                        .truncate()
                })),
        )
        .when(
            author_label.is_some() || repository_url.is_some() || homepage_url.is_some(),
            |this| {
                this.child(
                    h_flex()
                        .min_w_0()
                        .justify_between()
                        .gap_2()
                        .children(author_label.map(|author_label| {
                            h_flex()
                                .min_w_0()
                                .gap_1()
                                .child(
                                    Icon::new(IconName::Person)
                                        .size(IconSize::XSmall)
                                        .color(Color::Muted),
                                )
                                .child(
                                    Label::new(author_label)
                                        .size(LabelSize::Small)
                                        .color(Color::Muted)
                                        .truncate(),
                                )
                        }))
                        .child(
                            h_flex()
                                .gap_2()
                                .children(repository_url.map(|repository_url| {
                                    ButtonLink::new("Repository", repository_url)
                                }))
                                .children(
                                    homepage_url.map(|homepage_url| {
                                        ButtonLink::new("Homepage", homepage_url)
                                    }),
                                ),
                        ),
                )
            },
        )
        .when_some(development_path, |this, path| {
            this.child(
                Label::new(format!("Development path: {}", path.display()))
                    .size(LabelSize::Small)
                    .color(Color::Warning)
                    .truncate(),
            )
        })
        .when(plugin.has_development_source(), |this| {
            this.child(
                Label::new("Development override")
                    .size(LabelSize::Small)
                    .color(Color::Warning)
                    .truncate(),
            )
        })
        .when_some(error_message, |this, error| {
            this.child(
                Label::new(format!("Error: {error}"))
                    .size(LabelSize::Small)
                    .color(Color::Error)
                    .truncate(),
            )
        })
        .when(is_installed && !panels.is_empty(), |this| {
            this.child(h_flex().gap_2().children(panels.into_iter().map(|panel| {
                let plugin_id = plugin_id.clone();
                let panel_id = panel.id.clone();
                Button::new(
                    SharedString::from(format!("plugin-open-{}-{}", plugin_id, panel_id)),
                    format!("Open {}", panel.title),
                )
                .style(ButtonStyle::Subtle)
                .on_click(cx.listener(move |this, _, window, cx| {
                    if let Err(error) = this.open_panel(&plugin_id, &panel_id, window, cx) {
                        this.load_error = Some(SharedString::from(error.to_string()));
                        cx.notify();
                    }
                }))
            })))
        })
    }

    fn install_or_remove_registry_plugin(
        &mut self,
        plugin_id: Arc<str>,
        status: PluginStatus,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        match status {
            PluginStatus::Installed | PluginStatus::Removing => self.remove_plugin(&plugin_id, cx),
            PluginStatus::NotInstalled | PluginStatus::Installing | PluginStatus::Failed(_) => {
                self.install_plugin(&plugin_id, cx)
            }
        }
    }

    fn handle_plugin_action(
        &mut self,
        plugin_id: Arc<str>,
        source: PluginSource,
        status: PluginStatus,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        match source {
            PluginSource::Registry => self.install_or_remove_registry_plugin(plugin_id, status, cx),
            PluginSource::Development { path } => match status {
                PluginStatus::Installed | PluginStatus::Removing => {
                    self.remove_plugin(&plugin_id, cx)
                }
                PluginStatus::NotInstalled | PluginStatus::Installing | PluginStatus::Failed(_) => {
                    self.install_development_plugin(&path, cx)
                }
            },
        }
    }

    fn total_plugins_label(total: usize) -> SharedString {
        match total {
            0 => "No plugins".into(),
            1 => "1 plugin".into(),
            count => SharedString::from(format!("{count} plugins")),
        }
    }

    fn installed_plugins_label(plugins: &[PluginRecord]) -> SharedString {
        let count = plugins
            .iter()
            .filter(|plugin| plugin.status.is_installed())
            .count();
        match count {
            0 => "0 installed".into(),
            1 => "1 installed".into(),
            count => SharedString::from(format!("{count} installed")),
        }
    }

    fn development_plugins_label(plugins: &[PluginRecord]) -> SharedString {
        let count = plugins
            .iter()
            .filter(|plugin| matches!(plugin.source, PluginSource::Development { .. }))
            .count();
        match count {
            0 => "0 development".into(),
            1 => "1 development".into(),
            count => SharedString::from(format!("{count} development")),
        }
    }

    fn filter_label(label: &str, count: usize) -> SharedString {
        match count {
            0 => SharedString::from(format!("{label} (0)")),
            1 => SharedString::from(format!("{label} (1)")),
            count => SharedString::from(format!("{label} ({count})")),
        }
    }
}

impl Render for PluginsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_query_editor(window, cx);

        v_flex()
            .size_full()
            .bg(cx.theme().colors().editor_background)
            .child(
                v_flex()
                    .p_4()
                    .gap_4()
                    .border_b_1()
                    .border_color(cx.theme().colors().border_variant)
                    .child(
                        h_flex()
                            .w_full()
                            .gap_1p5()
                            .justify_between()
                            .child(Headline::new("Plugins").size(HeadlineSize::Large))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Button::new("plugins-install-dev", "Install Dev Plugin")
                                            .style(ButtonStyle::Outlined)
                                            .size(ButtonSize::Medium)
                                            .on_click(move |_event, window, cx| {
                                                window.dispatch_action(
                                                    InstallDevPlugin.boxed_clone(),
                                                    cx,
                                                );
                                            }),
                                    )
                                    .child(
                                        Chip::new(Self::total_plugins_label(self.plugins.len()))
                                            .label_size(LabelSize::XSmall),
                                    )
                                    .child(
                                        Chip::new(Self::installed_plugins_label(&self.plugins))
                                            .label_size(LabelSize::XSmall),
                                    )
                                    .child(
                                        Chip::new(Self::development_plugins_label(&self.plugins))
                                            .label_size(LabelSize::XSmall),
                                    ),
                            ),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .flex_wrap()
                            .gap_2()
                            .child(self.render_search(cx))
                            .child(
                                div().child(
                                    ToggleButtonGroup::single_row(
                                        "plugin-filter-buttons",
                                        [
                                            ToggleButtonSimple::new(
                                                Self::filter_label("All", self.plugins.len()),
                                                cx.listener(|this, _event, _, cx| {
                                                    this.filter = PluginFilter::All;
                                                    this.refilter_plugins();
                                                    this.scroll_to_top(cx);
                                                }),
                                            ),
                                            ToggleButtonSimple::new(
                                                Self::filter_label(
                                                    "Installed",
                                                    self.plugins
                                                        .iter()
                                                        .filter(|plugin| {
                                                            plugin.status.is_installed()
                                                        })
                                                        .count(),
                                                ),
                                                cx.listener(|this, _event, _, cx| {
                                                    this.filter = PluginFilter::Installed;
                                                    this.refilter_plugins();
                                                    this.scroll_to_top(cx);
                                                }),
                                            ),
                                            ToggleButtonSimple::new(
                                                Self::filter_label(
                                                    "Development",
                                                    self.plugins
                                                        .iter()
                                                        .filter(|plugin| {
                                                            matches!(
                                                                plugin.source,
                                                                PluginSource::Development { .. }
                                                            )
                                                        })
                                                        .count(),
                                                ),
                                                cx.listener(|this, _event, _, cx| {
                                                    this.filter = PluginFilter::Development;
                                                    this.refilter_plugins();
                                                    this.scroll_to_top(cx);
                                                }),
                                            ),
                                        ],
                                    )
                                    .style(ToggleButtonGroupStyle::Outlined)
                                    .size(ToggleButtonGroupSize::Custom(rems_from_px(30.)))
                                    .label_size(LabelSize::Default)
                                    .auto_width()
                                    .selected_index(match self.filter {
                                        PluginFilter::All => 0,
                                        PluginFilter::Installed => 1,
                                        PluginFilter::Development => 2,
                                    })
                                    .into_any_element(),
                                ),
                            ),
                    ),
            )
            .when_some(self.load_error.clone(), |this, error| {
                this.child(
                    div().px_4().pt_4().child(
                        Label::new(format!("Failed to load plugins: {error}"))
                            .size(LabelSize::Small)
                            .color(Color::Error),
                    ),
                )
            })
            .child(v_flex().px_4().size_full().overflow_y_hidden().map(|this| {
                let count = self.filtered_plugin_indices.len();

                if count == 0 {
                    this.child(self.render_empty_state(cx)).into_any_element()
                } else {
                    let scroll_handle = &self.list;
                    this.child(
                        uniform_list("plugin-entries", count, cx.processor(Self::render_plugins))
                            .flex_grow()
                            .pb_4()
                            .track_scroll(scroll_handle),
                    )
                    .vertical_scrollbar_for(scroll_handle, window, cx)
                    .into_any_element()
                }
            }))
    }
}

impl EventEmitter<ItemEvent> for PluginsPage {}

impl Focusable for PluginsPage {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.query_editor
            .as_ref()
            .map(|query_editor| query_editor.read(cx).focus_handle(cx))
            .unwrap_or_else(|| cx.focus_handle())
    }
}

impl Item for PluginsPage {
    type Event = ItemEvent;

    fn tab_content_text(&self, _detail: usize, _cx: &App) -> SharedString {
        "Plugins".into()
    }

    fn telemetry_event_text(&self) -> Option<&'static str> {
        Some("Plugins Page Opened")
    }

    fn show_toolbar(&self) -> bool {
        false
    }

    fn to_item_events(event: &Self::Event, f: &mut dyn FnMut(ItemEvent)) {
        f(*event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use std::path::PathBuf;

    #[test]
    fn install_plugin_surfaces_store_errors() {
        let store = Arc::new(EmptyStore);
        let mut page = PluginsPage::new(store);
        let mut app = gpui::TestApp::new();
        assert!(app.update(|cx| page.install_plugin("plugin", cx)).is_err());
    }

    #[test]
    fn focus_plugin_tracks_the_selected_plugin_identifier() {
        let store = Arc::new(EmptyStore);
        let mut page = PluginsPage::new(store);
        assert_eq!(page.focused_plugin_id(), None);

        page.focus_plugin("selected");

        assert_eq!(page.focused_plugin_id(), Some("selected"));
    }

    #[test]
    fn refilter_plugins_supports_all_installed_and_development_filters() {
        let store = Arc::new(EmptyStore);
        let mut page = PluginsPage::new(store);
        page.plugins = vec![
            registry_plugin("registry-alpha", PluginStatus::Installed),
            registry_plugin("registry-beta", PluginStatus::NotInstalled),
            development_plugin("dev-gamma", PluginStatus::Installed),
        ];

        page.filter = PluginFilter::All;
        page.refilter_plugins();
        assert_eq!(page.filtered_plugin_indices, vec![0, 1, 2]);

        page.filter = PluginFilter::Installed;
        page.refilter_plugins();
        assert_eq!(page.filtered_plugin_indices, vec![0, 2]);

        page.filter = PluginFilter::Development;
        page.refilter_plugins();
        assert_eq!(page.filtered_plugin_indices, vec![2]);
    }

    #[test]
    fn matches_search_supports_text_terms_and_id_queries() {
        let plugin = development_plugin("codex-usage-plugin", PluginStatus::Installed);

        assert!(PluginsPage::matches_search(&plugin, Some("codex usage")));
        assert!(PluginsPage::matches_search(
            &plugin,
            Some("id:codex-usage-plugin")
        ));
        assert!(PluginsPage::matches_search(
            &plugin,
            Some("/tmp/codex-usage-plugin")
        ));
        assert!(PluginsPage::matches_search(
            &plugin,
            Some("installed override")
        ));
        assert!(PluginsPage::matches_search(
            &plugin,
            Some("development team")
        ));
        assert!(PluginsPage::matches_search(
            &plugin,
            Some("github.com/example/dev-plugin")
        ));
        assert!(!PluginsPage::matches_search(&plugin, Some("nonexistent")));
        assert!(!PluginsPage::matches_search(
            &plugin,
            Some("id:other-plugin")
        ));
    }

    #[test]
    fn plugin_metadata_labels_include_counts_and_override_state() {
        let plugin = development_plugin("codex-usage-plugin", PluginStatus::Installed);

        assert_eq!(
            plugin.source_badge_label(),
            SharedString::from("Development override")
        );
        assert_eq!(plugin.panel_count_label(), SharedString::from("1 panel"));
        assert_eq!(
            plugin.author_label(),
            Some(SharedString::from("Development Team"))
        );
        assert!(plugin.has_development_source());
    }

    #[test]
    fn filter_and_header_labels_include_counts() {
        let plugins = vec![
            registry_plugin("registry-alpha", PluginStatus::Installed),
            registry_plugin("registry-beta", PluginStatus::NotInstalled),
            development_plugin("dev-gamma", PluginStatus::Installed),
        ];

        assert_eq!(
            PluginsPage::total_plugins_label(plugins.len()),
            SharedString::from("3 plugins")
        );
        assert_eq!(
            PluginsPage::installed_plugins_label(&plugins),
            SharedString::from("2 installed")
        );
        assert_eq!(
            PluginsPage::development_plugins_label(&plugins),
            SharedString::from("1 development")
        );
        assert_eq!(
            PluginsPage::filter_label("Installed", 2),
            SharedString::from("Installed (2)")
        );
    }

    fn registry_plugin(id: &str, status: PluginStatus) -> PluginRecord {
        PluginRecord {
            id: Arc::from(id),
            name: SharedString::from(format!("Registry {id}")),
            version: SharedString::from("1.0.0"),
            description: Some(SharedString::from("Registry plugin")),
            authors: vec!["Registry Team".into()],
            repository_url: Some("https://github.com/example/registry-plugin".into()),
            homepage_url: Some("https://example.com/registry-plugin".into()),
            source: PluginSource::Registry,
            status,
            panels: vec![PluginPanelRecord {
                id: Arc::from(format!("{id}-panel")),
                title: SharedString::from("Registry Panel"),
            }],
        }
    }

    fn development_plugin(id: &str, status: PluginStatus) -> PluginRecord {
        PluginRecord {
            id: Arc::from(id),
            name: SharedString::from(format!("Development {id}")),
            version: SharedString::from("0.1.0"),
            description: Some(SharedString::from("Development plugin")),
            authors: vec!["Development Team".into()],
            repository_url: Some("https://github.com/example/dev-plugin".into()),
            homepage_url: Some("https://example.com/dev-plugin".into()),
            source: PluginSource::Development {
                path: PathBuf::from(format!("/tmp/{id}")),
            },
            status,
            panels: vec![PluginPanelRecord {
                id: Arc::from(format!("{id}-panel")),
                title: SharedString::from("Development Panel"),
            }],
        }
    }

    struct EmptyStore;

    impl PluginStoreApi for EmptyStore {
        fn list_plugins(&self) -> Result<Vec<PluginRecord>> {
            Ok(Vec::new())
        }

        fn install_plugin(&self, _plugin_id: &str, _cx: &mut App) -> Result<()> {
            Err(anyhow!("not implemented"))
        }

        fn remove_plugin(&self, _plugin_id: &str, _cx: &mut App) -> Result<()> {
            Err(anyhow!("not implemented"))
        }

        fn install_development_plugin(
            &self,
            _source_directory: &Path,
            _cx: &mut App,
        ) -> Result<()> {
            Err(anyhow!("not implemented"))
        }

        fn open_panel(
            &self,
            _plugin_id: &str,
            _panel_id: &str,
            _window: &mut Window,
            _cx: &mut App,
        ) -> Result<()> {
            Err(anyhow!("not implemented"))
        }
    }
}
