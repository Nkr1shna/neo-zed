use std::{path::Path, sync::Arc};

use acp_thread::AcpThread;
use action_log::DiffStats;
use agent::{ContextServerRegistry, Thread};
use agent_client_protocol as acp;
use anyhow::{Result, anyhow};
use gpui::{App, AsyncWindowContext, BorrowAppContext, Context, Entity, EntityId, Global, SharedString, Task, WeakEntity, Window};
use prompt_store::PromptStore;
use ui::{AgentThreadStatus, IconName};
use util::path_list::PathList;
use workspace::{MultiWorkspace, Workspace};
use zed_actions::assistant::OpenRulesLibrary;
use zed_actions::assistant::{Toggle, ToggleFocus};

use crate::{
    Agent, AgentDiffPane, AgentInitialContent, AiWorkspace, AiWorkspaceEvent, AgentWorkspaceItem,
    ExternalSourcePrompt, NewNativeAgentThreadFromSummary, NewThread, StartThreadIn,
    TextThreadEditor, ThreadHistory, ToggleNavigationMenu, ToggleNewThreadMenu,
    ToggleOptionsMenu, agent_connection_store::AgentConnectionStore,
};

#[derive(Default)]
struct AgentWorkspaceControllers {
    by_workspace: collections::HashMap<EntityId, Entity<AiWorkspace>>,
}

impl Global for AgentWorkspaceControllers {}

#[derive(Clone, Debug)]
pub struct AgentThreadSummary {
    pub session_id: acp::SessionId,
    pub title: SharedString,
    pub status: AgentThreadStatus,
    pub icon: IconName,
    pub icon_from_external_svg: Option<SharedString>,
    pub is_background: bool,
    pub is_title_generating: bool,
    pub diff_stats: DiffStats,
}

pub fn initialize(
    workspace: WeakEntity<Workspace>,
    prompt_builder: Arc<prompt_store::PromptBuilder>,
    cx: AsyncWindowContext,
) -> Task<Result<()>> {
    cx.spawn(async move |cx| {
        let panel = AiWorkspace::load(workspace.clone(), prompt_builder, cx.clone()).await?;
        workspace.update_in(cx, |workspace, window, cx| {
            attach_workspace_controller(workspace, panel.clone(), window, cx);
        })?;
        anyhow::Ok(())
    })
}

fn register_workspace_controller(
    workspace: &Workspace,
    panel: Entity<AiWorkspace>,
    cx: &mut App,
) {
    let workspace_id = workspace.weak_handle().entity_id();
    cx.update_default_global(|controllers: &mut AgentWorkspaceControllers, _cx| {
        controllers.by_workspace.insert(workspace_id, panel);
    });
}

pub fn attach_workspace_controller(
    workspace: &Workspace,
    panel: Entity<AiWorkspace>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let workspace_id = workspace.weak_handle().entity_id();
    register_workspace_controller(workspace, panel.clone(), cx);
    cx.on_release(move |_workspace, cx| {
        cx.update_default_global(|controllers: &mut AgentWorkspaceControllers, _cx| {
            controllers.by_workspace.remove(&workspace_id);
        });
    })
    .detach();
    cx.subscribe_in(
        &panel,
        window,
        |_workspace, _panel, _event: &AiWorkspaceEvent, _window, cx| {
            cx.emit(workspace::Event::AiSurfaceChanged);
        },
    )
    .detach();
}

pub fn workspace_controller(
    workspace: &Workspace,
    cx: &App,
) -> Option<Entity<AiWorkspace>> {
    cx.try_global::<AgentWorkspaceControllers>()
        .and_then(|controllers| controllers.by_workspace.get(&workspace.weak_handle().entity_id()))
        .cloned()
}

pub(crate) fn deploy_panel_in_center(
    panel: Entity<AiWorkspace>,
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    AgentWorkspaceItem::deploy_in_workspace(panel, workspace, window, cx);
}

pub(crate) fn deploy_active_panel_item_in_center(
    panel: &Entity<AiWorkspace>,
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    deploy_panel_in_center(panel.clone(), workspace, window, cx);
    true
}

pub(crate) fn deploy_active_panel_item_in_center_from_panel(
    panel: &AiWorkspace,
    window: &mut Window,
    cx: &mut Context<AiWorkspace>,
) -> bool {
    let Some(workspace) = panel.workspace().upgrade() else {
        return false;
    };

    let Some(workspace_panel) = workspace_controller(&workspace.read(cx), cx) else {
        return false;
    };
    cx.defer_in(window, move |_panel, window, cx| {
        workspace.update(cx, |workspace: &mut Workspace, cx| {
            deploy_panel_in_center(workspace_panel, workspace, window, cx);
        });
    });
    true
}

pub fn focus_ai_surface(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    let Some(panel) = workspace_controller(workspace, cx) else {
        return false;
    };
    deploy_panel_in_center(panel, workspace, window, cx);
    true
}

pub fn toggle_focus(
    workspace: &mut Workspace,
    _: &ToggleFocus,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let Some(panel) = workspace_controller(workspace, cx) else {
        return;
    };

    if panel.read(cx).enabled(cx) {
        if deploy_active_panel_item_in_center(&panel, workspace, window, cx) {
            return;
        }

        let selected_agent = panel.read(cx).selected_agent();
        if let Some(agent) = selected_agent {
            let _ = new_external_agent_thread_in_center(workspace, Some(agent), window, cx);
        } else {
            let _ = new_text_thread_in_center(workspace, window, cx);
        }
    }
}

pub fn toggle(
    workspace: &mut Workspace,
    _: &Toggle,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let Some(panel) = workspace_controller(workspace, cx) else {
        return;
    };

    if panel.read(cx).enabled(cx) {
        if deploy_active_panel_item_in_center(&panel, workspace, window, cx) {
            return;
        }

        let selected_agent = panel.read(cx).selected_agent();
        if let Some(agent) = selected_agent {
            let _ = new_external_agent_thread_in_center(workspace, Some(agent), window, cx);
        } else {
            let _ = new_text_thread_in_center(workspace, window, cx);
        }
    }
}

pub fn new_agent_thread_with_external_source_prompt_in_center(
    workspace: &mut Workspace,
    external_source_prompt: Option<ExternalSourcePrompt>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    let Some(panel) = workspace_controller(workspace, cx) else {
        return false;
    };

    panel.update(cx, |panel, cx| {
        panel.new_agent_thread_with_external_source_prompt(
            external_source_prompt,
            true,
            window,
            cx,
        );
    });

    deploy_active_panel_item_in_center(&panel, workspace, window, cx)
}

pub(crate) fn new_thread_in_center(
    workspace: &mut Workspace,
    action: &NewThread,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    let Some(panel) = workspace_controller(workspace, cx) else {
        return false;
    };

    panel.update(cx, |panel, cx| panel.new_thread(action, window, cx));
    deploy_active_panel_item_in_center(&panel, workspace, window, cx)
}

pub(crate) fn new_native_agent_thread_from_summary_in_center(
    workspace: &mut Workspace,
    action: &NewNativeAgentThreadFromSummary,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    let Some(panel) = workspace_controller(workspace, cx) else {
        return false;
    };

    panel.update(cx, |panel, cx| {
        panel.new_native_agent_thread_from_summary(action, window, cx);
    });
    true
}

pub(crate) fn new_text_thread_in_center(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    let Some(panel) = workspace_controller(workspace, cx) else {
        return false;
    };

    panel.update(cx, |panel, cx| {
        panel.new_text_thread(window, cx);
    });
    deploy_active_panel_item_in_center(&panel, workspace, window, cx)
}

pub(crate) fn new_external_agent_thread_in_center(
    workspace: &mut Workspace,
    agent: Option<Agent>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    let Some(panel) = workspace_controller(workspace, cx) else {
        return false;
    };

    panel.update(cx, |panel, cx| {
        panel.external_thread(agent, None, None, None, None, true, window, cx);
    });
    deploy_active_panel_item_in_center(&panel, workspace, window, cx)
}

pub(crate) fn new_initial_content_thread_in_center(
    workspace: &mut Workspace,
    initial_content: AgentInitialContent,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    let Some(panel) = workspace_controller(workspace, cx) else {
        return false;
    };

    panel.update(cx, |panel, cx| {
        panel.external_thread(
            None,
            None,
            None,
            None,
            Some(initial_content),
            true,
            window,
            cx,
        );
    });
    deploy_active_panel_item_in_center(&panel, workspace, window, cx)
}

pub(crate) fn expand_message_editor(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    let Some(panel) = workspace_controller(workspace, cx) else {
        return false;
    };

    deploy_panel_in_center(panel.clone(), workspace, window, cx);
    panel.update(cx, |panel, cx| panel.expand_message_editor(window, cx));
    true
}

pub(crate) fn open_history(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    let Some(panel) = workspace_controller(workspace, cx) else {
        return false;
    };

    deploy_panel_in_center(panel.clone(), workspace, window, cx);
    panel.update(cx, |panel, cx| panel.open_history(window, cx));
    true
}

pub(crate) fn open_settings(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    let Some(panel) = workspace_controller(workspace, cx) else {
        return false;
    };

    deploy_panel_in_center(panel.clone(), workspace, window, cx);
    panel.update(cx, |panel, cx| panel.open_configuration(window, cx));
    true
}

pub(crate) fn open_rules_library_panel(
    workspace: &mut Workspace,
    action: &OpenRulesLibrary,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    let Some(panel) = workspace_controller(workspace, cx) else {
        return false;
    };

    deploy_panel_in_center(panel.clone(), workspace, window, cx);
    panel.update(cx, |panel, cx| panel.deploy_rules_library(action, window, cx));
    true
}

pub(crate) fn open_agent_diff(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    let Some(thread) = active_agent_thread(workspace, cx) else {
        return false;
    };

    AgentDiffPane::deploy_in_workspace(thread, workspace, window, cx);
    true
}

pub(crate) fn toggle_navigation_menu(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    let Some(panel) = workspace_controller(workspace, cx) else {
        return false;
    };

    deploy_panel_in_center(panel.clone(), workspace, window, cx);
    panel.update(cx, |panel, cx| {
        panel.toggle_navigation_menu(&ToggleNavigationMenu, window, cx);
    });
    true
}

pub(crate) fn toggle_options_menu(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    let Some(panel) = workspace_controller(workspace, cx) else {
        return false;
    };

    deploy_panel_in_center(panel.clone(), workspace, window, cx);
    panel.update(cx, |panel, cx| {
        panel.toggle_options_menu(&ToggleOptionsMenu, window, cx);
    });
    true
}

pub(crate) fn toggle_new_thread_menu(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    let Some(panel) = workspace_controller(workspace, cx) else {
        return false;
    };

    deploy_panel_in_center(panel.clone(), workspace, window, cx);
    panel.update(cx, |panel, cx| {
        panel.toggle_new_thread_menu(&ToggleNewThreadMenu, window, cx);
    });
    true
}

pub(crate) fn reset_agent_zoom(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    let Some(panel) = workspace_controller(workspace, cx) else {
        return false;
    };

    panel.update(cx, |panel, cx| panel.reset_agent_zoom(window, cx));
    true
}

pub(crate) fn copy_thread_to_clipboard(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    let Some(panel) = workspace_controller(workspace, cx) else {
        return false;
    };

    panel.update(cx, |panel, cx| panel.copy_thread_to_clipboard(window, cx));
    true
}

pub(crate) fn load_thread_from_clipboard(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    let Some(panel) = workspace_controller(workspace, cx) else {
        return false;
    };

    panel.update(cx, |panel, cx| panel.load_thread_from_clipboard(window, cx));
    true
}

pub(crate) fn set_start_thread_in(
    workspace: &mut Workspace,
    action: &StartThreadIn,
    cx: &mut Context<Workspace>,
) -> bool {
    let Some(panel) = workspace_controller(workspace, cx) else {
        return false;
    };

    panel.update(cx, |panel, cx| panel.set_start_thread_in(action, cx));
    true
}

pub(crate) fn cycle_start_thread_in(
    workspace: &mut Workspace,
    cx: &mut Context<Workspace>,
) -> bool {
    let Some(panel) = workspace_controller(workspace, cx) else {
        return false;
    };

    panel.update(cx, |panel, cx| panel.cycle_start_thread_in(cx));
    true
}

pub fn load_agent_thread_in_center(
    workspace: &mut Workspace,
    agent: Agent,
    session_id: acp::SessionId,
    work_dirs: Option<PathList>,
    title: Option<SharedString>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    let Some(panel) = workspace_controller(workspace, cx) else {
        return false;
    };

    panel.update(cx, |panel, cx| {
        panel.load_agent_thread(agent, session_id, work_dirs, title, false, window, cx);
    });

    deploy_active_panel_item_in_center(&panel, workspace, window, cx)
}

pub(crate) fn load_selected_agent_thread_in_center(
    workspace: &mut Workspace,
    session_id: acp::SessionId,
    work_dirs: Option<PathList>,
    title: Option<SharedString>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    let Some(panel) = workspace_controller(workspace, cx) else {
        return false;
    };

    let Some(agent) = panel.read(cx).selected_agent() else {
        return false;
    };

    panel.update(cx, |panel, cx| {
        panel.load_agent_thread(agent, session_id, work_dirs, title, false, window, cx);
    });

    deploy_active_panel_item_in_center(&panel, workspace, window, cx)
}

pub fn open_thread_in_center(
    workspace: &mut Workspace,
    session_id: acp::SessionId,
    work_dirs: Option<PathList>,
    title: Option<SharedString>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    load_agent_thread_in_center(
        workspace,
        Agent::NativeAgent,
        session_id,
        work_dirs,
        title,
        window,
        cx,
    )
}

pub fn open_saved_text_thread_in_center(
    workspace: &mut Workspace,
    path: Arc<Path>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Task<Result<()>> {
    let Some(panel) = workspace_controller(workspace, cx) else {
        return Task::ready(Err(anyhow!("Agent panel not available")));
    };

    deploy_panel_in_center(panel.clone(), workspace, window, cx);
    panel.update(cx, |panel, cx| {
        panel.open_saved_text_thread(path, window, cx)
    })
}

pub(crate) fn active_agent_thread(
    workspace: &Workspace,
    cx: &gpui::App,
) -> Option<Entity<AcpThread>> {
    workspace_controller(workspace, cx).and_then(|panel| panel.read(cx).active_agent_thread(cx))
}

pub fn focused_thread_id(
    workspace: &Workspace,
    cx: &gpui::App,
) -> Option<acp::SessionId> {
    workspace_controller(workspace, cx).and_then(|panel| {
        panel
            .read(cx)
            .active_conversation()
            .and_then(|conversation| conversation.read(cx).parent_id(cx))
    })
}

pub fn focused_thread_id_for_panel(
    panel: &Entity<AiWorkspace>,
    cx: &gpui::App,
) -> Option<acp::SessionId> {
    panel
        .read(cx)
        .workspace()
        .upgrade()
        .and_then(|workspace| focused_thread_id(&workspace.read(cx), cx))
}

pub fn active_workspace_focused_thread_id(
    multi_workspace: &MultiWorkspace,
    cx: &gpui::App,
) -> Option<acp::SessionId> {
    multi_workspace
        .workspaces()
        .get(multi_workspace.active_workspace_index())
        .cloned()
        .and_then(|workspace| focused_thread_id(&workspace.read(cx), cx))
}

pub fn thread_summaries(
    workspace: &Workspace,
    cx: &gpui::App,
) -> Vec<AgentThreadSummary> {
    let Some(ai_workspace) = workspace_controller(workspace, cx) else {
        return Vec::new();
    };
    let ai_workspace_ref = ai_workspace.read(cx);

    ai_workspace_ref
        .parent_threads(cx)
        .into_iter()
        .map(|thread_view| {
            let thread_view_ref = thread_view.read(cx);
            let thread = thread_view_ref.thread.read(cx);

            let session_id = thread.session_id().clone();
            let is_background = ai_workspace_ref.is_background_thread(&session_id);
            let status = if thread.is_waiting_for_confirmation() {
                AgentThreadStatus::WaitingForConfirmation
            } else if thread.had_error() {
                AgentThreadStatus::Error
            } else {
                match thread.status() {
                    acp_thread::ThreadStatus::Generating => AgentThreadStatus::Running,
                    acp_thread::ThreadStatus::Idle => AgentThreadStatus::Completed,
                }
            };

            AgentThreadSummary {
                session_id,
                title: thread.title(),
                status,
                icon: thread_view_ref.agent_icon,
                icon_from_external_svg: thread_view_ref.agent_icon_from_external_svg.clone(),
                is_background,
                is_title_generating: thread_view_ref.as_native_thread(cx).is_some()
                    && thread.has_provisional_title(),
                diff_stats: thread.action_log().read(cx).diff_stats(cx),
            }
        })
        .collect()
}

pub(crate) fn active_text_thread_editor(
    workspace: &Workspace,
    cx: &gpui::App,
) -> Option<Entity<TextThreadEditor>> {
    workspace_controller(workspace, cx).and_then(|panel| panel.read(cx).active_text_thread_editor())
}

pub(crate) fn active_native_agent_thread(
    workspace: &Workspace,
    cx: &gpui::App,
) -> Option<Entity<Thread>> {
    workspace_controller(workspace, cx).and_then(|panel| panel.read(cx).active_native_agent_thread(cx))
}

pub(crate) fn prompt_store(
    workspace: &Workspace,
    cx: &gpui::App,
) -> Option<Entity<PromptStore>> {
    workspace_controller(workspace, cx).and_then(|panel| panel.read(cx).prompt_store().clone())
}

pub(crate) fn native_agent_history(
    workspace: &Workspace,
    cx: &gpui::App,
) -> Option<Entity<ThreadHistory>> {
    workspace_controller(workspace, cx).and_then(|panel| {
        panel
            .read(cx)
            .connection_store()
            .read(cx)
            .entry(&crate::Agent::NativeAgent)
            .and_then(|entry| entry.read(cx).history().cloned())
    })
}

pub(crate) fn context_server_registry(
    workspace: &Workspace,
    cx: &gpui::App,
) -> Option<Entity<ContextServerRegistry>> {
    workspace_controller(workspace, cx)
        .map(|panel| panel.read(cx).context_server_registry().clone())
}

pub fn thread_store(
    workspace: &Workspace,
    cx: &gpui::App,
) -> Option<Entity<agent::ThreadStore>> {
    workspace_controller(workspace, cx).map(|panel| panel.read(cx).thread_store().clone())
}

pub fn agent_connection_store(
    workspace: &Workspace,
    cx: &gpui::App,
) -> Option<Entity<AgentConnectionStore>> {
    workspace_controller(workspace, cx).map(|panel| panel.read(cx).connection_store().clone())
}

pub(crate) fn start_thread_in(
    workspace: &Workspace,
    cx: &gpui::App,
) -> Option<crate::StartThreadIn> {
    workspace_controller(workspace, cx).map(|panel| panel.read(cx).start_thread_in().clone())
}

pub(crate) fn ai_surface_visible(workspace: &Workspace, cx: &gpui::App) -> bool {
    workspace.panes().iter().any(|pane| {
        let Some(active_item) = pane.read(cx).active_item() else {
            return false;
        };

        active_item.to_any_view().downcast::<AgentWorkspaceItem>().is_ok()
            || active_item.to_any_view().downcast::<TextThreadEditor>().is_ok()
    })
}
