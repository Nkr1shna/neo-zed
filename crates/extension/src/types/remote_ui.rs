#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandContext {
    pub workspace_id: u64,
    pub trusted: bool,
    pub active_item_kind: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountKind {
    TitlebarWidget,
    FooterWidget,
    Panel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountContext {
    pub workspace_id: u64,
    pub mount_kind: MountKind,
    pub trusted: bool,
    pub active_item_kind: Option<String>,
    pub appearance: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderReason {
    Initial,
    Event,
    HostContextChanged,
    VirtualRangeChanged,
    ExplicitRefresh,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteViewProperty {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualListProps {
    pub item_count: u32,
    pub estimated_row_height: u32,
    pub selection_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressBarProps {
    pub value: u32,
    pub max_value: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteViewNodeKind {
    Row,
    Column,
    Stack,
    Text(String),
    Icon(String),
    Button(String),
    Toggle(bool),
    Checkbox(bool),
    TextInput(String),
    Badge(String),
    ProgressBar(ProgressBarProps),
    Divider,
    Spacer,
    ScrollView,
    VirtualList(VirtualListProps),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteViewNode {
    pub node_id: String,
    pub parent_id: Option<String>,
    pub kind: RemoteViewNodeKind,
    pub properties: Vec<RemoteViewProperty>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteViewTree {
    pub revision: u64,
    pub root_id: String,
    pub nodes: Vec<RemoteViewNode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteViewEventKind {
    Click,
    Change,
    Submit,
    ListItemActivated,
    ListSelectionChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteViewEvent {
    pub node_id: String,
    pub kind: RemoteViewEventKind,
    pub payload_json: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualListRange {
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventOutcome {
    Noop,
    Rerender,
    RerenderVirtualRange(String),
    ShowError(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostMutation {
    ShowToast(String),
    OpenPanel(String),
    ClosePanel(String),
    CopyToClipboard(String),
    OpenExternalUrl(String),
}
