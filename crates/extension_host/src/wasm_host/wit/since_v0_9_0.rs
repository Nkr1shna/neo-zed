use self::{
    dap::{
        BuildTaskDefinition, BuildTaskDefinitionTemplatePayload, StartDebuggingRequestArguments,
        TcpArguments, TcpArgumentsTemplate,
    },
    lsp::{CompletionKind, CompletionLabelDetails, InsertTextFormat, SymbolKind},
    slash_command::SlashCommandOutputSection,
};
use crate::wasm_host::{WasmState, wit::ToWasmtimeResult};
use ::http_client::{AsyncBody, HttpRequestExt};
use ::settings::{Settings, WorktreeId};
use anyhow::{Context as _, Result, anyhow, bail};
use async_compression::futures::bufread::GzipDecoder;
use async_tar::Archive;
use async_trait::async_trait;
use extension::{
    CommandContext as ExtensionCommandContext, EventOutcome as ExtensionEventOutcome,
    ExtensionLanguageServerProxy, ExtensionRemoteUiProxy, HostMutation as ExtensionHostMutation,
    KeyValueStoreDelegate, MountContext as ExtensionMountContext, MountKind as ExtensionMountKind,
    ProgressBarProps as ExtensionProgressBarProps, ProjectDelegate,
    RemoteViewEvent as ExtensionRemoteViewEvent,
    RemoteViewEventKind as ExtensionRemoteViewEventKind, RemoteViewNode as ExtensionRemoteViewNode,
    RemoteViewNodeKind as ExtensionRemoteViewNodeKind,
    RemoteViewProperty as ExtensionRemoteViewProperty, RemoteViewTree as ExtensionRemoteViewTree,
    RenderReason as ExtensionRenderReason, VirtualListProps as ExtensionVirtualListProps,
    VirtualListRange as ExtensionVirtualListRange, WorktreeDelegate,
};
use futures::{AsyncReadExt, lock::Mutex};
use futures::{FutureExt as _, io::BufReader};
use gpui::{BackgroundExecutor, SharedString};
use language::{BinaryStatus, LanguageName, language_settings::AllLanguageSettings};
use project::project_settings::ProjectSettings;
use semver::Version;
use std::{
    env,
    net::Ipv4Addr,
    path::{Path, PathBuf},
    str::FromStr,
    sync::{Arc, OnceLock},
    time::Duration,
};
use task::{SpawnInTerminal, ZedDebugConfig};
use url::Url;
use util::{
    archive::extract_zip, fs::make_file_executable, maybe, paths::PathStyle, rel_path::RelPath,
};
use wasmtime::component::{Linker, Resource};

pub const MIN_VERSION: Version = Version::new(0, 9, 0);
pub const MAX_VERSION: Version = Version::new(0, 9, 0);

wasmtime::component::bindgen!({
    async: true,
    trappable_imports: true,
    path: "../extension_api/wit/since_v0.9.0",
    with: {
         "worktree": ExtensionWorktree,
         "project": ExtensionProject,
         "key-value-store": ExtensionKeyValueStore,
         "zed:extension/http-client/http-response-stream": ExtensionHttpResponseStream
    },
});

pub use self::zed::extension::*;

mod settings {
    #![allow(dead_code)]
    include!(concat!(env!("OUT_DIR"), "/since_v0.9.0/settings.rs"));
}

pub type ExtensionWorktree = Arc<dyn WorktreeDelegate>;
pub type ExtensionProject = Arc<dyn ProjectDelegate>;
pub type ExtensionKeyValueStore = Arc<dyn KeyValueStoreDelegate>;
pub type ExtensionHttpResponseStream = Arc<Mutex<::http_client::Response<AsyncBody>>>;

pub fn linker(executor: &BackgroundExecutor) -> &'static Linker<WasmState> {
    static LINKER: OnceLock<Linker<WasmState>> = OnceLock::new();
    LINKER.get_or_init(|| super::new_linker(executor, Extension::add_to_linker))
}

impl From<Range> for std::ops::Range<usize> {
    fn from(range: Range) -> Self {
        let start = range.start as usize;
        let end = range.end as usize;
        start..end
    }
}

impl From<Command> for extension::Command {
    fn from(value: Command) -> Self {
        Self {
            command: value.command.into(),
            args: value.args,
            env: value.env,
        }
    }
}

impl From<Command> for super::since_v0_8_0::Command {
    fn from(value: Command) -> Self {
        Self {
            command: value.command,
            args: value.args,
            env: value.env,
        }
    }
}

impl From<StartDebuggingRequestArgumentsRequest>
    for extension::StartDebuggingRequestArgumentsRequest
{
    fn from(value: StartDebuggingRequestArgumentsRequest) -> Self {
        match value {
            StartDebuggingRequestArgumentsRequest::Launch => Self::Launch,
            StartDebuggingRequestArgumentsRequest::Attach => Self::Attach,
        }
    }
}

impl TryFrom<StartDebuggingRequestArguments> for extension::StartDebuggingRequestArguments {
    type Error = anyhow::Error;

    fn try_from(value: StartDebuggingRequestArguments) -> Result<Self, Self::Error> {
        Ok(Self {
            configuration: serde_json::from_str(&value.configuration)?,
            request: value.request.into(),
        })
    }
}

impl From<TcpArguments> for extension::TcpArguments {
    fn from(value: TcpArguments) -> Self {
        Self {
            host: value.host.into(),
            port: value.port,
            timeout: value.timeout,
        }
    }
}

impl From<extension::TcpArgumentsTemplate> for TcpArgumentsTemplate {
    fn from(value: extension::TcpArgumentsTemplate) -> Self {
        Self {
            host: value.host.map(Ipv4Addr::to_bits),
            port: value.port,
            timeout: value.timeout,
        }
    }
}

impl From<TcpArgumentsTemplate> for extension::TcpArgumentsTemplate {
    fn from(value: TcpArgumentsTemplate) -> Self {
        Self {
            host: value.host.map(Ipv4Addr::from_bits),
            port: value.port,
            timeout: value.timeout,
        }
    }
}

impl TryFrom<extension::DebugTaskDefinition> for DebugTaskDefinition {
    type Error = anyhow::Error;

    fn try_from(value: extension::DebugTaskDefinition) -> Result<Self, Self::Error> {
        Ok(Self {
            label: value.label.to_string(),
            adapter: value.adapter.to_string(),
            config: value.config.to_string(),
            tcp_connection: value.tcp_connection.map(Into::into),
        })
    }
}

impl From<task::DebugRequest> for DebugRequest {
    fn from(value: task::DebugRequest) -> Self {
        match value {
            task::DebugRequest::Launch(launch_request) => Self::Launch(launch_request.into()),
            task::DebugRequest::Attach(attach_request) => Self::Attach(attach_request.into()),
        }
    }
}

impl From<DebugRequest> for task::DebugRequest {
    fn from(value: DebugRequest) -> Self {
        match value {
            DebugRequest::Launch(launch_request) => Self::Launch(launch_request.into()),
            DebugRequest::Attach(attach_request) => Self::Attach(attach_request.into()),
        }
    }
}

impl From<task::LaunchRequest> for LaunchRequest {
    fn from(value: task::LaunchRequest) -> Self {
        Self {
            program: value.program,
            cwd: value.cwd.map(|p| p.to_string_lossy().into_owned()),
            args: value.args,
            envs: value.env.into_iter().collect(),
        }
    }
}

impl From<task::AttachRequest> for AttachRequest {
    fn from(value: task::AttachRequest) -> Self {
        Self {
            process_id: value.process_id,
        }
    }
}

impl From<LaunchRequest> for task::LaunchRequest {
    fn from(value: LaunchRequest) -> Self {
        Self {
            program: value.program,
            cwd: value.cwd.map(|p| p.into()),
            args: value.args,
            env: value.envs.into_iter().collect(),
        }
    }
}
impl From<AttachRequest> for task::AttachRequest {
    fn from(value: AttachRequest) -> Self {
        Self {
            process_id: value.process_id,
        }
    }
}

impl From<ZedDebugConfig> for DebugConfig {
    fn from(value: ZedDebugConfig) -> Self {
        Self {
            label: value.label.into(),
            adapter: value.adapter.into(),
            request: value.request.into(),
            stop_on_entry: value.stop_on_entry,
        }
    }
}
impl TryFrom<DebugAdapterBinary> for extension::DebugAdapterBinary {
    type Error = anyhow::Error;
    fn try_from(value: DebugAdapterBinary) -> Result<Self, Self::Error> {
        Ok(Self {
            command: value.command,
            arguments: value.arguments,
            envs: value.envs.into_iter().collect(),
            cwd: value.cwd.map(|s| s.into()),
            connection: value.connection.map(Into::into),
            request_args: value.request_args.try_into()?,
        })
    }
}

impl From<BuildTaskDefinition> for extension::BuildTaskDefinition {
    fn from(value: BuildTaskDefinition) -> Self {
        match value {
            BuildTaskDefinition::ByName(name) => Self::ByName(name.into()),
            BuildTaskDefinition::Template(build_task_template) => Self::Template {
                task_template: build_task_template.template.into(),
                locator_name: build_task_template.locator_name.map(SharedString::from),
            },
        }
    }
}

impl From<extension::BuildTaskDefinition> for BuildTaskDefinition {
    fn from(value: extension::BuildTaskDefinition) -> Self {
        match value {
            extension::BuildTaskDefinition::ByName(name) => Self::ByName(name.into()),
            extension::BuildTaskDefinition::Template {
                task_template,
                locator_name,
            } => Self::Template(BuildTaskDefinitionTemplatePayload {
                template: task_template.into(),
                locator_name: locator_name.map(String::from),
            }),
        }
    }
}

impl From<BuildTaskTemplate> for extension::BuildTaskTemplate {
    fn from(value: BuildTaskTemplate) -> Self {
        Self {
            label: value.label,
            command: value.command,
            args: value.args,
            env: value.env.into_iter().collect(),
            cwd: value.cwd,
            ..Default::default()
        }
    }
}
impl From<extension::BuildTaskTemplate> for BuildTaskTemplate {
    fn from(value: extension::BuildTaskTemplate) -> Self {
        Self {
            label: value.label,
            command: value.command,
            args: value.args,
            env: value.env.into_iter().collect(),
            cwd: value.cwd,
        }
    }
}

impl TryFrom<DebugScenario> for extension::DebugScenario {
    type Error = anyhow::Error;

    fn try_from(value: DebugScenario) -> std::result::Result<Self, Self::Error> {
        Ok(Self {
            adapter: value.adapter.into(),
            label: value.label.into(),
            build: value.build.map(Into::into),
            config: serde_json::Value::from_str(&value.config)?,
            tcp_connection: value.tcp_connection.map(Into::into),
        })
    }
}

impl From<extension::DebugScenario> for DebugScenario {
    fn from(value: extension::DebugScenario) -> Self {
        Self {
            adapter: value.adapter.into(),
            label: value.label.into(),
            build: value.build.map(Into::into),
            config: value.config.to_string(),
            tcp_connection: value.tcp_connection.map(Into::into),
        }
    }
}

impl TryFrom<SpawnInTerminal> for ResolvedTask {
    type Error = anyhow::Error;

    fn try_from(value: SpawnInTerminal) -> Result<Self, Self::Error> {
        Ok(Self {
            label: value.label,
            command: value.command.context("missing command")?,
            args: value.args,
            env: value.env.into_iter().collect(),
            cwd: value.cwd.map(|s| {
                let s = s.to_string_lossy();
                if cfg!(target_os = "windows") {
                    s.replace('\\', "/")
                } else {
                    s.into_owned()
                }
            }),
        })
    }
}

impl From<CodeLabel> for extension::CodeLabel {
    fn from(value: CodeLabel) -> Self {
        Self {
            code: value.code,
            spans: value.spans.into_iter().map(Into::into).collect(),
            filter_range: value.filter_range.into(),
        }
    }
}

impl From<CodeLabel> for super::since_v0_8_0::CodeLabel {
    fn from(value: CodeLabel) -> Self {
        Self {
            code: value.code,
            spans: value.spans.into_iter().map(Into::into).collect(),
            filter_range: super::since_v0_8_0::Range {
                start: value.filter_range.start,
                end: value.filter_range.end,
            },
        }
    }
}

impl From<CodeLabelSpan> for extension::CodeLabelSpan {
    fn from(value: CodeLabelSpan) -> Self {
        match value {
            CodeLabelSpan::CodeRange(range) => Self::CodeRange(range.into()),
            CodeLabelSpan::Literal(literal) => Self::Literal(literal.into()),
        }
    }
}

impl From<CodeLabelSpan> for super::since_v0_8_0::CodeLabelSpan {
    fn from(value: CodeLabelSpan) -> Self {
        match value {
            CodeLabelSpan::CodeRange(range) => Self::CodeRange(super::since_v0_8_0::Range {
                start: range.start,
                end: range.end,
            }),
            CodeLabelSpan::Literal(literal) => Self::Literal(literal.into()),
        }
    }
}

impl From<CodeLabelSpanLiteral> for extension::CodeLabelSpanLiteral {
    fn from(value: CodeLabelSpanLiteral) -> Self {
        Self {
            text: value.text,
            highlight_name: value.highlight_name,
        }
    }
}

impl From<CodeLabelSpanLiteral> for super::since_v0_8_0::CodeLabelSpanLiteral {
    fn from(value: CodeLabelSpanLiteral) -> Self {
        Self {
            text: value.text,
            highlight_name: value.highlight_name,
        }
    }
}

impl From<extension::Completion> for Completion {
    fn from(value: extension::Completion) -> Self {
        Self {
            label: value.label,
            label_details: value.label_details.map(Into::into),
            detail: value.detail,
            kind: value.kind.map(Into::into),
            insert_text_format: value.insert_text_format.map(Into::into),
        }
    }
}

impl From<super::since_v0_8_0::Completion> for Completion {
    fn from(value: super::since_v0_8_0::Completion) -> Self {
        Self {
            label: value.label,
            label_details: value.label_details.map(Into::into),
            detail: value.detail,
            kind: value.kind.map(Into::into),
            insert_text_format: value.insert_text_format.map(Into::into),
        }
    }
}

impl From<extension::CompletionLabelDetails> for CompletionLabelDetails {
    fn from(value: extension::CompletionLabelDetails) -> Self {
        Self {
            detail: value.detail,
            description: value.description,
        }
    }
}

impl From<crate::wasm_host::wit::CompletionLabelDetails> for CompletionLabelDetails {
    fn from(value: crate::wasm_host::wit::CompletionLabelDetails) -> Self {
        Self {
            detail: value.detail,
            description: value.description,
        }
    }
}

impl From<extension::CompletionKind> for CompletionKind {
    fn from(value: extension::CompletionKind) -> Self {
        match value {
            extension::CompletionKind::Text => Self::Text,
            extension::CompletionKind::Method => Self::Method,
            extension::CompletionKind::Function => Self::Function,
            extension::CompletionKind::Constructor => Self::Constructor,
            extension::CompletionKind::Field => Self::Field,
            extension::CompletionKind::Variable => Self::Variable,
            extension::CompletionKind::Class => Self::Class,
            extension::CompletionKind::Interface => Self::Interface,
            extension::CompletionKind::Module => Self::Module,
            extension::CompletionKind::Property => Self::Property,
            extension::CompletionKind::Unit => Self::Unit,
            extension::CompletionKind::Value => Self::Value,
            extension::CompletionKind::Enum => Self::Enum,
            extension::CompletionKind::Keyword => Self::Keyword,
            extension::CompletionKind::Snippet => Self::Snippet,
            extension::CompletionKind::Color => Self::Color,
            extension::CompletionKind::File => Self::File,
            extension::CompletionKind::Reference => Self::Reference,
            extension::CompletionKind::Folder => Self::Folder,
            extension::CompletionKind::EnumMember => Self::EnumMember,
            extension::CompletionKind::Constant => Self::Constant,
            extension::CompletionKind::Struct => Self::Struct,
            extension::CompletionKind::Event => Self::Event,
            extension::CompletionKind::Operator => Self::Operator,
            extension::CompletionKind::TypeParameter => Self::TypeParameter,
            extension::CompletionKind::Other(value) => Self::Other(value),
        }
    }
}

impl From<crate::wasm_host::wit::CompletionKind> for CompletionKind {
    fn from(value: crate::wasm_host::wit::CompletionKind) -> Self {
        match value {
            crate::wasm_host::wit::CompletionKind::Text => Self::Text,
            crate::wasm_host::wit::CompletionKind::Method => Self::Method,
            crate::wasm_host::wit::CompletionKind::Function => Self::Function,
            crate::wasm_host::wit::CompletionKind::Constructor => Self::Constructor,
            crate::wasm_host::wit::CompletionKind::Field => Self::Field,
            crate::wasm_host::wit::CompletionKind::Variable => Self::Variable,
            crate::wasm_host::wit::CompletionKind::Class => Self::Class,
            crate::wasm_host::wit::CompletionKind::Interface => Self::Interface,
            crate::wasm_host::wit::CompletionKind::Module => Self::Module,
            crate::wasm_host::wit::CompletionKind::Property => Self::Property,
            crate::wasm_host::wit::CompletionKind::Unit => Self::Unit,
            crate::wasm_host::wit::CompletionKind::Value => Self::Value,
            crate::wasm_host::wit::CompletionKind::Enum => Self::Enum,
            crate::wasm_host::wit::CompletionKind::Keyword => Self::Keyword,
            crate::wasm_host::wit::CompletionKind::Snippet => Self::Snippet,
            crate::wasm_host::wit::CompletionKind::Color => Self::Color,
            crate::wasm_host::wit::CompletionKind::File => Self::File,
            crate::wasm_host::wit::CompletionKind::Reference => Self::Reference,
            crate::wasm_host::wit::CompletionKind::Folder => Self::Folder,
            crate::wasm_host::wit::CompletionKind::EnumMember => Self::EnumMember,
            crate::wasm_host::wit::CompletionKind::Constant => Self::Constant,
            crate::wasm_host::wit::CompletionKind::Struct => Self::Struct,
            crate::wasm_host::wit::CompletionKind::Event => Self::Event,
            crate::wasm_host::wit::CompletionKind::Operator => Self::Operator,
            crate::wasm_host::wit::CompletionKind::TypeParameter => Self::TypeParameter,
            crate::wasm_host::wit::CompletionKind::Other(value) => Self::Other(value),
        }
    }
}

impl From<extension::InsertTextFormat> for InsertTextFormat {
    fn from(value: extension::InsertTextFormat) -> Self {
        match value {
            extension::InsertTextFormat::PlainText => Self::PlainText,
            extension::InsertTextFormat::Snippet => Self::Snippet,
            extension::InsertTextFormat::Other(value) => Self::Other(value),
        }
    }
}

impl From<crate::wasm_host::wit::InsertTextFormat> for InsertTextFormat {
    fn from(value: crate::wasm_host::wit::InsertTextFormat) -> Self {
        match value {
            crate::wasm_host::wit::InsertTextFormat::PlainText => Self::PlainText,
            crate::wasm_host::wit::InsertTextFormat::Snippet => Self::Snippet,
            crate::wasm_host::wit::InsertTextFormat::Other(value) => Self::Other(value),
        }
    }
}

impl From<extension::Symbol> for Symbol {
    fn from(value: extension::Symbol) -> Self {
        Self {
            kind: value.kind.into(),
            name: value.name,
            container_name: value.container_name,
        }
    }
}

impl From<super::since_v0_8_0::Symbol> for Symbol {
    fn from(value: super::since_v0_8_0::Symbol) -> Self {
        Self {
            kind: value.kind.into(),
            name: value.name,
            container_name: value.container_name,
        }
    }
}

impl From<extension::SymbolKind> for SymbolKind {
    fn from(value: extension::SymbolKind) -> Self {
        match value {
            extension::SymbolKind::File => Self::File,
            extension::SymbolKind::Module => Self::Module,
            extension::SymbolKind::Namespace => Self::Namespace,
            extension::SymbolKind::Package => Self::Package,
            extension::SymbolKind::Class => Self::Class,
            extension::SymbolKind::Method => Self::Method,
            extension::SymbolKind::Property => Self::Property,
            extension::SymbolKind::Field => Self::Field,
            extension::SymbolKind::Constructor => Self::Constructor,
            extension::SymbolKind::Enum => Self::Enum,
            extension::SymbolKind::Interface => Self::Interface,
            extension::SymbolKind::Function => Self::Function,
            extension::SymbolKind::Variable => Self::Variable,
            extension::SymbolKind::Constant => Self::Constant,
            extension::SymbolKind::String => Self::String,
            extension::SymbolKind::Number => Self::Number,
            extension::SymbolKind::Boolean => Self::Boolean,
            extension::SymbolKind::Array => Self::Array,
            extension::SymbolKind::Object => Self::Object,
            extension::SymbolKind::Key => Self::Key,
            extension::SymbolKind::Null => Self::Null,
            extension::SymbolKind::EnumMember => Self::EnumMember,
            extension::SymbolKind::Struct => Self::Struct,
            extension::SymbolKind::Event => Self::Event,
            extension::SymbolKind::Operator => Self::Operator,
            extension::SymbolKind::TypeParameter => Self::TypeParameter,
            extension::SymbolKind::Other(value) => Self::Other(value),
        }
    }
}

impl From<crate::wasm_host::wit::SymbolKind> for SymbolKind {
    fn from(value: crate::wasm_host::wit::SymbolKind) -> Self {
        match value {
            crate::wasm_host::wit::SymbolKind::File => Self::File,
            crate::wasm_host::wit::SymbolKind::Module => Self::Module,
            crate::wasm_host::wit::SymbolKind::Namespace => Self::Namespace,
            crate::wasm_host::wit::SymbolKind::Package => Self::Package,
            crate::wasm_host::wit::SymbolKind::Class => Self::Class,
            crate::wasm_host::wit::SymbolKind::Method => Self::Method,
            crate::wasm_host::wit::SymbolKind::Property => Self::Property,
            crate::wasm_host::wit::SymbolKind::Field => Self::Field,
            crate::wasm_host::wit::SymbolKind::Constructor => Self::Constructor,
            crate::wasm_host::wit::SymbolKind::Enum => Self::Enum,
            crate::wasm_host::wit::SymbolKind::Interface => Self::Interface,
            crate::wasm_host::wit::SymbolKind::Function => Self::Function,
            crate::wasm_host::wit::SymbolKind::Variable => Self::Variable,
            crate::wasm_host::wit::SymbolKind::Constant => Self::Constant,
            crate::wasm_host::wit::SymbolKind::String => Self::String,
            crate::wasm_host::wit::SymbolKind::Number => Self::Number,
            crate::wasm_host::wit::SymbolKind::Boolean => Self::Boolean,
            crate::wasm_host::wit::SymbolKind::Array => Self::Array,
            crate::wasm_host::wit::SymbolKind::Object => Self::Object,
            crate::wasm_host::wit::SymbolKind::Key => Self::Key,
            crate::wasm_host::wit::SymbolKind::Null => Self::Null,
            crate::wasm_host::wit::SymbolKind::EnumMember => Self::EnumMember,
            crate::wasm_host::wit::SymbolKind::Struct => Self::Struct,
            crate::wasm_host::wit::SymbolKind::Event => Self::Event,
            crate::wasm_host::wit::SymbolKind::Operator => Self::Operator,
            crate::wasm_host::wit::SymbolKind::TypeParameter => Self::TypeParameter,
            crate::wasm_host::wit::SymbolKind::Other(value) => Self::Other(value),
        }
    }
}

impl From<extension::SlashCommand> for SlashCommand {
    fn from(value: extension::SlashCommand) -> Self {
        Self {
            name: value.name,
            description: value.description,
            tooltip_text: value.tooltip_text,
            requires_argument: value.requires_argument,
        }
    }
}

impl From<super::since_v0_8_0::SlashCommand> for SlashCommand {
    fn from(value: super::since_v0_8_0::SlashCommand) -> Self {
        Self {
            name: value.name,
            description: value.description,
            tooltip_text: value.tooltip_text,
            requires_argument: value.requires_argument,
        }
    }
}

impl From<SlashCommandOutput> for extension::SlashCommandOutput {
    fn from(value: SlashCommandOutput) -> Self {
        Self {
            text: value.text,
            sections: value.sections.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<SlashCommandOutput> for super::since_v0_8_0::SlashCommandOutput {
    fn from(value: SlashCommandOutput) -> Self {
        Self {
            text: value.text,
            sections: value.sections.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<SlashCommandOutputSection> for extension::SlashCommandOutputSection {
    fn from(value: SlashCommandOutputSection) -> Self {
        Self {
            range: value.range.start as usize..value.range.end as usize,
            label: value.label,
        }
    }
}

impl From<SlashCommandOutputSection>
    for crate::wasm_host::wit::since_v0_6_0::slash_command::SlashCommandOutputSection
{
    fn from(value: SlashCommandOutputSection) -> Self {
        Self {
            range: super::since_v0_8_0::Range {
                start: value.range.start,
                end: value.range.end,
            },
            label: value.label,
        }
    }
}

impl From<SlashCommandArgumentCompletion> for extension::SlashCommandArgumentCompletion {
    fn from(value: SlashCommandArgumentCompletion) -> Self {
        Self {
            label: value.label,
            new_text: value.new_text,
            run_command: value.run_command,
        }
    }
}

impl From<SlashCommandArgumentCompletion> for super::since_v0_8_0::SlashCommandArgumentCompletion {
    fn from(value: SlashCommandArgumentCompletion) -> Self {
        Self {
            label: value.label,
            new_text: value.new_text,
            run_command: value.run_command,
        }
    }
}

impl TryFrom<ContextServerConfiguration> for extension::ContextServerConfiguration {
    type Error = anyhow::Error;

    fn try_from(value: ContextServerConfiguration) -> Result<Self, Self::Error> {
        let settings_schema: serde_json::Value = serde_json::from_str(&value.settings_schema)
            .context("Failed to parse settings_schema")?;

        Ok(Self {
            installation_instructions: value.installation_instructions,
            default_settings: value.default_settings,
            settings_schema,
        })
    }
}

impl From<remote_ui::CommandContext> for ExtensionCommandContext {
    fn from(value: remote_ui::CommandContext) -> Self {
        Self {
            workspace_id: value.workspace_id,
            trusted: value.trusted,
            active_item_kind: value.active_item_kind,
        }
    }
}

impl From<ExtensionCommandContext> for remote_ui::CommandContext {
    fn from(value: ExtensionCommandContext) -> Self {
        Self {
            workspace_id: value.workspace_id,
            trusted: value.trusted,
            active_item_kind: value.active_item_kind,
        }
    }
}

impl From<remote_ui::MountKind> for ExtensionMountKind {
    fn from(value: remote_ui::MountKind) -> Self {
        match value {
            remote_ui::MountKind::TitlebarWidget => Self::TitlebarWidget,
            remote_ui::MountKind::FooterWidget => Self::FooterWidget,
            remote_ui::MountKind::Panel => Self::Panel,
        }
    }
}

impl From<ExtensionMountKind> for remote_ui::MountKind {
    fn from(value: ExtensionMountKind) -> Self {
        match value {
            ExtensionMountKind::TitlebarWidget => Self::TitlebarWidget,
            ExtensionMountKind::FooterWidget => Self::FooterWidget,
            ExtensionMountKind::Panel => Self::Panel,
        }
    }
}

impl From<remote_ui::MountContext> for ExtensionMountContext {
    fn from(value: remote_ui::MountContext) -> Self {
        Self {
            workspace_id: value.workspace_id,
            mount_kind: value.mount_kind.into(),
            trusted: value.trusted,
            active_item_kind: value.active_item_kind,
            appearance: value.appearance,
        }
    }
}

impl From<ExtensionMountContext> for remote_ui::MountContext {
    fn from(value: ExtensionMountContext) -> Self {
        Self {
            workspace_id: value.workspace_id,
            mount_kind: value.mount_kind.into(),
            trusted: value.trusted,
            active_item_kind: value.active_item_kind,
            appearance: value.appearance,
        }
    }
}

impl From<remote_ui::RenderReason> for ExtensionRenderReason {
    fn from(value: remote_ui::RenderReason) -> Self {
        match value {
            remote_ui::RenderReason::Initial => Self::Initial,
            remote_ui::RenderReason::Event => Self::Event,
            remote_ui::RenderReason::HostContextChanged => Self::HostContextChanged,
            remote_ui::RenderReason::VirtualRangeChanged => Self::VirtualRangeChanged,
            remote_ui::RenderReason::ExplicitRefresh => Self::ExplicitRefresh,
        }
    }
}

impl From<ExtensionRenderReason> for remote_ui::RenderReason {
    fn from(value: ExtensionRenderReason) -> Self {
        match value {
            ExtensionRenderReason::Initial => Self::Initial,
            ExtensionRenderReason::Event => Self::Event,
            ExtensionRenderReason::HostContextChanged => Self::HostContextChanged,
            ExtensionRenderReason::VirtualRangeChanged => Self::VirtualRangeChanged,
            ExtensionRenderReason::ExplicitRefresh => Self::ExplicitRefresh,
        }
    }
}

impl From<remote_ui::VirtualListProps> for ExtensionVirtualListProps {
    fn from(value: remote_ui::VirtualListProps) -> Self {
        Self {
            item_count: value.item_count,
            estimated_row_height: value.estimated_row_height,
            selection_mode: value.selection_mode,
        }
    }
}

impl From<ExtensionVirtualListProps> for remote_ui::VirtualListProps {
    fn from(value: ExtensionVirtualListProps) -> Self {
        Self {
            item_count: value.item_count,
            estimated_row_height: value.estimated_row_height,
            selection_mode: value.selection_mode,
        }
    }
}

impl From<remote_ui::ProgressBarProps> for ExtensionProgressBarProps {
    fn from(value: remote_ui::ProgressBarProps) -> Self {
        Self {
            value: value.value,
            max_value: value.max_value,
        }
    }
}

impl From<ExtensionProgressBarProps> for remote_ui::ProgressBarProps {
    fn from(value: ExtensionProgressBarProps) -> Self {
        Self {
            value: value.value,
            max_value: value.max_value,
        }
    }
}

impl From<remote_ui::RemoteViewNodeKind> for ExtensionRemoteViewNodeKind {
    fn from(value: remote_ui::RemoteViewNodeKind) -> Self {
        match value {
            remote_ui::RemoteViewNodeKind::Row => Self::Row,
            remote_ui::RemoteViewNodeKind::Column => Self::Column,
            remote_ui::RemoteViewNodeKind::Stack => Self::Stack,
            remote_ui::RemoteViewNodeKind::Text(text) => Self::Text(text),
            remote_ui::RemoteViewNodeKind::Icon(icon) => Self::Icon(icon),
            remote_ui::RemoteViewNodeKind::Button(label) => Self::Button(label),
            remote_ui::RemoteViewNodeKind::Toggle(value) => Self::Toggle(value),
            remote_ui::RemoteViewNodeKind::Checkbox(value) => Self::Checkbox(value),
            remote_ui::RemoteViewNodeKind::TextInput(value) => Self::TextInput(value),
            remote_ui::RemoteViewNodeKind::Badge(value) => Self::Badge(value),
            remote_ui::RemoteViewNodeKind::ProgressBar(props) => Self::ProgressBar(props.into()),
            remote_ui::RemoteViewNodeKind::Divider => Self::Divider,
            remote_ui::RemoteViewNodeKind::Spacer => Self::Spacer,
            remote_ui::RemoteViewNodeKind::ScrollView => Self::ScrollView,
            remote_ui::RemoteViewNodeKind::VirtualList(props) => Self::VirtualList(props.into()),
        }
    }
}

impl From<ExtensionRemoteViewNodeKind> for remote_ui::RemoteViewNodeKind {
    fn from(value: ExtensionRemoteViewNodeKind) -> Self {
        match value {
            ExtensionRemoteViewNodeKind::Row => Self::Row,
            ExtensionRemoteViewNodeKind::Column => Self::Column,
            ExtensionRemoteViewNodeKind::Stack => Self::Stack,
            ExtensionRemoteViewNodeKind::Text(text) => Self::Text(text),
            ExtensionRemoteViewNodeKind::Icon(icon) => Self::Icon(icon),
            ExtensionRemoteViewNodeKind::Button(label) => Self::Button(label),
            ExtensionRemoteViewNodeKind::Toggle(value) => Self::Toggle(value),
            ExtensionRemoteViewNodeKind::Checkbox(value) => Self::Checkbox(value),
            ExtensionRemoteViewNodeKind::TextInput(value) => Self::TextInput(value),
            ExtensionRemoteViewNodeKind::Badge(value) => Self::Badge(value),
            ExtensionRemoteViewNodeKind::ProgressBar(props) => Self::ProgressBar(props.into()),
            ExtensionRemoteViewNodeKind::Divider => Self::Divider,
            ExtensionRemoteViewNodeKind::Spacer => Self::Spacer,
            ExtensionRemoteViewNodeKind::ScrollView => Self::ScrollView,
            ExtensionRemoteViewNodeKind::VirtualList(props) => Self::VirtualList(props.into()),
        }
    }
}

impl From<remote_ui::RemoteViewProperty> for ExtensionRemoteViewProperty {
    fn from(value: remote_ui::RemoteViewProperty) -> Self {
        Self {
            name: value.name,
            value: value.value,
        }
    }
}

impl From<ExtensionRemoteViewProperty> for remote_ui::RemoteViewProperty {
    fn from(value: ExtensionRemoteViewProperty) -> Self {
        Self {
            name: value.name,
            value: value.value,
        }
    }
}

impl TryFrom<remote_ui::RemoteViewNode> for ExtensionRemoteViewNode {
    type Error = anyhow::Error;

    fn try_from(value: remote_ui::RemoteViewNode) -> Result<Self, Self::Error> {
        Ok(Self {
            node_id: value.node_id,
            parent_id: value.parent_id,
            kind: value.kind.into(),
            properties: value.properties.into_iter().map(Into::into).collect(),
        })
    }
}

impl From<ExtensionRemoteViewNode> for remote_ui::RemoteViewNode {
    fn from(value: ExtensionRemoteViewNode) -> Self {
        Self {
            node_id: value.node_id,
            parent_id: value.parent_id,
            kind: value.kind.into(),
            properties: value.properties.into_iter().map(Into::into).collect(),
        }
    }
}

impl TryFrom<remote_ui::RemoteViewTree> for ExtensionRemoteViewTree {
    type Error = anyhow::Error;

    fn try_from(value: remote_ui::RemoteViewTree) -> Result<Self, Self::Error> {
        Ok(Self {
            revision: value.revision,
            root_id: value.root_id,
            nodes: value
                .nodes
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

impl From<ExtensionRemoteViewTree> for remote_ui::RemoteViewTree {
    fn from(value: ExtensionRemoteViewTree) -> Self {
        Self {
            revision: value.revision,
            root_id: value.root_id,
            nodes: value.nodes.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<remote_ui::RemoteViewEventKind> for ExtensionRemoteViewEventKind {
    fn from(value: remote_ui::RemoteViewEventKind) -> Self {
        match value {
            remote_ui::RemoteViewEventKind::Click => Self::Click,
            remote_ui::RemoteViewEventKind::Change => Self::Change,
            remote_ui::RemoteViewEventKind::Submit => Self::Submit,
            remote_ui::RemoteViewEventKind::ListItemActivated => Self::ListItemActivated,
            remote_ui::RemoteViewEventKind::ListSelectionChanged => Self::ListSelectionChanged,
        }
    }
}

impl From<ExtensionRemoteViewEventKind> for remote_ui::RemoteViewEventKind {
    fn from(value: ExtensionRemoteViewEventKind) -> Self {
        match value {
            ExtensionRemoteViewEventKind::Click => Self::Click,
            ExtensionRemoteViewEventKind::Change => Self::Change,
            ExtensionRemoteViewEventKind::Submit => Self::Submit,
            ExtensionRemoteViewEventKind::ListItemActivated => Self::ListItemActivated,
            ExtensionRemoteViewEventKind::ListSelectionChanged => Self::ListSelectionChanged,
        }
    }
}

impl From<remote_ui::RemoteViewEvent> for ExtensionRemoteViewEvent {
    fn from(value: remote_ui::RemoteViewEvent) -> Self {
        Self {
            node_id: value.node_id,
            kind: value.kind.into(),
            payload_json: value.payload_json,
        }
    }
}

impl From<ExtensionRemoteViewEvent> for remote_ui::RemoteViewEvent {
    fn from(value: ExtensionRemoteViewEvent) -> Self {
        Self {
            node_id: value.node_id,
            kind: value.kind.into(),
            payload_json: value.payload_json,
        }
    }
}

impl From<remote_ui::VirtualListRange> for ExtensionVirtualListRange {
    fn from(value: remote_ui::VirtualListRange) -> Self {
        Self {
            start: value.start,
            end: value.end,
        }
    }
}

impl From<ExtensionVirtualListRange> for remote_ui::VirtualListRange {
    fn from(value: ExtensionVirtualListRange) -> Self {
        Self {
            start: value.start,
            end: value.end,
        }
    }
}

impl From<remote_ui::EventOutcome> for ExtensionEventOutcome {
    fn from(value: remote_ui::EventOutcome) -> Self {
        match value {
            remote_ui::EventOutcome::Noop => Self::Noop,
            remote_ui::EventOutcome::Rerender => Self::Rerender,
            remote_ui::EventOutcome::RerenderVirtualRange(node_id) => {
                Self::RerenderVirtualRange(node_id)
            }
            remote_ui::EventOutcome::ShowError(message) => Self::ShowError(message),
        }
    }
}

impl From<ExtensionEventOutcome> for remote_ui::EventOutcome {
    fn from(value: ExtensionEventOutcome) -> Self {
        match value {
            ExtensionEventOutcome::Noop => Self::Noop,
            ExtensionEventOutcome::Rerender => Self::Rerender,
            ExtensionEventOutcome::RerenderVirtualRange(node_id) => {
                Self::RerenderVirtualRange(node_id)
            }
            ExtensionEventOutcome::ShowError(message) => Self::ShowError(message),
        }
    }
}

impl From<remote_ui::HostMutation> for ExtensionHostMutation {
    fn from(value: remote_ui::HostMutation) -> Self {
        match value {
            remote_ui::HostMutation::ShowToast(message) => Self::ShowToast(message),
            remote_ui::HostMutation::OpenPanel(panel_id) => Self::OpenPanel(panel_id),
            remote_ui::HostMutation::ClosePanel(panel_id) => Self::ClosePanel(panel_id),
            remote_ui::HostMutation::CopyToClipboard(value) => Self::CopyToClipboard(value),
            remote_ui::HostMutation::OpenExternalUrl(url) => Self::OpenExternalUrl(url),
        }
    }
}

impl From<ExtensionHostMutation> for remote_ui::HostMutation {
    fn from(value: ExtensionHostMutation) -> Self {
        match value {
            ExtensionHostMutation::ShowToast(message) => Self::ShowToast(message),
            ExtensionHostMutation::OpenPanel(panel_id) => Self::OpenPanel(panel_id),
            ExtensionHostMutation::ClosePanel(panel_id) => Self::ClosePanel(panel_id),
            ExtensionHostMutation::CopyToClipboard(value) => Self::CopyToClipboard(value),
            ExtensionHostMutation::OpenExternalUrl(url) => Self::OpenExternalUrl(url),
        }
    }
}

impl HostKeyValueStore for WasmState {
    async fn insert(
        &mut self,
        kv_store: Resource<ExtensionKeyValueStore>,
        key: String,
        value: String,
    ) -> wasmtime::Result<Result<(), String>> {
        let kv_store = self.table.get(&kv_store)?;
        kv_store.insert(key, value).await.to_wasmtime_result()
    }

    async fn drop(&mut self, _worktree: Resource<ExtensionKeyValueStore>) -> Result<()> {
        // We only ever hand out borrows of key-value stores.
        Ok(())
    }
}

impl HostProject for WasmState {
    async fn worktree_ids(
        &mut self,
        project: Resource<ExtensionProject>,
    ) -> wasmtime::Result<Vec<u64>> {
        let project = self.table.get(&project)?;
        Ok(project.worktree_ids())
    }

    async fn drop(&mut self, _project: Resource<Project>) -> Result<()> {
        // We only ever hand out borrows of projects.
        Ok(())
    }
}

impl HostWorktree for WasmState {
    async fn id(&mut self, delegate: Resource<Arc<dyn WorktreeDelegate>>) -> wasmtime::Result<u64> {
        let delegate = self.table.get(&delegate)?;
        Ok(delegate.id())
    }

    async fn root_path(
        &mut self,
        delegate: Resource<Arc<dyn WorktreeDelegate>>,
    ) -> wasmtime::Result<String> {
        let delegate = self.table.get(&delegate)?;
        Ok(delegate.root_path())
    }

    async fn read_text_file(
        &mut self,
        delegate: Resource<Arc<dyn WorktreeDelegate>>,
        path: String,
    ) -> wasmtime::Result<Result<String, String>> {
        let delegate = self.table.get(&delegate)?;
        Ok(delegate
            .read_text_file(&RelPath::new(Path::new(&path), PathStyle::Posix)?)
            .await
            .map_err(|error| error.to_string()))
    }

    async fn shell_env(
        &mut self,
        delegate: Resource<Arc<dyn WorktreeDelegate>>,
    ) -> wasmtime::Result<EnvVars> {
        let delegate = self.table.get(&delegate)?;
        Ok(delegate.shell_env().await.into_iter().collect())
    }

    async fn which(
        &mut self,
        delegate: Resource<Arc<dyn WorktreeDelegate>>,
        binary_name: String,
    ) -> wasmtime::Result<Option<String>> {
        let delegate = self.table.get(&delegate)?;
        Ok(delegate.which(binary_name).await)
    }

    async fn drop(&mut self, _worktree: Resource<Worktree>) -> Result<()> {
        // We only ever hand out borrows of worktrees.
        Ok(())
    }
}

impl common::Host for WasmState {}

impl remote_ui::Host for WasmState {
    async fn dispatch_workspace_action(
        &mut self,
        workspace_id: u64,
        action_id: String,
        payload_json: Option<String>,
    ) -> wasmtime::Result<Result<(), String>> {
        let proxy = self.host.proxy.clone();
        Ok(self
            .on_main_thread(move |cx| {
                async move {
                    cx.update(|cx| {
                        proxy
                            .dispatch_remote_ui_workspace_action(
                                workspace_id,
                                &action_id,
                                payload_json.as_deref(),
                                cx,
                            )
                            .map_err(|error: anyhow::Error| error.to_string())
                    })
                }
                .boxed_local()
            })
            .await)
    }

    async fn request_host_mutation(
        &mut self,
        workspace_id: u64,
        mutation: remote_ui::HostMutation,
    ) -> wasmtime::Result<Result<(), String>> {
        let proxy = self.host.proxy.clone();
        let extension_id = self.manifest.id.clone();
        let mutation = ExtensionHostMutation::from(mutation);
        Ok(self
            .on_main_thread(move |cx| {
                async move {
                    cx.update(|cx| {
                        proxy
                            .request_remote_ui_host_mutation(
                                extension_id.as_ref(),
                                workspace_id,
                                mutation,
                                cx,
                            )
                            .map_err(|error: anyhow::Error| error.to_string())
                    })
                }
                .boxed_local()
            })
            .await)
    }
}

impl sidecar::Host for WasmState {
    async fn call(
        &mut self,
        method: String,
        params_json: Option<String>,
        timeout_ms: Option<u32>,
    ) -> wasmtime::Result<Result<String, String>> {
        let params = params_json
            .map(|params_json| {
                serde_json::from_str(&params_json)
                    .map_err(|error| anyhow!("invalid sidecar params JSON: {error}"))
            })
            .transpose()?;
        let timeout = timeout_ms.map(|timeout_ms| Duration::from_millis(timeout_ms as u64));
        Ok(self
            .request_sidecar(&method, params, timeout)
            .await
            .and_then(|value| serde_json::to_string(&value).context("serializing sidecar response"))
            .map_err(|error| error.to_string()))
    }

    async fn close(&mut self) -> wasmtime::Result<Result<(), String>> {
        Ok(self.close_sidecar().map_err(|error| error.to_string()))
    }
}

impl http_client::Host for WasmState {
    async fn fetch(
        &mut self,
        request: http_client::HttpRequest,
    ) -> wasmtime::Result<Result<http_client::HttpResponse, String>> {
        maybe!(async {
            let url = &request.url;
            let request = convert_request(&request)?;
            let mut response = self.host.http_client.send(request).await?;

            if response.status().is_client_error() || response.status().is_server_error() {
                bail!("failed to fetch '{url}': status code {}", response.status())
            }
            convert_response(&mut response).await
        })
        .await
        .to_wasmtime_result()
    }

    async fn fetch_stream(
        &mut self,
        request: http_client::HttpRequest,
    ) -> wasmtime::Result<Result<Resource<ExtensionHttpResponseStream>, String>> {
        let request = convert_request(&request)?;
        let response = self.host.http_client.send(request);
        maybe!(async {
            let response = response.await?;
            let stream = Arc::new(Mutex::new(response));
            let resource = self.table.push(stream)?;
            Ok(resource)
        })
        .await
        .to_wasmtime_result()
    }
}

impl http_client::HostHttpResponseStream for WasmState {
    async fn next_chunk(
        &mut self,
        resource: Resource<ExtensionHttpResponseStream>,
    ) -> wasmtime::Result<Result<Option<Vec<u8>>, String>> {
        let stream = self.table.get(&resource)?.clone();
        maybe!(async move {
            let mut response = stream.lock().await;
            let mut buffer = vec![0; 8192]; // 8KB buffer
            let bytes_read = response.body_mut().read(&mut buffer).await?;
            if bytes_read == 0 {
                Ok(None)
            } else {
                buffer.truncate(bytes_read);
                Ok(Some(buffer))
            }
        })
        .await
        .to_wasmtime_result()
    }

    async fn drop(&mut self, _resource: Resource<ExtensionHttpResponseStream>) -> Result<()> {
        Ok(())
    }
}

impl From<http_client::HttpMethod> for ::http_client::Method {
    fn from(value: http_client::HttpMethod) -> Self {
        match value {
            http_client::HttpMethod::Get => Self::GET,
            http_client::HttpMethod::Post => Self::POST,
            http_client::HttpMethod::Put => Self::PUT,
            http_client::HttpMethod::Delete => Self::DELETE,
            http_client::HttpMethod::Head => Self::HEAD,
            http_client::HttpMethod::Options => Self::OPTIONS,
            http_client::HttpMethod::Patch => Self::PATCH,
        }
    }
}

fn convert_request(
    extension_request: &http_client::HttpRequest,
) -> anyhow::Result<::http_client::Request<AsyncBody>> {
    let mut request = ::http_client::Request::builder()
        .method(::http_client::Method::from(extension_request.method))
        .uri(&extension_request.url)
        .follow_redirects(match extension_request.redirect_policy {
            http_client::RedirectPolicy::NoFollow => ::http_client::RedirectPolicy::NoFollow,
            http_client::RedirectPolicy::FollowLimit(limit) => {
                ::http_client::RedirectPolicy::FollowLimit(limit)
            }
            http_client::RedirectPolicy::FollowAll => ::http_client::RedirectPolicy::FollowAll,
        });
    for (key, value) in &extension_request.headers {
        request = request.header(key, value);
    }
    let body = extension_request
        .body
        .clone()
        .map(AsyncBody::from)
        .unwrap_or_default();
    request.body(body).map_err(anyhow::Error::from)
}

async fn convert_response(
    response: &mut ::http_client::Response<AsyncBody>,
) -> anyhow::Result<http_client::HttpResponse> {
    let mut extension_response = http_client::HttpResponse {
        body: Vec::new(),
        headers: Vec::new(),
    };

    for (key, value) in response.headers() {
        extension_response
            .headers
            .push((key.to_string(), value.to_str().unwrap_or("").to_string()));
    }

    response
        .body_mut()
        .read_to_end(&mut extension_response.body)
        .await?;

    Ok(extension_response)
}

impl nodejs::Host for WasmState {
    async fn node_binary_path(&mut self) -> wasmtime::Result<Result<String, String>> {
        self.host
            .node_runtime
            .binary_path()
            .await
            .map(|path| path.to_string_lossy().into_owned())
            .to_wasmtime_result()
    }

    async fn npm_package_latest_version(
        &mut self,
        package_name: String,
    ) -> wasmtime::Result<Result<String, String>> {
        self.host
            .node_runtime
            .npm_package_latest_version(&package_name)
            .await
            .map(|v| v.to_string())
            .to_wasmtime_result()
    }

    async fn npm_package_installed_version(
        &mut self,
        package_name: String,
    ) -> wasmtime::Result<Result<Option<String>, String>> {
        self.host
            .node_runtime
            .npm_package_installed_version(&self.work_dir(), &package_name)
            .await
            .map(|option| option.map(|version| version.to_string()))
            .to_wasmtime_result()
    }

    async fn npm_install_package(
        &mut self,
        package_name: String,
        version: String,
    ) -> wasmtime::Result<Result<(), String>> {
        self.capability_granter
            .grant_npm_install_package(&package_name)?;

        self.host
            .node_runtime
            .npm_install_packages(&self.work_dir(), &[(&package_name, &version)])
            .await
            .to_wasmtime_result()
    }
}

#[async_trait]
impl lsp::Host for WasmState {}

impl From<::http_client::github::GithubRelease> for github::GithubRelease {
    fn from(value: ::http_client::github::GithubRelease) -> Self {
        Self {
            version: value.tag_name,
            assets: value.assets.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<::http_client::github::GithubReleaseAsset> for github::GithubReleaseAsset {
    fn from(value: ::http_client::github::GithubReleaseAsset) -> Self {
        Self {
            name: value.name,
            download_url: value.browser_download_url,
            digest: value.digest,
        }
    }
}

impl github::Host for WasmState {
    async fn latest_github_release(
        &mut self,
        repo: String,
        options: github::GithubReleaseOptions,
    ) -> wasmtime::Result<Result<github::GithubRelease, String>> {
        maybe!(async {
            let release = ::http_client::github::latest_github_release(
                &repo,
                options.require_assets,
                options.pre_release,
                self.host.http_client.clone(),
            )
            .await?;
            Ok(release.into())
        })
        .await
        .to_wasmtime_result()
    }

    async fn github_release_by_tag_name(
        &mut self,
        repo: String,
        tag: String,
    ) -> wasmtime::Result<Result<github::GithubRelease, String>> {
        maybe!(async {
            let release = ::http_client::github::get_release_by_tag_name(
                &repo,
                &tag,
                self.host.http_client.clone(),
            )
            .await?;
            Ok(release.into())
        })
        .await
        .to_wasmtime_result()
    }
}

impl platform::Host for WasmState {
    async fn current_platform(&mut self) -> Result<(platform::Os, platform::Architecture)> {
        Ok((
            match env::consts::OS {
                "macos" => platform::Os::Mac,
                "linux" => platform::Os::Linux,
                "windows" => platform::Os::Windows,
                _ => panic!("unsupported os"),
            },
            match env::consts::ARCH {
                "aarch64" => platform::Architecture::Aarch64,
                "x86" => platform::Architecture::X86,
                "x86_64" => platform::Architecture::X8664,
                _ => panic!("unsupported architecture"),
            },
        ))
    }
}

impl From<std::process::Output> for process::Output {
    fn from(output: std::process::Output) -> Self {
        Self {
            status: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        }
    }
}

impl process::Host for WasmState {
    async fn run_command(
        &mut self,
        command: process::Command,
    ) -> wasmtime::Result<Result<process::Output, String>> {
        maybe!(async {
            self.capability_granter
                .grant_exec(&command.command, &command.args)?;

            let output = util::command::new_command(command.command.as_str())
                .args(&command.args)
                .envs(command.env)
                .output()
                .await?;

            Ok(output.into())
        })
        .await
        .to_wasmtime_result()
    }
}

#[async_trait]
impl slash_command::Host for WasmState {}

#[async_trait]
impl context_server::Host for WasmState {}

impl dap::Host for WasmState {
    async fn resolve_tcp_template(
        &mut self,
        template: TcpArgumentsTemplate,
    ) -> wasmtime::Result<Result<TcpArguments, String>> {
        maybe!(async {
            let (host, port, timeout) =
                ::dap::configure_tcp_connection(task::TcpArgumentsTemplate {
                    port: template.port,
                    host: template.host.map(Ipv4Addr::from_bits),
                    timeout: template.timeout,
                })
                .await?;
            Ok(TcpArguments {
                port,
                host: host.to_bits(),
                timeout,
            })
        })
        .await
        .to_wasmtime_result()
    }
}

impl ExtensionImports for WasmState {
    async fn get_settings(
        &mut self,
        location: Option<self::SettingsLocation>,
        category: String,
        key: Option<String>,
    ) -> wasmtime::Result<Result<String, String>> {
        self.on_main_thread(|cx| {
            async move {
                let path = location.as_ref().and_then(|location| {
                    RelPath::new(Path::new(&location.path), PathStyle::Posix).ok()
                });
                let location = path
                    .as_ref()
                    .zip(location.as_ref())
                    .map(|(path, location)| ::settings::SettingsLocation {
                        worktree_id: WorktreeId::from_proto(location.worktree_id),
                        path,
                    });

                cx.update(|cx| match category.as_str() {
                    "language" => {
                        let key = key.map(|k| LanguageName::new(&k));
                        let settings = AllLanguageSettings::get(location, cx).language(
                            location,
                            key.as_ref(),
                            cx,
                        );
                        Ok(serde_json::to_string(&settings::LanguageSettings {
                            tab_size: settings.tab_size,
                        })?)
                    }
                    "lsp" => {
                        let settings = key
                            .and_then(|key| {
                                ProjectSettings::get(location, cx)
                                    .lsp
                                    .get(&::lsp::LanguageServerName::from_proto(key))
                            })
                            .cloned()
                            .unwrap_or_default();
                        Ok(serde_json::to_string(&settings::LspSettings {
                            binary: settings.binary.map(|binary| settings::CommandSettings {
                                path: binary.path,
                                arguments: binary.arguments,
                                env: binary.env.map(|env| env.into_iter().collect()),
                            }),
                            settings: settings.settings,
                            initialization_options: settings.initialization_options,
                        })?)
                    }
                    "context_servers" => {
                        let settings = key
                            .and_then(|key| {
                                ProjectSettings::get(location, cx)
                                    .context_servers
                                    .get(key.as_str())
                            })
                            .cloned()
                            .unwrap_or_else(|| {
                                project::project_settings::ContextServerSettings::default_extension(
                                )
                            });

                        match settings {
                            project::project_settings::ContextServerSettings::Stdio {
                                enabled: _,
                                command,
                                ..
                            } => Ok(serde_json::to_string(&settings::ContextServerSettings {
                                command: Some(settings::CommandSettings {
                                    path: command.path.to_str().map(|path| path.to_string()),
                                    arguments: Some(command.args),
                                    env: command.env.map(|env| env.into_iter().collect()),
                                }),
                                settings: None,
                            })?),
                            project::project_settings::ContextServerSettings::Extension {
                                enabled: _,
                                settings,
                                ..
                            } => Ok(serde_json::to_string(&settings::ContextServerSettings {
                                command: None,
                                settings: Some(settings),
                            })?),
                            project::project_settings::ContextServerSettings::Http { .. } => {
                                bail!("remote context server settings not supported in 0.6.0")
                            }
                        }
                    }
                    _ => {
                        bail!("Unknown settings category: {}", category);
                    }
                })
            }
            .boxed_local()
        })
        .await
        .to_wasmtime_result()
    }

    async fn set_language_server_installation_status(
        &mut self,
        server_name: String,
        status: LanguageServerInstallationStatus,
    ) -> wasmtime::Result<()> {
        let status = match status {
            LanguageServerInstallationStatus::CheckingForUpdate => BinaryStatus::CheckingForUpdate,
            LanguageServerInstallationStatus::Downloading => BinaryStatus::Downloading,
            LanguageServerInstallationStatus::None => BinaryStatus::None,
            LanguageServerInstallationStatus::Failed(error) => BinaryStatus::Failed { error },
        };

        self.host
            .proxy
            .update_language_server_status(::lsp::LanguageServerName(server_name.into()), status);

        Ok(())
    }

    async fn download_file(
        &mut self,
        url: String,
        path: String,
        file_type: DownloadedFileType,
    ) -> wasmtime::Result<Result<(), String>> {
        maybe!(async {
            let parsed_url = Url::parse(&url)?;
            self.capability_granter.grant_download_file(&parsed_url)?;

            let path = PathBuf::from(path);
            let extension_work_dir = self.host.work_dir.join(self.manifest.id.as_ref());

            self.host.fs.create_dir(&extension_work_dir).await?;

            let destination_path = self
                .host
                .writeable_path_from_extension(&self.manifest.id, &path)
                .await?;

            let mut response = self
                .host
                .http_client
                .get(&url, Default::default(), true)
                .await
                .context("downloading release")?;

            anyhow::ensure!(
                response.status().is_success(),
                "download failed with status {}",
                response.status()
            );
            let body = BufReader::new(response.body_mut());

            match file_type {
                DownloadedFileType::Uncompressed => {
                    futures::pin_mut!(body);
                    self.host
                        .fs
                        .create_file_with(&destination_path, body)
                        .await?;
                }
                DownloadedFileType::Gzip => {
                    let body = GzipDecoder::new(body);
                    futures::pin_mut!(body);
                    self.host
                        .fs
                        .create_file_with(&destination_path, body)
                        .await?;
                }
                DownloadedFileType::GzipTar => {
                    let body = GzipDecoder::new(body);
                    futures::pin_mut!(body);
                    self.host
                        .fs
                        .extract_tar_file(&destination_path, Archive::new(body))
                        .await?;
                }
                DownloadedFileType::Zip => {
                    futures::pin_mut!(body);
                    extract_zip(&destination_path, body)
                        .await
                        .with_context(|| format!("unzipping {path:?} archive"))?;
                }
            }

            Ok(())
        })
        .await
        .to_wasmtime_result()
    }

    async fn make_file_executable(&mut self, path: String) -> wasmtime::Result<Result<(), String>> {
        let path = self
            .host
            .writeable_path_from_extension(&self.manifest.id, Path::new(&path))
            .await?;

        make_file_executable(&path)
            .await
            .with_context(|| format!("setting permissions for path {path:?}"))
            .to_wasmtime_result()
    }
}
