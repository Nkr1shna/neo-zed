use std::ffi::OsStr;
use std::fmt;
use std::path::Component;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow, bail};
use cloud_api_types::ExtensionProvides;
use collections::{BTreeMap, BTreeSet, HashMap};
use fs::Fs;
use language::LanguageName;
use lsp::LanguageServerName;
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::ExtensionCapability;

/// This is the old version of the extension manifest, from when it was `extension.json`.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct OldExtensionManifest {
    pub name: String,
    pub version: Arc<str>,

    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,

    #[serde(default)]
    pub themes: BTreeMap<Arc<str>, PathBuf>,
    #[serde(default)]
    pub languages: BTreeMap<Arc<str>, PathBuf>,
    #[serde(default)]
    pub grammars: BTreeMap<Arc<str>, PathBuf>,
}

/// The schema version of the [`ExtensionManifest`].
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy, Serialize, Deserialize)]
pub struct SchemaVersion(pub i32);

impl fmt::Display for SchemaVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl SchemaVersion {
    pub const ZERO: Self = Self(0);

    pub fn is_v0(&self) -> bool {
        self == &Self::ZERO
    }
}

// TODO: We should change this to just always be a Vec<PathBuf> once we bump the
// extension.toml schema version to 2
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ExtensionSnippets {
    Single(PathBuf),
    Multiple(Vec<PathBuf>),
}

impl ExtensionSnippets {
    pub fn paths(&self) -> impl Iterator<Item = &PathBuf> {
        match self {
            ExtensionSnippets::Single(path) => std::slice::from_ref(path).iter(),
            ExtensionSnippets::Multiple(paths) => paths.iter(),
        }
    }
}

impl From<&str> for ExtensionSnippets {
    fn from(value: &str) -> Self {
        ExtensionSnippets::Single(value.into())
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct ExtensionManifest {
    pub id: Arc<str>,
    pub name: String,
    pub version: Arc<str>,
    pub schema_version: SchemaVersion,

    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub lib: LibManifestEntry,

    #[serde(default)]
    pub themes: Vec<PathBuf>,
    #[serde(default)]
    pub icon_themes: Vec<PathBuf>,
    #[serde(default)]
    pub languages: Vec<PathBuf>,
    #[serde(default)]
    pub grammars: BTreeMap<Arc<str>, GrammarManifestEntry>,
    #[serde(default)]
    pub language_servers: BTreeMap<LanguageServerName, LanguageServerManifestEntry>,
    #[serde(default)]
    pub context_servers: BTreeMap<Arc<str>, ContextServerManifestEntry>,
    #[serde(default)]
    pub agent_servers: BTreeMap<Arc<str>, AgentServerManifestEntry>,
    #[serde(default)]
    pub slash_commands: BTreeMap<Arc<str>, SlashCommandManifestEntry>,
    #[serde(default)]
    pub snippets: Option<ExtensionSnippets>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidecar: Option<ExtensionSidecarManifestEntry>,
    #[serde(default)]
    pub capabilities: Vec<ExtensionCapability>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub debug_adapters: BTreeMap<Arc<str>, DebugAdapterManifestEntry>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub debug_locators: BTreeMap<Arc<str>, DebugLocatorManifestEntry>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub language_model_providers: BTreeMap<Arc<str>, LanguageModelProviderManifestEntry>,
}

#[derive(Clone, Default, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct RemoteUiManifest {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub commands: BTreeMap<Arc<str>, ExtensionCommandManifestEntry>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub panels: BTreeMap<Arc<str>, ExtensionPanelManifestEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub titlebar_widgets: Vec<TitlebarWidgetManifestEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub footer_widgets: Vec<FooterWidgetManifestEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub menus: Vec<ExtensionMenuManifestEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_actions: Vec<ExtensionContextActionManifestEntry>,
}

impl RemoteUiManifest {
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
            && self.panels.is_empty()
            && self.titlebar_widgets.is_empty()
            && self.footer_widgets.is_empty()
            && self.menus.is_empty()
            && self.context_actions.is_empty()
    }

    pub async fn load(fs: Arc<dyn Fs>, extension_dir: &Path) -> Result<Self> {
        let extension_manifest_path = extension_dir.join("extension.toml");
        if !fs.is_file(&extension_manifest_path).await {
            return Ok(Self::default());
        }

        let manifest_content = fs.load(&extension_manifest_path).await?;
        parse_remote_ui_manifest(&manifest_content)
    }

    pub fn validate(&self, schema_version: SchemaVersion) -> Result<()> {
        if !self.is_empty() && schema_version.0 < 2 {
            bail!(
                "remote UI manifest entries require schema_version >= 2, found {}",
                schema_version.0
            );
        }

        for (command_id, command) in &self.commands {
            validate_identifier(command_id, "commands.<id>")?;
            validate_non_empty(&command.title, &format!("commands.{command_id}.title"))?;
            validate_non_empty(
                &command.description,
                &format!("commands.{command_id}.description"),
            )?;
            validate_optional_non_empty(
                command.when.as_deref(),
                &format!("commands.{command_id}.when"),
            )?;
            if let Some(input_schema) = &command.input_schema {
                validate_relative_path(
                    input_schema,
                    &format!("commands.{command_id}.input_schema"),
                )?;
            }
        }

        for (panel_id, panel) in &self.panels {
            validate_identifier(panel_id, "panels.<id>")?;
            validate_non_empty(&panel.title, &format!("panels.{panel_id}.title"))?;
            validate_non_empty(&panel.root_view, &format!("panels.{panel_id}.root_view"))?;
            validate_optional_non_empty(panel.icon.as_deref(), &format!("panels.{panel_id}.icon"))?;
            validate_optional_non_empty(
                panel.toggle_command.as_deref(),
                &format!("panels.{panel_id}.toggle_command"),
            )?;
            if let Some(default_size) = panel.default_size {
                if default_size == 0 {
                    bail!("panels.{panel_id}.default_size must be greater than 0");
                }
            }

            if let Some(toggle_command) = &panel.toggle_command
                && !self.commands.contains_key(toggle_command.as_str())
            {
                bail!(
                    "panels.{panel_id}.toggle_command references unknown command `{toggle_command}`"
                );
            }
        }

        validate_widget_entries(
            &self.titlebar_widgets,
            "titlebar_widgets",
            &self.panels,
            &self.commands,
        )?;
        validate_widget_entries(
            &self.footer_widgets,
            "footer_widgets",
            &self.panels,
            &self.commands,
        )?;
        validate_footer_widget_entries(&self.footer_widgets)?;

        let mut menu_ids = BTreeSet::new();
        for menu in &self.menus {
            validate_identifier(&menu.id, "menus[].id")?;
            if !menu_ids.insert(menu.id.clone()) {
                bail!("duplicate menus[].id `{}`", menu.id);
            }

            validate_non_empty(&menu.title, &format!("menus[{}].title", menu.id))?;
            validate_non_empty(&menu.command, &format!("menus[{}].command", menu.id))?;
            validate_optional_non_empty(
                menu.group.as_deref(),
                &format!("menus[{}].group", menu.id),
            )?;
            validate_optional_non_empty(
                menu.panel.as_deref(),
                &format!("menus[{}].panel", menu.id),
            )?;
            validate_optional_non_empty(menu.when.as_deref(), &format!("menus[{}].when", menu.id))?;

            if menu.priority == 0 {
                bail!("menus[{}].priority must be greater than 0", menu.id);
            }

            if !self.commands.contains_key(menu.command.as_str()) {
                bail!(
                    "menus[{}].command references unknown command `{}`",
                    menu.id,
                    menu.command
                );
            }

            if let Some(panel_id) = &menu.panel
                && !self.panels.contains_key(panel_id.as_str())
            {
                bail!(
                    "menus[{}].panel references unknown panel `{}`",
                    menu.id,
                    panel_id
                );
            }
        }

        let mut context_action_ids = BTreeSet::new();
        for action in &self.context_actions {
            validate_identifier(&action.id, "context_actions[].id")?;
            if !context_action_ids.insert(action.id.clone()) {
                bail!("duplicate context_actions[].id `{}`", action.id);
            }

            validate_non_empty(
                &action.title,
                &format!("context_actions[{}].title", action.id),
            )?;
            validate_non_empty(
                &action.command,
                &format!("context_actions[{}].command", action.id),
            )?;
            validate_optional_non_empty(
                action.group.as_deref(),
                &format!("context_actions[{}].group", action.id),
            )?;
            validate_optional_non_empty(
                action.when.as_deref(),
                &format!("context_actions[{}].when", action.id),
            )?;

            if action.priority == 0 {
                bail!(
                    "context_actions[{}].priority must be greater than 0",
                    action.id
                );
            }

            if !self.commands.contains_key(action.command.as_str()) {
                bail!(
                    "context_actions[{}].command references unknown command `{}`",
                    action.id,
                    action.command
                );
            }
        }

        Ok(())
    }

    pub fn input_schema_paths(&self) -> impl Iterator<Item = &PathBuf> {
        self.commands
            .values()
            .filter_map(|command| command.input_schema.as_ref())
    }
}

impl ExtensionManifest {
    /// Returns the set of features provided by the extension.
    pub fn provides(&self) -> BTreeSet<ExtensionProvides> {
        let mut provides = BTreeSet::default();
        if !self.themes.is_empty() {
            provides.insert(ExtensionProvides::Themes);
        }

        if !self.icon_themes.is_empty() {
            provides.insert(ExtensionProvides::IconThemes);
        }

        if !self.languages.is_empty() {
            provides.insert(ExtensionProvides::Languages);
        }

        if !self.grammars.is_empty() {
            provides.insert(ExtensionProvides::Grammars);
        }

        if !self.language_servers.is_empty() {
            provides.insert(ExtensionProvides::LanguageServers);
        }

        if !self.context_servers.is_empty() {
            provides.insert(ExtensionProvides::ContextServers);
        }

        if !self.agent_servers.is_empty() {
            provides.insert(ExtensionProvides::AgentServers);
        }

        if self.snippets.is_some() {
            provides.insert(ExtensionProvides::Snippets);
        }

        if !self.debug_adapters.is_empty() {
            provides.insert(ExtensionProvides::DebugAdapters);
        }

        provides
    }

    pub fn allow_exec(
        &self,
        desired_command: &str,
        desired_args: &[impl AsRef<str> + std::fmt::Debug],
    ) -> Result<()> {
        let is_allowed = self.capabilities.iter().any(|capability| match capability {
            ExtensionCapability::ProcessExec(capability) => {
                capability.allows(desired_command, desired_args)
            }
            _ => false,
        });

        if !is_allowed {
            bail!(
                "capability for process:exec {desired_command} {desired_args:?} was not listed in the extension manifest",
            );
        }

        Ok(())
    }

    pub fn allow_remote_load(&self) -> bool {
        !self.language_servers.is_empty()
            || !self.debug_adapters.is_empty()
            || !self.debug_locators.is_empty()
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct ExtensionCommandManifestEntry {
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub palette: bool,
    #[serde(default)]
    pub when: Option<String>,
    #[serde(default)]
    pub input_schema: Option<PathBuf>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DockSide {
    Left,
    Bottom,
    Right,
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct ExtensionPanelManifestEntry {
    pub title: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub default_dock: Option<DockSide>,
    #[serde(default)]
    pub default_size: Option<u32>,
    pub root_view: String,
    #[serde(default)]
    pub toggle_command: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WidgetSide {
    Left,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, Serialize, Default)]
pub enum WidgetSize {
    #[serde(rename = "s")]
    Small,
    #[default]
    #[serde(rename = "m")]
    Medium,
    #[serde(rename = "l")]
    Large,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, Serialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum FooterWidgetZone {
    Left,
    #[default]
    Center,
    Right,
}

pub trait WidgetManifestEntry {
    fn id(&self) -> &str;
    fn root_view(&self) -> &str;
    fn priority(&self) -> u32;
    fn size(&self) -> WidgetSize;
    fn min_width(&self) -> Option<u32>;
    fn max_width(&self) -> Option<u32>;
    fn refresh_interval_seconds(&self) -> Option<u32>;
    fn when(&self) -> Option<&str>;
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct TitlebarWidgetManifestEntry {
    pub id: String,
    pub root_view: String,
    pub side: WidgetSide,
    #[serde(default)]
    pub size: WidgetSize,
    pub priority: u32,
    #[serde(default)]
    pub min_width: Option<u32>,
    #[serde(default)]
    pub max_width: Option<u32>,
    #[serde(default)]
    pub refresh_interval_seconds: Option<u32>,
    #[serde(default)]
    pub when: Option<String>,
}

impl WidgetManifestEntry for TitlebarWidgetManifestEntry {
    fn id(&self) -> &str {
        &self.id
    }

    fn root_view(&self) -> &str {
        &self.root_view
    }

    fn priority(&self) -> u32 {
        self.priority
    }

    fn size(&self) -> WidgetSize {
        self.size
    }

    fn min_width(&self) -> Option<u32> {
        self.min_width
    }

    fn max_width(&self) -> Option<u32> {
        self.max_width
    }

    fn refresh_interval_seconds(&self) -> Option<u32> {
        self.refresh_interval_seconds
    }

    fn when(&self) -> Option<&str> {
        self.when.as_deref()
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct FooterWidgetManifestEntry {
    pub id: String,
    pub root_view: String,
    #[serde(alias = "side", default)]
    pub zone: FooterWidgetZone,
    #[serde(default)]
    pub size: WidgetSize,
    pub priority: u32,
    #[serde(default)]
    pub min_width: Option<u32>,
    #[serde(default)]
    pub max_width: Option<u32>,
    #[serde(default)]
    pub refresh_interval_seconds: Option<u32>,
    #[serde(default)]
    pub when: Option<String>,
}

impl WidgetManifestEntry for FooterWidgetManifestEntry {
    fn id(&self) -> &str {
        &self.id
    }

    fn root_view(&self) -> &str {
        &self.root_view
    }

    fn priority(&self) -> u32 {
        self.priority
    }

    fn size(&self) -> WidgetSize {
        self.size
    }

    fn min_width(&self) -> Option<u32> {
        self.min_width
    }

    fn max_width(&self) -> Option<u32> {
        self.max_width
    }

    fn refresh_interval_seconds(&self) -> Option<u32> {
        self.refresh_interval_seconds
    }

    fn when(&self) -> Option<&str> {
        self.when.as_deref()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MenuLocation {
    CommandPalette,
    EditorContext,
    ProjectPanelContext,
    PanelOverflow,
    ItemTabContext,
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct ExtensionMenuManifestEntry {
    pub id: String,
    pub location: MenuLocation,
    pub title: String,
    pub command: String,
    #[serde(default)]
    pub panel: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
    pub priority: u32,
    #[serde(default)]
    pub when: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContextActionTarget {
    Editor,
    ProjectPanel,
    Panel,
    ItemTab,
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct ExtensionContextActionManifestEntry {
    pub id: String,
    pub target: ContextActionTarget,
    pub title: String,
    pub command: String,
    #[serde(default)]
    pub group: Option<String>,
    pub priority: u32,
    #[serde(default)]
    pub when: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct ExtensionSidecarManifestEntry {
    /// Command to launch for the extension sidecar's stdio JSON-RPC transport.
    pub command: String,
    /// Arguments passed to the sidecar command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables added when launching the sidecar.
    #[serde(default)]
    pub env: HashMap<String, String>,
}

impl ExtensionSidecarManifestEntry {
    pub fn bundle_paths(&self) -> impl Iterator<Item = PathBuf> + '_ {
        std::iter::once(&self.command)
            .chain(self.args.iter())
            .filter_map(|value| {
                let path = PathBuf::from(value);
                if path.as_os_str().is_empty() || path.is_absolute() {
                    None
                } else {
                    Some(path)
                }
            })
    }
}

pub fn build_debug_adapter_schema_path(
    adapter_name: &Arc<str>,
    meta: &DebugAdapterManifestEntry,
) -> PathBuf {
    meta.schema_path.clone().unwrap_or_else(|| {
        Path::new("debug_adapter_schemas")
            .join(Path::new(adapter_name.as_ref()).with_extension("json"))
    })
}

#[derive(Clone, Default, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct LibManifestEntry {
    pub kind: Option<ExtensionLibraryKind>,
    pub version: Option<Version>,
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct AgentServerManifestEntry {
    /// Display name for the agent (shown in menus).
    pub name: String,
    /// Environment variables to set when launching the agent server.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Optional icon path (relative to extension root, e.g., "ai.svg").
    /// Should be a small SVG icon for display in menus.
    #[serde(default)]
    pub icon: Option<String>,
    /// Per-target configuration for archive-based installation.
    /// The key format is "{os}-{arch}" where:
    /// - os: "darwin" (macOS), "linux", "windows"
    /// - arch: "aarch64" (arm64), "x86_64"
    ///
    /// Example:
    /// ```toml
    /// [agent_servers.myagent.targets.darwin-aarch64]
    /// archive = "https://example.com/myagent-darwin-arm64.zip"
    /// cmd = "./myagent"
    /// args = ["--serve"]
    /// sha256 = "abc123..."  # optional
    /// ```
    ///
    /// For Node.js-based agents, you can use "node" as the cmd to automatically
    /// use Zed's managed Node.js runtime instead of relying on the user's PATH:
    /// ```toml
    /// [agent_servers.nodeagent.targets.darwin-aarch64]
    /// archive = "https://example.com/nodeagent.zip"
    /// cmd = "node"
    /// args = ["index.js", "--port", "3000"]
    /// ```
    ///
    /// Note: All commands are executed with the archive extraction directory as the
    /// working directory, so relative paths in args (like "index.js") will resolve
    /// relative to the extracted archive contents.
    pub targets: HashMap<String, TargetConfig>,
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct TargetConfig {
    /// URL to download the archive from (e.g., "https://github.com/owner/repo/releases/download/v1.0.0/myagent-darwin-arm64.zip")
    pub archive: String,
    /// Command to run (e.g., "./myagent" or "./myagent.exe")
    pub cmd: String,
    /// Command-line arguments to pass to the agent server.
    #[serde(default)]
    pub args: Vec<String>,
    /// Optional SHA-256 hash of the archive for verification.
    /// If not provided and the URL is a GitHub release, we'll attempt to fetch it from GitHub.
    #[serde(default)]
    pub sha256: Option<String>,
    /// Environment variables to set when launching the agent server.
    /// These target-specific env vars will override any env vars set at the agent level.
    #[serde(default)]
    pub env: HashMap<String, String>,
}

impl TargetConfig {
    pub fn from_proto(proto: proto::ExternalExtensionAgentTarget) -> Self {
        Self {
            archive: proto.archive,
            cmd: proto.cmd,
            args: proto.args,
            sha256: proto.sha256,
            env: proto.env.into_iter().collect(),
        }
    }

    pub fn to_proto(&self) -> proto::ExternalExtensionAgentTarget {
        proto::ExternalExtensionAgentTarget {
            archive: self.archive.clone(),
            cmd: self.cmd.clone(),
            args: self.args.clone(),
            sha256: self.sha256.clone(),
            env: self
                .env
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub enum ExtensionLibraryKind {
    Rust,
}

#[derive(Clone, Default, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct GrammarManifestEntry {
    pub repository: String,
    #[serde(alias = "commit")]
    pub rev: String,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Clone, Default, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct LanguageServerManifestEntry {
    /// Deprecated in favor of `languages`.
    #[serde(default)]
    language: Option<LanguageName>,
    /// The list of languages this language server should work with.
    #[serde(default)]
    languages: Vec<LanguageName>,
    #[serde(default)]
    pub language_ids: HashMap<LanguageName, String>,
    #[serde(default)]
    pub code_action_kinds: Option<Vec<lsp::CodeActionKind>>,
}

impl LanguageServerManifestEntry {
    /// Returns the list of languages for the language server.
    ///
    /// Prefer this over accessing the `language` or `languages` fields directly,
    /// as we currently support both.
    ///
    /// We can replace this with just field access for the `languages` field once
    /// we have removed `language`.
    pub fn languages(&self) -> impl IntoIterator<Item = LanguageName> + '_ {
        let language = if self.languages.is_empty() {
            self.language.clone()
        } else {
            None
        };
        self.languages.iter().cloned().chain(language)
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct ContextServerManifestEntry {}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct SlashCommandManifestEntry {
    pub description: String,
    pub requires_argument: bool,
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct DebugAdapterManifestEntry {
    pub schema_path: Option<PathBuf>,
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct DebugLocatorManifestEntry {}

/// Manifest entry for a language model provider.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct LanguageModelProviderManifestEntry {
    /// Display name for the provider.
    pub name: String,
    /// Path to an SVG icon file relative to the extension root (e.g., "icons/provider.svg").
    #[serde(default)]
    pub icon: Option<String>,
}

impl ExtensionManifest {
    pub async fn load(fs: Arc<dyn Fs>, extension_dir: &Path) -> Result<Self> {
        let extension_name = extension_dir
            .file_name()
            .and_then(OsStr::to_str)
            .context("invalid extension name")?;

        let extension_manifest_path = extension_dir.join("extension.toml");
        if fs.is_file(&extension_manifest_path).await {
            let manifest_content = fs.load(&extension_manifest_path).await.with_context(|| {
                format!("loading {extension_name} extension.toml, {extension_manifest_path:?}")
            })?;
            parse_extension_toml(&manifest_content, extension_name)
        } else if let extension_manifest_path = extension_manifest_path.with_extension("json")
            && fs.is_file(&extension_manifest_path).await
        {
            let manifest_content = fs.load(&extension_manifest_path).await.with_context(|| {
                format!("loading {extension_name} extension.json, {extension_manifest_path:?}")
            })?;

            serde_json::from_str::<OldExtensionManifest>(&manifest_content)
                .with_context(|| format!("invalid extension.json for extension {extension_name}"))
                .map(|manifest_json| manifest_from_old_manifest(manifest_json, extension_name))
        } else {
            anyhow::bail!("No extension manifest found for extension {extension_name}")
        }
    }
}

pub fn serialize_extension_manifest_with_remote_ui(
    manifest: &ExtensionManifest,
    remote_ui: &RemoteUiManifest,
) -> Result<String> {
    let mut manifest_value =
        toml::Value::try_from(manifest).context("failed to serialize extension manifest")?;
    let remote_ui_value =
        toml::Value::try_from(remote_ui).context("failed to serialize remote UI manifest")?;
    merge_toml_tables(&mut manifest_value, remote_ui_value)?;
    toml::to_string(&manifest_value).context("failed to render extension.toml")
}

#[derive(Deserialize)]
struct ParsedExtensionToml {
    #[serde(flatten)]
    manifest: ExtensionManifest,
    #[serde(flatten)]
    remote_ui: RemoteUiManifest,
}

fn parse_extension_toml(manifest_content: &str, extension_name: &str) -> Result<ExtensionManifest> {
    let parsed = toml::from_str::<ParsedExtensionToml>(manifest_content)
        .map_err(|err| anyhow!("Invalid extension.toml for extension {extension_name}:\n{err}"))?;
    parsed.remote_ui.validate(parsed.manifest.schema_version)?;
    if let Some(sidecar) = &parsed.manifest.sidecar {
        validate_sidecar_manifest(sidecar)?;
    }

    Ok(parsed.manifest)
}

fn parse_remote_ui_manifest(manifest_content: &str) -> Result<RemoteUiManifest> {
    Ok(toml::from_str::<ParsedExtensionToml>(manifest_content)?.remote_ui)
}

fn merge_toml_tables(base: &mut toml::Value, extra: toml::Value) -> Result<()> {
    let base_table = base
        .as_table_mut()
        .context("serialized extension manifest must be a table")?;
    let extra_table = extra
        .as_table()
        .context("serialized remote UI manifest must be a table")?;

    for (key, value) in extra_table {
        if base_table.insert(key.clone(), value.clone()).is_some() {
            bail!("duplicate key `{key}` while merging remote UI manifest");
        }
    }

    Ok(())
}

fn validate_widget_entries<T: WidgetManifestEntry>(
    entries: &[T],
    field_name: &str,
    _panels: &BTreeMap<Arc<str>, ExtensionPanelManifestEntry>,
    _commands: &BTreeMap<Arc<str>, ExtensionCommandManifestEntry>,
) -> Result<()> {
    let mut ids = BTreeSet::new();
    for entry in entries {
        validate_identifier(entry.id(), &format!("{field_name}[].id"))?;
        if !ids.insert(entry.id().to_string()) {
            bail!("duplicate {field_name}[].id `{}`", entry.id());
        }

        validate_non_empty(
            entry.root_view(),
            &format!("{field_name}[{}].root_view", entry.id()),
        )?;
        validate_optional_non_empty(entry.when(), &format!("{field_name}[{}].when", entry.id()))?;

        if entry.priority() == 0 {
            bail!(
                "{field_name}[{}].priority must be greater than 0",
                entry.id()
            );
        }

        if field_name == "titlebar_widgets"
            && entry.size() == WidgetSize::Small
            && entry.max_width().is_some_and(|max_width| max_width > 32)
        {
            bail!(
                "{field_name}[{}].size `s` cannot exceed 32px max_width",
                entry.id()
            );
        }

        if let Some(min_width) = entry.min_width()
            && min_width == 0
        {
            bail!(
                "{field_name}[{}].min_width must be greater than 0",
                entry.id()
            );
        }
        if let Some(max_width) = entry.max_width()
            && max_width == 0
        {
            bail!(
                "{field_name}[{}].max_width must be greater than 0",
                entry.id()
            );
        }
        if let (Some(min_width), Some(max_width)) = (entry.min_width(), entry.max_width())
            && min_width > max_width
        {
            bail!(
                "{field_name}[{}].min_width must be less than or equal to max_width",
                entry.id()
            );
        }
        if let Some(refresh_interval_seconds) = entry.refresh_interval_seconds()
            && refresh_interval_seconds == 0
        {
            bail!(
                "{field_name}[{}].refresh_interval_seconds must be greater than 0",
                entry.id()
            );
        }
    }

    Ok(())
}

fn validate_footer_widget_entries(entries: &[FooterWidgetManifestEntry]) -> Result<()> {
    for entry in entries {
        if matches!(entry.zone, FooterWidgetZone::Left | FooterWidgetZone::Right)
            && entry.size != WidgetSize::Small
        {
            bail!(
                "footer_widgets[{}].size must be `s` for left or right zones",
                entry.id
            );
        }
    }

    Ok(())
}

fn validate_sidecar_manifest(sidecar: &ExtensionSidecarManifestEntry) -> Result<()> {
    validate_non_empty(&sidecar.command, "sidecar.command")?;

    for env_key in sidecar.env.keys() {
        validate_non_empty(env_key, "sidecar.env.<key>")?;
    }

    Ok(())
}

fn validate_non_empty(value: &str, field_name: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{field_name} must not be empty");
    }
    Ok(())
}

fn validate_optional_non_empty(value: Option<&str>, field_name: &str) -> Result<()> {
    if let Some(value) = value {
        validate_non_empty(value, field_name)?;
    }
    Ok(())
}

fn validate_identifier(value: &str, field_name: &str) -> Result<()> {
    validate_non_empty(value, field_name)?;
    Ok(())
}

fn validate_relative_path(path: &Path, field_name: &str) -> Result<()> {
    if path.is_absolute() {
        bail!("{field_name} must be a relative path");
    }

    if path
        .components()
        .any(|component| component == Component::ParentDir)
    {
        bail!("{field_name} must not contain `..` path traversal");
    }

    Ok(())
}

fn manifest_from_old_manifest(
    manifest_json: OldExtensionManifest,
    extension_id: &str,
) -> ExtensionManifest {
    ExtensionManifest {
        id: extension_id.into(),
        name: manifest_json.name,
        version: manifest_json.version,
        description: manifest_json.description,
        repository: manifest_json.repository,
        authors: manifest_json.authors,
        schema_version: SchemaVersion::ZERO,
        lib: Default::default(),
        themes: {
            let mut themes = manifest_json.themes.into_values().collect::<Vec<_>>();
            themes.sort();
            themes.dedup();
            themes
        },
        icon_themes: Vec::new(),
        languages: {
            let mut languages = manifest_json.languages.into_values().collect::<Vec<_>>();
            languages.sort();
            languages.dedup();
            languages
        },
        grammars: manifest_json
            .grammars
            .into_keys()
            .map(|grammar_name| (grammar_name, Default::default()))
            .collect(),
        language_servers: Default::default(),
        context_servers: BTreeMap::default(),
        agent_servers: BTreeMap::default(),
        slash_commands: BTreeMap::default(),
        snippets: None,
        sidecar: None,
        capabilities: Vec::new(),
        debug_adapters: Default::default(),
        debug_locators: Default::default(),
        language_model_providers: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use crate::ProcessExecCapability;

    use super::*;

    fn extension_manifest() -> ExtensionManifest {
        ExtensionManifest {
            id: "test".into(),
            name: "Test".to_string(),
            version: "1.0.0".into(),
            schema_version: SchemaVersion::ZERO,
            description: None,
            repository: None,
            authors: vec![],
            lib: Default::default(),
            themes: vec![],
            icon_themes: vec![],
            languages: vec![],
            grammars: BTreeMap::default(),
            language_servers: BTreeMap::default(),
            context_servers: BTreeMap::default(),
            agent_servers: BTreeMap::default(),
            slash_commands: BTreeMap::default(),
            snippets: None,
            sidecar: None,
            capabilities: vec![],
            debug_adapters: Default::default(),
            debug_locators: Default::default(),
            language_model_providers: BTreeMap::default(),
        }
    }

    #[test]
    fn parse_remote_ui_manifest_entries() {
        let toml_src = r#"
id = "remote-ui-test"
name = "Remote UI Test"
version = "1.0.0"
schema_version = 2

[commands.sample-open]
title = "Open Sample"
description = "Open the sample panel"
palette = true
when = "workspace.trusted"
input_schema = "schemas/sample.json"

[panels.sample]
title = "Sample"
icon = "bolt"
default_dock = "right"
root_view = "sample.panel"
toggle_command = "sample-open"

[[titlebar_widgets]]
id = "sample-titlebar"
root_view = "sample.titlebar"
side = "right"
size = "m"
priority = 100
min_width = 96
max_width = 220

[[footer_widgets]]
id = "sample-footer"
root_view = "sample.footer"
zone = "left"
size = "s"
priority = 200

[[menus]]
id = "sample-menu"
location = "panel-overflow"
title = "Refresh"
command = "sample-open"
panel = "sample"
priority = 100

[[context_actions]]
id = "sample-context"
target = "editor"
title = "Explain"
command = "sample-open"
priority = 100
"#;

        let remote_ui = parse_remote_ui_manifest(toml_src).expect("manifest should parse");
        assert_eq!(remote_ui.commands.len(), 1);
        assert_eq!(remote_ui.panels.len(), 1);
        assert_eq!(remote_ui.titlebar_widgets.len(), 1);
        assert_eq!(remote_ui.footer_widgets.len(), 1);
        assert_eq!(remote_ui.menus.len(), 1);
        assert_eq!(remote_ui.context_actions.len(), 1);
        assert_eq!(
            remote_ui
                .commands
                .get("sample-open")
                .and_then(|command| command.input_schema.as_ref()),
            Some(&PathBuf::from("schemas/sample.json"))
        );
    }

    #[test]
    fn remote_ui_manifest_requires_schema_version_two() {
        let remote_ui = RemoteUiManifest {
            commands: [(
                Arc::<str>::from("sample-open"),
                ExtensionCommandManifestEntry {
                    title: "Open Sample".to_string(),
                    description: "Open the sample panel".to_string(),
                    palette: true,
                    when: None,
                    input_schema: None,
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };

        assert!(remote_ui.validate(SchemaVersion(1)).is_err());
        assert!(remote_ui.validate(SchemaVersion(2)).is_ok());
    }

    #[test]
    fn remote_ui_manifest_rejects_invalid_references() {
        let remote_ui = RemoteUiManifest {
            panels: [(
                Arc::<str>::from("sample"),
                ExtensionPanelManifestEntry {
                    title: "Sample".to_string(),
                    icon: None,
                    default_dock: None,
                    default_size: None,
                    root_view: "sample.panel".to_string(),
                    toggle_command: Some("missing-command".to_string()),
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };

        assert!(remote_ui.validate(SchemaVersion(2)).is_err());
    }

    #[test]
    fn remote_ui_manifest_rejects_invalid_input_schema_path() {
        let remote_ui = RemoteUiManifest {
            commands: [(
                Arc::<str>::from("sample-open"),
                ExtensionCommandManifestEntry {
                    title: "Open Sample".to_string(),
                    description: "Open the sample panel".to_string(),
                    palette: true,
                    when: None,
                    input_schema: Some(PathBuf::from("../schemas/sample.json")),
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };

        assert!(remote_ui.validate(SchemaVersion(2)).is_err());
    }

    #[test]
    fn serialize_manifest_with_remote_ui_entries() {
        let manifest = extension_manifest();
        let remote_ui = RemoteUiManifest {
            commands: [(
                Arc::<str>::from("sample-open"),
                ExtensionCommandManifestEntry {
                    title: "Open Sample".to_string(),
                    description: "Open the sample panel".to_string(),
                    palette: true,
                    when: None,
                    input_schema: Some(PathBuf::from("schemas/sample.json")),
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };

        let serialized =
            serialize_extension_manifest_with_remote_ui(&manifest, &remote_ui).unwrap();

        assert!(serialized.contains("[commands.sample-open]"));
        assert!(serialized.contains("input_schema = \"schemas/sample.json\""));
    }

    #[test]
    fn remote_ui_manifest_does_not_change_provides_yet() {
        let toml_src = r#"
id = "remote-ui-test"
name = "Remote UI Test"
version = "1.0.0"
schema_version = 2

[commands.sample-open]
title = "Open Sample"
description = "Open the sample panel"
palette = true

[panels.sample]
title = "Sample"
root_view = "sample.panel"
toggle_command = "sample-open"
"#;

        let manifest =
            parse_extension_toml(toml_src, "remote-ui-test").expect("manifest should parse");

        assert!(
            manifest.provides().is_empty(),
            "remote UI contributions are tracked separately from manifest-local `provides`"
        );
    }

    #[test]
    fn parse_manifest_with_stdio_json_rpc_sidecar() {
        let toml_src = r#"
id = "sidecar-test"
name = "Sidecar Test"
version = "1.0.0"
schema_version = 0

[sidecar]
command = "node"
args = ["./sidecar.js", "--stdio"]

[sidecar.env]
RUST_LOG = "debug"
"#;

        let manifest =
            parse_extension_toml(toml_src, "sidecar-test").expect("manifest should parse");

        let sidecar = manifest.sidecar.expect("sidecar should parse");
        assert_eq!(sidecar.command, "node");
        assert_eq!(sidecar.args, vec!["./sidecar.js", "--stdio"]);
        assert_eq!(
            sidecar.env.get("RUST_LOG").map(String::as_str),
            Some("debug")
        );
    }

    #[test]
    fn sidecar_manifest_requires_a_command() {
        let toml_src = r#"
id = "sidecar-test"
name = "Sidecar Test"
version = "1.0.0"
schema_version = 0

[sidecar]
command = "   "
"#;

        assert!(parse_extension_toml(toml_src, "sidecar-test").is_err());
    }

    #[test]
    fn serialize_manifest_with_sidecar() {
        let manifest = ExtensionManifest {
            sidecar: Some(ExtensionSidecarManifestEntry {
                command: "node".to_string(),
                args: vec!["./sidecar.js".to_string(), "--stdio".to_string()],
                env: [("RUST_LOG".to_string(), "debug".to_string())]
                    .into_iter()
                    .collect(),
            }),
            ..extension_manifest()
        };

        let serialized = toml::to_string(&manifest).expect("manifest should serialize");

        assert!(serialized.contains("[sidecar]"));
        assert!(serialized.contains("command = \"node\""));
        assert!(serialized.contains("args = [\"./sidecar.js\", \"--stdio\"]"));
        assert!(serialized.contains("[sidecar.env]"));
    }

    #[test]
    fn test_build_adapter_schema_path_with_schema_path() {
        let adapter_name = Arc::from("my_adapter");
        let entry = DebugAdapterManifestEntry {
            schema_path: Some(PathBuf::from("foo/bar")),
        };

        let path = build_debug_adapter_schema_path(&adapter_name, &entry);
        assert_eq!(path, PathBuf::from("foo/bar"));
    }

    #[test]
    fn test_build_adapter_schema_path_without_schema_path() {
        let adapter_name = Arc::from("my_adapter");
        let entry = DebugAdapterManifestEntry { schema_path: None };

        let path = build_debug_adapter_schema_path(&adapter_name, &entry);
        assert_eq!(
            path,
            PathBuf::from("debug_adapter_schemas").join("my_adapter.json")
        );
    }

    #[test]
    fn test_allow_exec_exact_match() {
        let manifest = ExtensionManifest {
            capabilities: vec![ExtensionCapability::ProcessExec(ProcessExecCapability {
                command: "ls".to_string(),
                args: vec!["-la".to_string()],
            })],
            ..extension_manifest()
        };

        assert!(manifest.allow_exec("ls", &["-la"]).is_ok());
        assert!(manifest.allow_exec("ls", &["-l"]).is_err());
        assert!(manifest.allow_exec("pwd", &[] as &[&str]).is_err());
    }

    #[test]
    fn test_allow_exec_wildcard_arg() {
        let manifest = ExtensionManifest {
            capabilities: vec![ExtensionCapability::ProcessExec(ProcessExecCapability {
                command: "git".to_string(),
                args: vec!["*".to_string()],
            })],
            ..extension_manifest()
        };

        assert!(manifest.allow_exec("git", &["status"]).is_ok());
        assert!(manifest.allow_exec("git", &["commit"]).is_ok());
        assert!(manifest.allow_exec("git", &["status", "-s"]).is_err()); // too many args
        assert!(manifest.allow_exec("npm", &["install"]).is_err()); // wrong command
    }

    #[test]
    fn test_allow_exec_double_wildcard() {
        let manifest = ExtensionManifest {
            capabilities: vec![ExtensionCapability::ProcessExec(ProcessExecCapability {
                command: "cargo".to_string(),
                args: vec!["test".to_string(), "**".to_string()],
            })],
            ..extension_manifest()
        };

        assert!(manifest.allow_exec("cargo", &["test"]).is_ok());
        assert!(manifest.allow_exec("cargo", &["test", "--all"]).is_ok());
        assert!(
            manifest
                .allow_exec("cargo", &["test", "--all", "--no-fail-fast"])
                .is_ok()
        );
        assert!(manifest.allow_exec("cargo", &["build"]).is_err()); // wrong first arg
    }

    #[test]
    fn test_allow_exec_mixed_wildcards() {
        let manifest = ExtensionManifest {
            capabilities: vec![ExtensionCapability::ProcessExec(ProcessExecCapability {
                command: "docker".to_string(),
                args: vec!["run".to_string(), "*".to_string(), "**".to_string()],
            })],
            ..extension_manifest()
        };

        assert!(manifest.allow_exec("docker", &["run", "nginx"]).is_ok());
        assert!(manifest.allow_exec("docker", &["run"]).is_err());
        assert!(
            manifest
                .allow_exec("docker", &["run", "ubuntu", "bash"])
                .is_ok()
        );
        assert!(
            manifest
                .allow_exec("docker", &["run", "alpine", "sh", "-c", "echo hello"])
                .is_ok()
        );
        assert!(manifest.allow_exec("docker", &["ps"]).is_err()); // wrong first arg
    }
    #[test]
    fn parse_manifest_with_agent_server_archive_launcher() {
        let toml_src = r#"
id = "example.agent-server-ext"
name = "Agent Server Example"
version = "1.0.0"
schema_version = 0

[agent_servers.foo]
name = "Foo Agent"

[agent_servers.foo.targets.linux-x86_64]
archive = "https://example.com/agent-linux-x64.tar.gz"
cmd = "./agent"
args = ["--serve"]
"#;

        let manifest: ExtensionManifest = toml::from_str(toml_src).expect("manifest should parse");
        assert_eq!(manifest.id.as_ref(), "example.agent-server-ext");
        assert!(manifest.agent_servers.contains_key("foo"));
        let entry = manifest.agent_servers.get("foo").unwrap();
        assert!(entry.targets.contains_key("linux-x86_64"));
        let target = entry.targets.get("linux-x86_64").unwrap();
        assert_eq!(target.archive, "https://example.com/agent-linux-x64.tar.gz");
        assert_eq!(target.cmd, "./agent");
        assert_eq!(target.args, vec!["--serve"]);
    }
}
