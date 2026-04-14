use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use strum::EnumString;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginDockPosition {
    Left,
    Right,
    Bottom,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginPanelActivation {
    OnDemand,
    OnStartup,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginPanelDescriptor {
    pub id: String,
    pub title: String,
    pub dock: PluginDockPosition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<String>,
    pub activation: PluginPanelActivation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginTitlebarWidgetSide {
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginTitlebarWidgetDescriptor {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<String>,
    #[serde(default = "default_titlebar_widget_side")]
    pub side: PluginTitlebarWidgetSide,
    #[serde(default = "default_titlebar_widget_priority")]
    pub priority: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opens_panel_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginActionDescriptor {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct PluginApiManifest {
    pub name: String,
    pub version: Arc<str>,
    pub description: Option<String>,
    pub authors: Vec<String>,
    pub repository: Option<String>,
    pub homepage: Option<String>,
    pub schema_version: u32,
    #[serde(default)]
    pub panels: Vec<PluginPanelDescriptor>,
    #[serde(default)]
    pub titlebar_widgets: Vec<PluginTitlebarWidgetDescriptor>,
    #[serde(default)]
    pub actions: Vec<PluginActionDescriptor>,
    #[serde(default)]
    pub artifacts: Vec<PluginArtifact>,
}

#[derive(
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    EnumString,
    strum::Display,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum PluginArtifactKind {
    SourceArchive,
    BinaryArchive,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct PluginArtifact {
    pub kind: PluginArtifactKind,
    pub archive_path: Arc<str>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operating_system: Option<Arc<str>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architecture: Option<Arc<str>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable_path: Option<Arc<str>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<Arc<str>>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct PluginMetadata {
    pub id: Arc<str>,
    #[serde(flatten)]
    pub manifest: PluginApiManifest,
    pub published_at: DateTime<Utc>,
    pub download_count: u64,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct GetPluginsResponse {
    pub data: Vec<PluginMetadata>,
}

fn default_titlebar_widget_side() -> PluginTitlebarWidgetSide {
    PluginTitlebarWidgetSide::Right
}

fn default_titlebar_widget_priority() -> u32 {
    100
}
