use crate::client::{
    TaskLifecycleStatus, TaskRemoteTarget, TaskStatusResponse, WORKFLOW_GLOBALS_NODE_TYPE_ID,
    WorkflowClient, WorkflowDefinitionRecord, WorkflowEdge, WorkflowNode, WorkflowNodePort,
    WorkflowNodePrimitive, WorkflowNodeType, WorkflowNodeTypeCategory, conditional_output_ports,
    default_configuration_for_node_type, editor_node_types, infer_workflow_node_primitive,
};
use crate::inspector::{NodeInspectorPanel, upsert_workflow_def_cache};
use editor::Editor;
use gpui::{
    App, AppContext, Context, Corner, DismissEvent, FocusHandle, Focusable, PinchEvent, Pixels,
    Point, Subscription, Task, Window, WindowHandle, px,
};
use multi_buffer::MultiBuffer;
use recent_projects::open_remote_project;
use remote::{DockerConnectionOptions, RemoteConnectionOptions};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use ui::{ActiveTheme, ContextMenu};
use util::ResultExt;
use util::path_list::PathList;
use uuid::Uuid;
use workspace::{MultiWorkspace, Toast, Workspace, notifications::NotificationId};

const NODE_WIDTH_F: f32 = 300.0;
const NODE_HEIGHT_F: f32 = 156.0;
const NODE_H_GAP: f32 = 96.0;
const NODE_V_GAP: f32 = 84.0;
const EDGE_STROKE: Pixels = px(2.0);
const NODE_CORNER_RADIUS: Pixels = px(8.0);
const NODE_HEADER_X_INSET_F: f32 = 14.0;
const NODE_HEADER_Y_F: f32 = 14.0;
const NODE_KIND_Y_F: f32 = 34.0;
const NODE_PORTS_TOP_INSET_F: f32 = 46.0;
const NODE_PORTS_BOTTOM_INSET_F: f32 = 10.0;
const BORDER_WIDTH_NORMAL: Pixels = px(1.5);
const BORDER_WIDTH_SELECTED: Pixels = px(3.0);
const PORT_RADIUS_F: f32 = 7.0;
const PORT_HIT_RADIUS_F: f32 = 12.0;
const PORT_LABEL_FONT_SIZE_F: f32 = 10.0;
const PORT_LABEL_X_INSET_F: f32 = 16.0;

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct NodePos {
    pub x: f32,
    pub y: f32,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct CanvasLayout {
    pub node_positions: HashMap<String, NodePos>,
    pub viewport_offset: (f32, f32),
    pub zoom: f32,
    #[serde(skip)]
    pub edge_waypoints: HashMap<(String, String, String, String), (f32, f32)>,
}

impl Default for CanvasLayout {
    fn default() -> Self {
        Self {
            node_positions: HashMap::new(),
            viewport_offset: (0.0, 0.0),
            zoom: 1.0,
            edge_waypoints: HashMap::new(),
        }
    }
}

impl Clone for CanvasLayout {
    fn clone(&self) -> Self {
        Self {
            node_positions: self.node_positions.clone(),
            viewport_offset: self.viewport_offset,
            zoom: self.zoom,
            edge_waypoints: self.edge_waypoints.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CanvasSelection {
    None,
    Node(String),
    Edge(String, String, String, String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PortEndpoint {
    node_id: String,
    port_id: String,
}

pub enum WorkflowCanvasEvent {
    NodeSelected(Option<String>),
    NodeActivated(String),
    WorkflowSaved,
    RunFailed { task_id: Uuid, message: String },
}

enum SaveState {
    Idle,
    Saving,
    Success,
    Error(String),
}

pub fn auto_layout(nodes: &[WorkflowNode], edges: &[WorkflowEdge]) -> HashMap<String, NodePos> {
    if nodes.is_empty() {
        return HashMap::new();
    }

    // Identify back edges using node-list order as a proxy for topological order.
    // Edges that go from a later node back to an earlier node (by list index) are back edges.
    // This works reliably for workflow graphs where nodes are created in flow order.
    let node_order: HashMap<String, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.clone(), i))
        .collect();
    let back_edges: HashSet<(String, String)> = edges
        .iter()
        .filter(|e| {
            let from_ord = node_order.get(&e.from_node_id).copied().unwrap_or(0);
            let to_ord = node_order
                .get(&e.to_node_id)
                .copied()
                .unwrap_or(usize::MAX);
            from_ord > to_ord
        })
        .map(|e| (e.from_node_id.clone(), e.to_node_id.clone()))
        .collect();

    // Build forward-only adjacency (skip back edges to break cycles for layering)
    let mut forward_adj: HashMap<String, Vec<String>> = nodes.iter().map(|n| (n.id.clone(), vec![])).collect();
    let mut in_degree: HashMap<String, usize> = nodes.iter().map(|n| (n.id.clone(), 0)).collect();
    for edge in edges {
        if !back_edges.contains(&(edge.from_node_id.clone(), edge.to_node_id.clone())) {
            forward_adj.entry(edge.from_node_id.clone()).or_default().push(edge.to_node_id.clone());
            *in_degree.entry(edge.to_node_id.clone()).or_insert(0) += 1;
        }
    }

    // Longest-path layering via BFS from sources
    let queue: std::collections::VecDeque<String> = in_degree
        .iter()
        .filter_map(|(id, &deg)| if deg == 0 { Some(id.clone()) } else { None })
        .collect();
    // Sort for determinism
    let mut queue_vec: Vec<String> = queue.into_iter().collect();
    queue_vec.sort();
    let mut queue: std::collections::VecDeque<String> = queue_vec.into();

    let mut depth: HashMap<String, usize> = HashMap::new();
    while let Some(node) = queue.pop_front() {
        let d = depth.get(&node).copied().unwrap_or(0);
        for next in forward_adj.get(&node).unwrap_or(&vec![]) {
            let next_depth = d + 1;
            let entry = depth.entry(next.clone()).or_insert(0);
            if next_depth > *entry {
                *entry = next_depth;
            }
            queue.push_back(next.clone());
        }
    }

    // Group nodes by column (layer)
    let max_col = depth.values().copied().max().unwrap_or(0);
    let mut columns: Vec<Vec<String>> = vec![vec![]; max_col + 1];
    for node in nodes {
        let col = depth.get(&node.id).copied().unwrap_or(0);
        columns[col].push(node.id.clone());
    }

    // Sort each column by node id for initial determinism
    for col_nodes in &mut columns {
        col_nodes.sort();
    }

    // Build predecessor adjacency for barycenter pass
    let mut pred_adj: HashMap<String, Vec<String>> = nodes.iter().map(|n| (n.id.clone(), vec![])).collect();
    for edge in edges {
        if !back_edges.contains(&(edge.from_node_id.clone(), edge.to_node_id.clone())) {
            pred_adj.entry(edge.to_node_id.clone()).or_default().push(edge.from_node_id.clone());
        }
    }

    // Assign initial row positions for barycenter computation
    let mut row_pos: HashMap<String, f32> = HashMap::new();
    for col_nodes in &columns {
        for (row, node_id) in col_nodes.iter().enumerate() {
            row_pos.insert(node_id.clone(), row as f32);
        }
    }

    // Left-to-right barycenter pass to minimize crossings
    for col_idx in 1..=max_col {
        let col_nodes = columns[col_idx].clone();
        let empty_preds: Vec<String> = vec![];
        let mut barycenters: Vec<(String, f32)> = col_nodes.iter().map(|node_id| {
            let preds = pred_adj.get(node_id.as_str()).unwrap_or(&empty_preds);
            let bc = if preds.is_empty() {
                row_pos.get(node_id.as_str()).copied().unwrap_or(0.0)
            } else {
                preds.iter().map(|p| row_pos.get(p.as_str()).copied().unwrap_or(0.0)).sum::<f32>() / preds.len() as f32
            };
            (node_id.clone(), bc)
        }).collect();
        barycenters.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(&b.0)));
        columns[col_idx] = barycenters.into_iter().map(|(id, _)| id).collect();
        for (row, node_id) in columns[col_idx].iter().enumerate() {
            row_pos.insert(node_id.clone(), row as f32);
        }
    }

    // Compute final positions, centering each column vertically relative to the tallest column
    let max_col_count = columns.iter().map(|c| c.len()).max().unwrap_or(1);
    let max_h = max_col_count as f32 * (NODE_HEIGHT_F + NODE_V_GAP) - NODE_V_GAP;

    let mut positions = HashMap::new();
    for (col_idx, col_nodes) in columns.iter().enumerate() {
        let count = col_nodes.len();
        let col_h = count as f32 * (NODE_HEIGHT_F + NODE_V_GAP) - NODE_V_GAP;
        let start_y = 40.0 + (max_h - col_h) * 0.5;
        for (row_idx, node_id) in col_nodes.iter().enumerate() {
            positions.insert(node_id.clone(), NodePos {
                x: col_idx as f32 * (NODE_WIDTH_F + NODE_H_GAP) + 40.0,
                y: start_y + row_idx as f32 * (NODE_HEIGHT_F + NODE_V_GAP),
            });
        }
    }
    positions
}

pub fn to_screen_point(
    layout: &CanvasLayout,
    canvas_x: f32,
    canvas_y: f32,
    origin: Point<Pixels>,
) -> Point<Pixels> {
    let (ox, oy) = layout.viewport_offset;
    let z = layout.zoom;
    gpui::point(
        origin.x + px((canvas_x + ox) * z),
        origin.y + px((canvas_y + oy) * z),
    )
}

pub fn scaled(layout: &CanvasLayout, canvas_val: f32) -> Pixels {
    px(canvas_val * layout.zoom)
}

pub fn to_canvas_point(
    layout: &CanvasLayout,
    screen_x: Pixels,
    screen_y: Pixels,
    origin: Point<Pixels>,
) -> (f32, f32) {
    let (ox, oy) = layout.viewport_offset;
    let z = layout.zoom;
    (
        (screen_x - origin.x).as_f32() / z - ox,
        (screen_y - origin.y).as_f32() / z - oy,
    )
}

/// Returns (min_y, max_y) in canvas coordinates covering all node rects,
/// expanded by `padding` on each side.
fn nodes_bounding_box(
    node_positions: &HashMap<String, NodePos>,
    padding: f32,
) -> (f32, f32) {
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;
    for pos in node_positions.values() {
        min_y = min_y.min(pos.y);
        max_y = max_y.max(pos.y + NODE_HEIGHT_F);
    }
    if min_y == f32::MAX {
        return (0.0, NODE_HEIGHT_F);
    }
    (min_y - padding, max_y + padding)
}

/// Compute the rail Y for a backward edge on the given side.
/// `above=true` → above top rail, `above=false` → below bottom rail.
/// `rail_index=0` is the innermost rail.
fn backward_edge_rail_y(
    bbox_top: f32,
    bbox_bottom: f32,
    rail_index: usize,
    above: bool,
) -> f32 {
    let offset = rail_index as f32 * 12.0;
    if above {
        bbox_top - offset
    } else {
        bbox_bottom + offset
    }
}

/// Given an ordered slice of `(from_port, to_port)` canvas-coordinate pairs,
/// returns per-edge `(above: bool, rail_index: usize)` assignments in the same order.
/// Edges whose midpoint is closer to the top bounding box edge route above; others route below.
/// Within each group, wider-span edges get lower rail indices (innermost rails).
fn compute_backward_edge_rails(
    port_pairs: &[((f32, f32), (f32, f32))],
    bbox_top: f32,
    bbox_bottom: f32,
) -> Vec<(bool, usize)> {
    let annotated: Vec<(usize, bool, f32)> = port_pairs
        .iter()
        .enumerate()
        .map(|(index, (from, to))| {
            let mid_y = (from.1 + to.1) / 2.0;
            let above = (mid_y - bbox_top) <= (bbox_bottom - mid_y);
            let span = (from.0 - to.0).abs();
            (index, above, span)
        })
        .collect();

    let mut above_group: Vec<_> = annotated.iter().filter(|(_, above, _)| *above).collect();
    let mut below_group: Vec<_> = annotated.iter().filter(|(_, above, _)| !above).collect();
    above_group.sort_by(|a, b| b.2.total_cmp(&a.2));
    below_group.sort_by(|a, b| b.2.total_cmp(&a.2));

    let mut result = vec![(true, 0usize); port_pairs.len()];
    for (rail_index, (original_index, above, _)) in above_group.iter().enumerate() {
        result[*original_index] = (*above, rail_index);
    }
    for (rail_index, (original_index, above, _)) in below_group.iter().enumerate() {
        result[*original_index] = (*above, rail_index);
    }
    result
}

pub struct WorkflowCanvas {
    pub workflow: Option<WorkflowDefinitionRecord>,
    pub run: Option<TaskStatusResponse>,
    node_types: Vec<WorkflowNodeType>,
    pub layout: CanvasLayout,
    pub selection: CanvasSelection,
    pending_connection: Option<PortEndpoint>,
    pending_connection_target: Option<(f32, f32)>,
    drag_node: Option<String>,
    drag_node_start_pos: Option<NodePos>,
    drag_mouse_start: Option<Point<Pixels>>,
    drag_edge_waypoint_key: Option<(String, String, String, String)>,
    pan_mouse_start: Option<Point<Pixels>>,
    pan_viewport_start: Option<(f32, f32)>,
    background_click_start: Option<Point<Pixels>>,
    context_menu: Option<(gpui::Entity<ContextMenu>, Point<Pixels>, Subscription)>,
    save_state: SaveState,
    animation_phase: f32,
    focus_handle: FocusHandle,
    pub on_node_selected: Option<Box<dyn Fn(Option<String>, &mut Window, &mut App)>>,
    _poll_task: Option<Task<()>>,
    _node_types_task: Option<Task<()>>,
    _save_task: Option<Task<()>>,
    client: Arc<WorkflowClient>,
    canvas_bounds: Option<gpui::Bounds<Pixels>>,
}

impl gpui::EventEmitter<WorkflowCanvasEvent> for WorkflowCanvas {}

impl WorkflowCanvas {
    pub fn new_edit(
        workflow: WorkflowDefinitionRecord,
        client: Arc<WorkflowClient>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut layout = CanvasLayout::default();
        if layout.node_positions.is_empty() {
            layout.node_positions = auto_layout(&workflow.nodes, &workflow.edges);
        }
        let mut canvas = Self {
            workflow: Some(workflow),
            run: None,
            node_types: Vec::new(),
            layout,
            selection: CanvasSelection::None,
            pending_connection: None,
            pending_connection_target: None,
            drag_node: None,
            drag_node_start_pos: None,
            drag_mouse_start: None,
            drag_edge_waypoint_key: None,
            pan_mouse_start: None,
            pan_viewport_start: None,
            background_click_start: None,
            context_menu: None,
            save_state: SaveState::Idle,
            animation_phase: 0.0,
            focus_handle: cx.focus_handle(),
            on_node_selected: None,
            _poll_task: None,
            _node_types_task: None,
            _save_task: None,
            client,
            canvas_bounds: None,
        };
        canvas.start_loading_node_types(cx);
        canvas
    }

    #[cfg(test)]
    pub(crate) fn new_edit_for_test(
        workflow: WorkflowDefinitionRecord,
        client: Arc<WorkflowClient>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut layout = CanvasLayout::default();
        if layout.node_positions.is_empty() {
            layout.node_positions = auto_layout(&workflow.nodes, &workflow.edges);
        }
        Self {
            workflow: Some(workflow),
            run: None,
            node_types: Vec::new(),
            layout,
            selection: CanvasSelection::None,
            pending_connection: None,
            pending_connection_target: None,
            drag_node: None,
            drag_node_start_pos: None,
            drag_mouse_start: None,
            drag_edge_waypoint_key: None,
            pan_mouse_start: None,
            pan_viewport_start: None,
            background_click_start: None,
            context_menu: None,
            save_state: SaveState::Idle,
            animation_phase: 0.0,
            focus_handle: cx.focus_handle(),
            on_node_selected: None,
            _poll_task: None,
            _node_types_task: None,
            _save_task: None,
            client,
            canvas_bounds: None,
        }
    }

    pub fn new_run(
        run: TaskStatusResponse,
        client: Arc<WorkflowClient>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut layout = CanvasLayout::default();
        if let Some(ref workflow) = run.workflow {
            layout.node_positions = auto_layout(&workflow.nodes, &workflow.edges);
        } else {
            let nodes: Vec<WorkflowNode> = run
                .nodes
                .iter()
                .map(|n| WorkflowNode {
                    id: n.id.clone(),
                    node_type: n.node_type.clone(),
                    label: n.label.clone(),
                    configuration: serde_json::json!({}),
                    runtime: serde_json::json!({}),
                })
                .collect();
            layout.node_positions = auto_layout(&nodes, &[]);
        }
        let task_id = run.task.id;
        let mut canvas = Self {
            workflow: run.workflow.clone(),
            run: Some(run),
            node_types: Vec::new(),
            layout,
            selection: CanvasSelection::None,
            pending_connection: None,
            pending_connection_target: None,
            drag_node: None,
            drag_node_start_pos: None,
            drag_mouse_start: None,
            drag_edge_waypoint_key: None,
            pan_mouse_start: None,
            pan_viewport_start: None,
            background_click_start: None,
            context_menu: None,
            save_state: SaveState::Idle,
            animation_phase: 0.0,
            focus_handle: cx.focus_handle(),
            on_node_selected: None,
            _poll_task: None,
            _node_types_task: None,
            _save_task: None,
            client,
            canvas_bounds: None,
        };
        canvas.start_polling(task_id, cx);
        canvas.start_loading_node_types(cx);
        canvas
    }

    fn has_running_nodes(&self) -> bool {
        self.run.as_ref().map_or(false, |r| {
            r.nodes
                .iter()
                .any(|n| n.status == TaskLifecycleStatus::Running)
                || r.task.status == TaskLifecycleStatus::Running
        })
    }

    fn is_editable(&self) -> bool {
        self.run.is_none()
    }

    fn start_loading_node_types(&mut self, cx: &mut Context<Self>) {
        let client = self.client.clone();
        self._node_types_task = Some(cx.spawn(async move |this, cx| {
            let Ok(node_types) = client.list_workflow_node_types().await else {
                return;
            };
            this.update(cx, |canvas, cx| {
                canvas.node_types = editor_node_types(node_types);
                cx.notify();
            })
            .ok();
        }));
    }

    fn globals_node_id(&self) -> Option<String> {
        let wf = self.workflow.as_ref()?;
        wf.nodes
            .iter()
            .find(|n| n.node_type == WORKFLOW_GLOBALS_NODE_TYPE_ID)
            .map(|n| n.id.clone())
    }

    fn hit_test_node(
        &self,
        screen_pt: Point<Pixels>,
        canvas_origin: Point<Pixels>,
    ) -> Option<String> {
        let (cx_coord, cy_coord) =
            to_canvas_point(&self.layout, screen_pt.x, screen_pt.y, canvas_origin);
        let globals_id = self.globals_node_id();
        for (id, pos) in &self.layout.node_positions {
            if globals_id.as_deref() == Some(id.as_str()) {
                continue;
            }
            if cx_coord >= pos.x
                && cx_coord <= pos.x + NODE_WIDTH_F
                && cy_coord >= pos.y
                && cy_coord <= pos.y + NODE_HEIGHT_F
            {
                return Some(id.clone());
            }
        }
        None
    }

    fn input_ports_for_node(&self, node: &WorkflowNode) -> Vec<WorkflowNodePort> {
        effective_ports_for_node(&self.node_types, node, true)
    }

    fn output_ports_for_node(&self, node: &WorkflowNode) -> Vec<WorkflowNodePort> {
        effective_ports_for_node(&self.node_types, node, false)
    }

    fn hit_test_port(
        &self,
        screen_pt: Point<Pixels>,
        canvas_origin: Point<Pixels>,
        input_side: bool,
    ) -> Option<PortEndpoint> {
        let workflow = self.workflow.as_ref()?;
        for node in &workflow.nodes {
            if node.node_type == WORKFLOW_GLOBALS_NODE_TYPE_ID {
                continue;
            }
            let Some(position) = self.layout.node_positions.get(&node.id) else {
                continue;
            };
            let ports = if input_side {
                self.input_ports_for_node(node)
            } else {
                self.output_ports_for_node(node)
            };
            for (index, port) in ports.iter().enumerate() {
                let port_position = port_canvas_position(position, input_side, index, ports.len());
                let port_screen = to_screen_point(
                    &self.layout,
                    port_position.0,
                    port_position.1,
                    canvas_origin,
                );
                let dx = (screen_pt.x - port_screen.x).as_f32();
                let dy = (screen_pt.y - port_screen.y).as_f32();
                if dx * dx + dy * dy <= (PORT_HIT_RADIUS_F * self.layout.zoom).powi(2) {
                    return Some(PortEndpoint {
                        node_id: node.id.clone(),
                        port_id: port.id.clone(),
                    });
                }
            }
        }
        None
    }

    fn backward_edge_rail_assignments(
        &self,
        bbox_top: f32,
        bbox_bottom: f32,
    ) -> HashMap<(String, String, String, String), (bool, usize)> {
        let Some(wf) = self.workflow.as_ref() else {
            return HashMap::new();
        };

        let backward_edges: Vec<_> = wf
            .edges
            .iter()
            .filter_map(|edge| {
                let from = port_position_for_node(
                    &self.layout,
                    wf,
                    &self.node_types,
                    &edge.from_node_id,
                    &edge.from_output_id,
                    false,
                )?;
                let to = port_position_for_node(
                    &self.layout,
                    wf,
                    &self.node_types,
                    &edge.to_node_id,
                    &edge.to_input_id,
                    true,
                )?;
                if !is_backward_edge(from, to) {
                    return None;
                }
                Some((edge, from, to))
            })
            .collect();

        let port_pairs: Vec<_> = backward_edges.iter().map(|(_, from, to)| (*from, *to)).collect();
        let assignments = compute_backward_edge_rails(&port_pairs, bbox_top, bbox_bottom);

        let mut result = HashMap::new();
        for ((edge, _, _), (above, rail_index)) in backward_edges.iter().zip(assignments) {
            let key = (
                edge.from_node_id.clone(),
                edge.from_output_id.clone(),
                edge.to_node_id.clone(),
                edge.to_input_id.clone(),
            );
            result.insert(key, (above, rail_index));
        }
        result
    }

    fn hit_test_edge(
        &self,
        screen_pt: Point<Pixels>,
        canvas_origin: Point<Pixels>,
    ) -> Option<WorkflowEdge> {
        let workflow = self.workflow.as_ref()?;
        let (bbox_top, bbox_bottom) = nodes_bounding_box(&self.layout.node_positions, 40.0);
        let rail_map = self.backward_edge_rail_assignments(bbox_top, bbox_bottom);
        for edge in &workflow.edges {
            let (Some(from_port), Some(to_port)) = (
                port_position_for_node(
                    &self.layout,
                    workflow,
                    &self.node_types,
                    &edge.from_node_id,
                    &edge.from_output_id,
                    false,
                ),
                port_position_for_node(
                    &self.layout,
                    workflow,
                    &self.node_types,
                    &edge.to_node_id,
                    &edge.to_input_id,
                    true,
                ),
            ) else {
                continue;
            };

            if is_backward_edge(from_port, to_port) {
                let key = (
                    edge.from_node_id.clone(),
                    edge.from_output_id.clone(),
                    edge.to_node_id.clone(),
                    edge.to_input_id.clone(),
                );
                let (above, rail_index) = rail_map.get(&key).copied().unwrap_or((true, 0));
                let rail_y = backward_edge_rail_y(bbox_top, bbox_bottom, rail_index, above);
                if backward_arc_contains_screen_point(
                    &self.layout,
                    from_port,
                    to_port,
                    rail_y,
                    screen_pt,
                    canvas_origin,
                ) {
                    return Some(edge.clone());
                }
                continue;
            }

            if edge_contains_screen_point(
                &self.layout,
                from_port,
                to_port,
                canvas_origin,
                screen_pt,
            ) {
                return Some(edge.clone());
            }
        }
        None
    }

    fn hit_test_edge_handle(
        &self,
        screen_pt: Point<Pixels>,
        canvas_origin: Point<Pixels>,
    ) -> Option<(String, String, String, String)> {
        if !self.is_editable() {
            return None;
        }
        let CanvasSelection::Edge(ref fn_id, ref fo_id, ref tn_id, ref ti_id) = self.selection
        else {
            return None;
        };
        let wf = self.workflow.as_ref()?;
        let edge_key = (fn_id.clone(), fo_id.clone(), tn_id.clone(), ti_id.clone());
        let from = port_position_for_node(&self.layout, wf, &self.node_types, fn_id, fo_id, false)?;
        let to = port_position_for_node(&self.layout, wf, &self.node_types, tn_id, ti_id, true)?;

        if is_backward_edge(from, to) {
            return None;
        }

        let handle_canvas = self
            .layout
            .edge_waypoints
            .get(&edge_key)
            .copied()
            .unwrap_or_else(|| edge_canvas_midpoint(from, to));

        let handle_screen =
            to_screen_point(&self.layout, handle_canvas.0, handle_canvas.1, canvas_origin);
        let dx = (screen_pt.x - handle_screen.x).as_f32();
        let dy = (screen_pt.y - handle_screen.y).as_f32();
        if dx * dx + dy * dy <= (8.0 * self.layout.zoom).powi(2) {
            Some(edge_key)
        } else {
            None
        }
    }

    fn start_polling(&mut self, task_id: Uuid, cx: &mut Context<Self>) {
        let client = self.client.clone();
        self._poll_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(2))
                    .await;
                let Ok(status) = client.get_task_status(task_id).await else {
                    continue;
                };
                let failure_message = run_failure_message(&status);
                let is_terminal = status.task.status.is_terminal();
                this.update(cx, |canvas, cx| {
                    let already_failed = canvas
                        .run
                        .as_ref()
                        .is_some_and(|run| run.task.status == TaskLifecycleStatus::Failed);
                    canvas.run = Some(status);
                    if !already_failed && let Some(message) = failure_message {
                        cx.emit(WorkflowCanvasEvent::RunFailed { task_id, message });
                    }
                    cx.notify();
                })
                .ok();
                if is_terminal {
                    break;
                }
            }
        }));
    }

    fn canvas_origin(&self) -> Point<Pixels> {
        self.canvas_bounds
            .map(|b| b.origin)
            .unwrap_or(gpui::point(px(0.0), px(0.0)))
    }

    fn contains_canvas_point(&self, position: Point<Pixels>) -> bool {
        self.canvas_bounds
            .map(|bounds| bounds.contains(&position))
            .unwrap_or(false)
    }

    fn handle_mouse_down(
        &mut self,
        event: &gpui::MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.context_menu.take();
        let origin = self.canvas_origin();
        let position = event.position;

        if !self.contains_canvas_point(position) {
            return;
        }

        if self.is_editable()
            && let Some(port) = self.hit_test_port(position, origin, false)
        {
            self.select_node(port.node_id.clone(), window, cx);
            self.pending_connection = Some(port);
            self.pending_connection_target = Some(to_canvas_point(
                &self.layout,
                position.x,
                position.y,
                origin,
            ));
            cx.notify();
            return;
        }

        if let Some(edge_key) = self.hit_test_edge_handle(position, origin) {
            if event.click_count == 2 {
                self.layout.edge_waypoints.remove(&edge_key);
                cx.notify();
            } else {
                self.drag_edge_waypoint_key = Some(edge_key);
                cx.notify();
            }
            return;
        }

        if let Some(node_id) = self.hit_test_node(position, origin) {
            if event.click_count == 2 {
                cx.emit(WorkflowCanvasEvent::NodeActivated(node_id));
            } else {
                if self.is_editable() {
                    self.drag_node = Some(node_id.clone());
                    self.drag_mouse_start = Some(position);
                    self.drag_node_start_pos = self.layout.node_positions.get(&node_id).copied();
                }
                self.select_node(node_id, window, cx);
            }
        } else {
            self.clear_selection(window, cx);
            self.pending_connection = None;
            self.pending_connection_target = None;
            self.pan_mouse_start = Some(position);
            self.pan_viewport_start = Some(self.layout.viewport_offset);
            self.background_click_start = Some(position);
        }

        cx.notify();
    }

    fn handle_secondary_mouse_down(
        &mut self,
        event: &gpui::MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.context_menu.take();

        if self.run.is_some() || !self.contains_canvas_point(event.position) {
            return;
        }

        let origin = self.canvas_origin();
        let Some(node_id) = self.hit_test_node(event.position, origin) else {
            if let Some(edge) = self.hit_test_edge(event.position, origin) {
                cx.stop_propagation();
                window.prevent_default();
                self.pending_connection = None;
                self.pending_connection_target = None;
                self.drag_node = None;
                self.drag_node_start_pos = None;
                self.drag_mouse_start = None;
                self.pan_mouse_start = None;
                self.pan_viewport_start = None;
                self.select_edge(&edge, window, cx);
                self.deploy_edge_context_menu(edge, event.position, window, cx);
                cx.notify();
            }
            return;
        };

        cx.stop_propagation();
        window.prevent_default();
        self.pending_connection = None;
        self.pending_connection_target = None;
        self.drag_node = None;
        self.drag_node_start_pos = None;
        self.drag_mouse_start = None;
        self.pan_mouse_start = None;
        self.pan_viewport_start = None;
        self.select_node(node_id.clone(), window, cx);
        self.deploy_context_menu(node_id, event.position, window, cx);
        cx.notify();
    }

    fn handle_mouse_move(
        &mut self,
        event: &gpui::MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let position = event.position;

        if let (Some(pan_start), Some(pan_offset_start)) =
            (self.pan_mouse_start, self.pan_viewport_start)
        {
            let z = self.layout.zoom;
            let dx = (position.x - pan_start.x).as_f32() / z;
            let dy = (position.y - pan_start.y).as_f32() / z;
            self.layout.viewport_offset = (pan_offset_start.0 + dx, pan_offset_start.1 + dy);
            cx.notify();
            return;
        }

        if self.is_editable() && self.pending_connection.is_some() {
            let origin = self.canvas_origin();
            self.pending_connection_target = Some(to_canvas_point(
                &self.layout,
                position.x,
                position.y,
                origin,
            ));
            cx.notify();
            return;
        }

        if let (Some(ref node_id), Some(drag_start), Some(start_pos)) = (
            self.drag_node.clone(),
            self.drag_mouse_start,
            self.drag_node_start_pos,
        ) {
            let z = self.layout.zoom;
            let dx = (position.x - drag_start.x).as_f32() / z;
            let dy = (position.y - drag_start.y).as_f32() / z;
            self.layout.node_positions.insert(
                node_id.clone(),
                NodePos {
                    x: start_pos.x + dx,
                    y: start_pos.y + dy,
                },
            );
            cx.notify();
        }

        if let Some(ref key) = self.drag_edge_waypoint_key.clone() {
            let origin = self.canvas_origin();
            let canvas_pos =
                to_canvas_point(&self.layout, position.x, position.y, origin);
            self.layout.edge_waypoints.insert(key.clone(), canvas_pos);
            cx.notify();
        }
    }

    fn handle_mouse_up(
        &mut self,
        event: &gpui::MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_editable()
            && let Some(source) = self.pending_connection.take()
        {
            let origin = self.canvas_origin();
            if let Some(target) = self.hit_test_port(event.position, origin, true) {
                if source.node_id != target.node_id {
                    if let Some(workflow) = self.workflow.as_mut() {
                        let edge_exists = workflow.edges.iter().any(|edge| {
                            edge.from_node_id == source.node_id
                                && edge.from_output_id == source.port_id
                                && edge.to_node_id == target.node_id
                                && edge.to_input_id == target.port_id
                        });
                        if !edge_exists {
                            workflow.edges.push(WorkflowEdge {
                                from_node_id: source.node_id,
                                from_output_id: source.port_id,
                                to_node_id: target.node_id,
                                to_input_id: target.port_id,
                            });
                        }
                    }
                }
            }
            self.pending_connection_target = None;
            cx.notify();
        }

        if !self.is_editable() {
            self.pending_connection = None;
            self.pending_connection_target = None;
        }

        self.drag_node = None;
        self.drag_mouse_start = None;
        self.drag_node_start_pos = None;
        self.drag_edge_waypoint_key = None;
        self.pan_mouse_start = None;
        self.pan_viewport_start = None;

        if let Some(click_start) = self.background_click_start.take() {
            let dx = (event.position.x - click_start.x).as_f32();
            let dy = (event.position.y - click_start.y).as_f32();
            let dist_sq = dx * dx + dy * dy;
            if dist_sq < 4.0 * 4.0 {
                cx.emit(WorkflowCanvasEvent::NodeSelected(self.globals_node_id()));
            }
        }
    }

    fn handle_scroll_wheel(
        &mut self,
        event: &gpui::ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let origin = self.canvas_origin();
        let delta = event.delta.pixel_delta(px(20.0));
        if event.modifiers.platform {
            let zoom_delta = 1.0 + delta.y.as_f32() * 0.01;
            apply_canvas_zoom_at_position(&mut self.layout, origin, event.position, zoom_delta);
        } else {
            let z = self.layout.zoom;
            self.layout.viewport_offset.0 += delta.x.as_f32() / z;
            self.layout.viewport_offset.1 += delta.y.as_f32() / z;
        }
        cx.notify();
    }

    fn handle_pinch(&mut self, event: &PinchEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let origin = self.canvas_origin();
        let zoom_factor = 1.0 + event.delta;
        apply_canvas_zoom_at_position(&mut self.layout, origin, event.position, zoom_factor);
        cx.notify();
    }

    fn handle_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.keystroke.key.as_str() {
            "backspace" | "delete" => {
                if self.is_editable() {
                    self.delete_selected_node(cx);
                }
            }
            "escape" => {
                self.selection = CanvasSelection::None;
                self.pending_connection = None;
                self.pending_connection_target = None;
                self.context_menu.take();
                cx.emit(WorkflowCanvasEvent::NodeSelected(None));
                cx.notify();
            }
            _ => {}
        }
    }

    fn add_node(&mut self, node_type: &WorkflowNodeType, cx: &mut Context<Self>) {
        if !self.is_editable() {
            return;
        }

        let Some(ref mut workflow) = self.workflow else {
            return;
        };

        if node_type.id == WORKFLOW_GLOBALS_NODE_TYPE_ID
            && let Some(existing_node) = workflow
                .nodes
                .iter()
                .find(|node| node.node_type == WORKFLOW_GLOBALS_NODE_TYPE_ID)
        {
            self.selection = CanvasSelection::Node(existing_node.id.clone());
            cx.emit(WorkflowCanvasEvent::NodeSelected(Some(
                existing_node.id.clone(),
            )));
            cx.notify();
            return;
        }

        let id = uuid::Uuid::new_v4().to_string();
        let (ox, oy) = self.layout.viewport_offset;
        let canvas_x = -ox + 300.0 / self.layout.zoom;
        let canvas_y = -oy + 200.0 / self.layout.zoom;
        let label = node_type.label.clone();
        workflow.nodes.push(crate::client::WorkflowNode {
            id: id.clone(),
            node_type: node_type.id.clone(),
            label,
            configuration: default_configuration_for_node_type(node_type),
            runtime: serde_json::json!({}),
        });
        self.layout.node_positions.insert(
            id.clone(),
            NodePos {
                x: canvas_x,
                y: canvas_y,
            },
        );
        self.selection = CanvasSelection::Node(id.clone());
        cx.emit(WorkflowCanvasEvent::NodeSelected(Some(id)));
        cx.notify();
    }

    fn delete_selected_node(&mut self, cx: &mut Context<Self>) {
        if !self.is_editable() {
            return;
        }

        match self.selection.clone() {
            CanvasSelection::Node(node_id) => self.delete_node(&node_id, cx),
            CanvasSelection::Edge(from_node_id, from_output_id, to_node_id, to_input_id) => {
                self.delete_edge(
                    &from_node_id,
                    &from_output_id,
                    &to_node_id,
                    &to_input_id,
                    cx,
                );
            }
            CanvasSelection::None => {}
        }
    }

    fn delete_node(&mut self, node_id: &str, cx: &mut Context<Self>) {
        if !self.is_editable() {
            return;
        }

        let Some(workflow) = self.workflow.as_mut() else {
            return;
        };

        workflow.nodes.retain(|node| node.id != node_id);
        workflow
            .edges
            .retain(|edge| edge.from_node_id != node_id && edge.to_node_id != node_id);
        self.layout.node_positions.remove(node_id);
        self.selection = CanvasSelection::None;
        self.pending_connection = None;
        self.pending_connection_target = None;
        self.context_menu.take();
        cx.emit(WorkflowCanvasEvent::NodeSelected(None));
        cx.notify();
    }

    fn delete_edge(
        &mut self,
        from_node_id: &str,
        from_output_id: &str,
        to_node_id: &str,
        to_input_id: &str,
        cx: &mut Context<Self>,
    ) {
        if !self.is_editable() {
            return;
        }

        let Some(workflow) = self.workflow.as_mut() else {
            return;
        };

        workflow.edges.retain(|edge| {
            !(edge.from_node_id == from_node_id
                && edge.from_output_id == from_output_id
                && edge.to_node_id == to_node_id
                && edge.to_input_id == to_input_id)
        });
        self.selection = CanvasSelection::None;
        self.context_menu.take();
        cx.emit(WorkflowCanvasEvent::NodeSelected(None));
        cx.notify();
    }

    fn select_node(&mut self, node_id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.selection = CanvasSelection::Node(node_id.clone());
        if let Some(ref callback) = self.on_node_selected {
            callback(Some(node_id.clone()), window, cx);
        }
        cx.emit(WorkflowCanvasEvent::NodeSelected(Some(node_id)));
    }

    fn clear_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.selection = CanvasSelection::None;
        if let Some(ref callback) = self.on_node_selected {
            callback(None, window, cx);
        }
        cx.emit(WorkflowCanvasEvent::NodeSelected(None));
    }

    fn select_edge(&mut self, edge: &WorkflowEdge, window: &mut Window, cx: &mut Context<Self>) {
        self.selection = CanvasSelection::Edge(
            edge.from_node_id.clone(),
            edge.from_output_id.clone(),
            edge.to_node_id.clone(),
            edge.to_input_id.clone(),
        );
        if let Some(ref callback) = self.on_node_selected {
            callback(None, window, cx);
        }
        cx.emit(WorkflowCanvasEvent::NodeSelected(None));
    }

    fn deploy_context_menu(
        &mut self,
        node_id: String,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let this = cx.weak_entity();
        let context_menu = ContextMenu::build(window, cx, move |menu, _window, _cx| {
            let node_id = node_id.clone();
            menu.entry("Delete node", None, move |window, cx| {
                let node_id = node_id.clone();
                this.update(cx, |canvas, cx| {
                    canvas.delete_node(&node_id, cx);
                    window.focus(&canvas.focus_handle, cx);
                })
                .ok();
            })
        });

        window.focus(&context_menu.focus_handle(cx), cx);
        let subscription = cx.subscribe(&context_menu, |this, _, _: &DismissEvent, cx| {
            this.context_menu.take();
            cx.notify();
        });
        self.context_menu = Some((context_menu, position, subscription));
    }

    fn deploy_edge_context_menu(
        &mut self,
        edge: WorkflowEdge,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let this = cx.weak_entity();
        let context_menu = ContextMenu::build(window, cx, move |menu, _window, _cx| {
            let edge = edge.clone();
            menu.entry("Delete edge", None, move |window, cx| {
                let edge = edge.clone();
                this.update(cx, |canvas, cx| {
                    canvas.delete_edge(
                        &edge.from_node_id,
                        &edge.from_output_id,
                        &edge.to_node_id,
                        &edge.to_input_id,
                        cx,
                    );
                    window.focus(&canvas.focus_handle, cx);
                })
                .ok();
            })
        });

        window.focus(&context_menu.focus_handle(cx), cx);
        let subscription = cx.subscribe(&context_menu, |this, _, _: &DismissEvent, cx| {
            this.context_menu.take();
            cx.notify();
        });
        self.context_menu = Some((context_menu, position, subscription));
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        use ui::{Tooltip, prelude::*};
        let save_label: SharedString = match &self.save_state {
            SaveState::Idle => "Save".into(),
            SaveState::Saving => "Saving...".into(),
            SaveState::Success => "Saved!".into(),
            SaveState::Error(_) => "Error".into(),
        };
        let save_color = match &self.save_state {
            SaveState::Success => Color::Success,
            SaveState::Error(_) => Color::Error,
            _ => Color::Default,
        };
        h_flex()
            .justify_between()
            .items_center()
            .gap_2()
            .p_2()
            .child(
                h_flex()
                    .gap_1()
                    .children(self.node_types.iter().map(|node_type| {
                        let node_type = node_type.clone();
                        let button_id = format!("add-node-type-{}", node_type.id);
                        let button_label = format!("+ {}", node_type.label);
                        let tooltip_label = format!("Add {} node", node_type.label);
                        Button::new(button_id, button_label)
                            .style(ButtonStyle::Subtle)
                            .tooltip(Tooltip::text(tooltip_label))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.add_node(&node_type, cx);
                            }))
                    })),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .when_some(
                        match &self.save_state {
                            SaveState::Error(message) => Some(message.clone()),
                            _ => None,
                        },
                        |this, message| {
                            this.child(
                                Label::new(message)
                                    .color(Color::Error)
                                    .size(LabelSize::Small),
                            )
                        },
                    )
                    .child(
                        Button::new("save-workflow-canvas", save_label)
                            .color(save_color)
                            .on_click(cx.listener(|this, _, _window, cx| {
                                this.save_workflow(cx);
                            })),
                    ),
            )
    }

    fn save_workflow(&mut self, cx: &mut Context<Self>) {
        let Some(workflow) = self.workflow.clone() else {
            return;
        };

        let request = workflow.to_request();
        let workflow_id = workflow.id;
        let is_new = workflow_id.is_nil();
        let client = self.client.clone();
        self.save_state = SaveState::Saving;
        cx.notify();

        self._save_task = Some(cx.spawn(async move |this, cx| {
            let result = if is_new {
                client.create_workflow(&request).await
            } else {
                client.update_workflow(workflow_id, &request).await
            };

            this.update(cx, |canvas, cx| {
                match result {
                    Ok(workflow) => {
                        let saved_workflow = workflow.clone();
                        canvas.workflow = Some(saved_workflow);
                        canvas.save_state = SaveState::Success;
                        upsert_workflow_def_cache(workflow, cx);
                        cx.emit(WorkflowCanvasEvent::WorkflowSaved);
                    }
                    Err(error) => {
                        canvas.save_state = SaveState::Error(error.to_string());
                    }
                }
                cx.notify();
            })
            .ok();

            cx.background_executor()
                .timer(std::time::Duration::from_secs(3))
                .await;

            this.update(cx, |canvas, cx| {
                canvas.save_state = SaveState::Idle;
                cx.notify();
            })
            .ok();
        }));
    }
}

impl gpui::Focusable for WorkflowCanvas {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn node_fill_and_border(
    primitive: &WorkflowNodePrimitive,
    appearance: gpui::WindowAppearance,
) -> (gpui::Rgba, gpui::Rgba) {
    use gpui::WindowAppearance::*;
    match (primitive, appearance) {
        (WorkflowNodePrimitive::Llm, Dark | VibrantDark) => {
            (gpui::rgba(0x1a3a5cff), gpui::rgba(0x3b82f6ff))
        }
        (WorkflowNodePrimitive::Llm, _) => (gpui::rgba(0xdbeafeff), gpui::rgba(0x3b82f6ff)),
        (WorkflowNodePrimitive::Conditional, Dark | VibrantDark) => {
            (gpui::rgba(0x3a2e00ff), gpui::rgba(0xf59e0bff))
        }
        (WorkflowNodePrimitive::Conditional, _) => (gpui::rgba(0xfef3c7ff), gpui::rgba(0xf59e0bff)),
        (WorkflowNodePrimitive::Globals, Dark | VibrantDark) => {
            (gpui::rgba(0x10283bff), gpui::rgba(0x38bdf8ff))
        }
        (WorkflowNodePrimitive::Globals, _) => (gpui::rgba(0xe0f2feff), gpui::rgba(0x0ea5e9ff)),
        (WorkflowNodePrimitive::ExecuteShellCommand, Dark | VibrantDark) => {
            (gpui::rgba(0x0d2e1aff), gpui::rgba(0x22c55eff))
        }
        (WorkflowNodePrimitive::ExecuteShellCommand, _) => {
            (gpui::rgba(0xdcfce7ff), gpui::rgba(0x22c55eff))
        }
    }
}

fn status_badge_color(status: &TaskLifecycleStatus) -> gpui::Rgba {
    match status {
        TaskLifecycleStatus::Queued => gpui::rgba(0x6b7280ff),
        TaskLifecycleStatus::Running => gpui::rgba(0x3b82f6ff),
        TaskLifecycleStatus::Completed => gpui::rgba(0x22c55eff),
        TaskLifecycleStatus::Failed => gpui::rgba(0xef4444ff),
    }
}

fn status_badge_text_color() -> gpui::Rgba {
    gpui::rgba(0xf8fafcff)
}

fn paint_node(
    layout: &CanvasLayout,
    node: &WorkflowNode,
    primitive: &WorkflowNodePrimitive,
    node_type_label: &str,
    pos: &NodePos,
    input_ports: &[crate::client::WorkflowNodePort],
    output_ports: &[crate::client::WorkflowNodePort],
    selected: bool,
    status: Option<&TaskLifecycleStatus>,
    origin: Point<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let top_left = to_screen_point(layout, pos.x, pos.y, origin);
    let width = scaled(layout, NODE_WIDTH_F);
    let height = scaled(layout, NODE_HEIGHT_F);
    let corner_radius = scaled(layout, NODE_CORNER_RADIUS.as_f32());
    let border_width = if selected {
        scaled(layout, BORDER_WIDTH_SELECTED.as_f32())
    } else {
        scaled(layout, BORDER_WIDTH_NORMAL.as_f32())
    };

    let appearance = window.appearance();
    let (fill_color, border_color) = node_fill_and_border(primitive, appearance);

    let bounds = gpui::Bounds {
        origin: top_left,
        size: gpui::size(width, height),
    };

    let paint_quad = gpui::quad(
        bounds,
        corner_radius,
        fill_color,
        border_width,
        border_color,
        gpui::BorderStyle::Solid,
    );
    window.paint_quad(paint_quad);

    if let Some(status) = status {
        paint_status_badge(layout, pos, status, origin, window, cx);
    }

    paint_ports(layout, pos, input_ports, true, origin, window);
    paint_ports(layout, pos, output_ports, false, origin, window);
    paint_port_labels(layout, pos, input_ports, true, origin, window, cx);
    paint_port_labels(layout, pos, output_ports, false, origin, window, cx);
    paint_label(layout, node, node_type_label, pos, origin, window, cx);
}

fn paint_ports(
    layout: &CanvasLayout,
    pos: &NodePos,
    ports: &[crate::client::WorkflowNodePort],
    input_side: bool,
    origin: Point<Pixels>,
    window: &mut Window,
) {
    for (index, _port) in ports.iter().enumerate() {
        let (port_x, port_y) = port_canvas_position(pos, input_side, index, ports.len());
        let center = to_screen_point(layout, port_x, port_y, origin);
        let radius = px(PORT_RADIUS_F * layout.zoom);
        let port_bounds = gpui::Bounds {
            origin: gpui::point(center.x - radius, center.y - radius),
            size: gpui::size(radius * 2.0, radius * 2.0),
        };
        let paint_quad = gpui::quad(
            port_bounds,
            radius,
            gpui::rgba(0xf8fafcff),
            scaled(layout, 1.5),
            gpui::rgba(0x6b7280ff),
            gpui::BorderStyle::Solid,
        );
        window.paint_quad(paint_quad);
    }
}

fn paint_port_labels(
    layout: &CanvasLayout,
    pos: &NodePos,
    ports: &[crate::client::WorkflowNodePort],
    input_side: bool,
    origin: Point<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let font_size = scaled(layout, PORT_LABEL_FONT_SIZE_F);
    let font = gpui::Font::default();
    let text_color = cx.theme().colors().text_muted;

    for (index, port) in ports.iter().enumerate() {
        let (_, port_y) = port_canvas_position(pos, input_side, index, ports.len());
        let label_text: gpui::SharedString = port.label.clone().into();
        let run = gpui::TextRun {
            len: label_text.len(),
            font: font.clone(),
            color: text_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let layout_line =
            window
                .text_system()
                .layout_line(&label_text, font_size, &[run.clone()], None);
        let shaped = window
            .text_system()
            .shape_line(label_text, font_size, &[run], None);
        let label_width_canvas = layout_line.width.as_f32() / layout.zoom;
        let label_x = port_label_canvas_x(pos, input_side, label_width_canvas);
        let label_y = port_y - font_size.as_f32() / layout.zoom / 2.0;
        let label_origin = to_screen_point(layout, label_x, label_y, origin);
        let line_height = font_size * 1.4;

        shaped
            .paint(
                label_origin,
                line_height,
                gpui::TextAlign::Left,
                None,
                window,
                cx,
            )
            .log_err();
    }
}

fn paint_status_badge(
    layout: &CanvasLayout,
    pos: &NodePos,
    status: &TaskLifecycleStatus,
    origin: Point<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let label = status.display_name();
    let font_size = scaled(layout, 9.0);
    let font = gpui::Font::default();
    let text_color = status_badge_text_color();
    let text_run = gpui::TextRun {
        len: label.len(),
        font,
        color: gpui::Hsla::from(text_color),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let layout_line = window
        .text_system()
        .layout_line(label, font_size, &[text_run.clone()], None);
    let shaped_line = window
        .text_system()
        .shape_line(label.into(), font_size, &[text_run], None);

    let horizontal_padding = scaled(layout, 8.0);
    let vertical_padding = scaled(layout, 4.0);
    let badge_width = layout_line.width + horizontal_padding * 2.0;
    let badge_height = font_size + vertical_padding * 2.0;
    let badge_x = pos.x + NODE_WIDTH_F - badge_width.as_f32() / layout.zoom - 10.0;
    let badge_y = pos.y + 8.0;
    let badge_origin = to_screen_point(layout, badge_x, badge_y, origin);
    let badge_bounds = gpui::Bounds {
        origin: badge_origin,
        size: gpui::size(badge_width, badge_height),
    };

    window.paint_quad(gpui::quad(
        badge_bounds,
        badge_height / 2.0,
        status_badge_color(status),
        px(0.0),
        gpui::rgba(0x00000000),
        gpui::BorderStyle::Solid,
    ));

    shaped_line
        .paint(
            gpui::point(
                badge_origin.x + horizontal_padding,
                badge_origin.y + vertical_padding,
            ),
            font_size * 1.2,
            gpui::TextAlign::Left,
            None,
            window,
            cx,
        )
        .log_err();
}

fn paint_label(
    layout: &CanvasLayout,
    node: &WorkflowNode,
    node_type_label: &str,
    pos: &NodePos,
    origin: Point<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let font_size = scaled(layout, 13.0);
    let label_text: gpui::SharedString = node.label.clone().into();
    let text_color = cx.theme().colors().text;
    let font = gpui::Font::default();
    let run = gpui::TextRun {
        len: label_text.len(),
        font,
        color: text_color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = window
        .text_system()
        .shape_line(label_text, font_size, &[run], None);
    let label_x = pos.x + NODE_HEADER_X_INSET_F;
    let label_y = pos.y + NODE_HEADER_Y_F;
    let label_origin = to_screen_point(layout, label_x, label_y, origin);
    let line_height = font_size * 1.5;
    shaped
        .paint(
            label_origin,
            line_height,
            gpui::TextAlign::Left,
            None,
            window,
            cx,
        )
        .log_err();

    let kind_text: gpui::SharedString = node_type_label.to_string().into();
    let kind_color = cx.theme().colors().text_muted;
    let kind_font_size = scaled(layout, 11.0);
    let kind_font = gpui::Font::default();
    let kind_run = gpui::TextRun {
        len: kind_text.len(),
        font: kind_font,
        color: kind_color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let kind_shaped = window
        .text_system()
        .shape_line(kind_text, kind_font_size, &[kind_run], None);
    let kind_y = pos.y + NODE_KIND_Y_F;
    let kind_origin = to_screen_point(layout, label_x, kind_y, origin);
    let kind_line_height = kind_font_size * 1.5;
    kind_shaped
        .paint(
            kind_origin,
            kind_line_height,
            gpui::TextAlign::Left,
            None,
            window,
            cx,
        )
        .log_err();
}

fn is_backward_edge(from: (f32, f32), to: (f32, f32)) -> bool {
    to.0 < from.0 - 20.0
}

// React Flow-style bezier control point offset.
// Scales with distance for forward edges; uses sqrt scaling for backward/opposing ones.
fn bezier_ctrl_offset(dx: f32) -> f32 {
    if dx >= 0.0 {
        (dx * 0.5).max(40.0)
    } else {
        // React Flow: curvature(0.25) * 25 * sqrt(-dx)
        0.25 * 25.0 * (-dx).sqrt()
    }
}

fn paint_edge(
    layout: &CanvasLayout,
    from: (f32, f32),
    to: (f32, f32),
    waypoint: Option<(f32, f32)>,
    is_selected: bool,
    origin: Point<Pixels>,
    window: &mut Window,
) {
    let from_pt = to_screen_point(layout, from.0, from.1, origin);
    let to_pt = to_screen_point(layout, to.0, to.1, origin);
    let edge_color = if is_selected {
        gpui::rgba(0xffffffff)
    } else {
        gpui::rgba(0x9ca3afff)
    };
    let stroke_width = scaled(layout, if is_selected { 2.5 } else { EDGE_STROKE.as_f32() });

    if let Some(wp) = waypoint {
        let wp_pt = to_screen_point(layout, wp.0, wp.1, origin);

        if is_selected {
            let glow_color = gpui::rgba(0xffffff40);
            let glow_width = scaled(layout, 6.0);
            let dx1 = (wp_pt.x - from_pt.x).as_f32();
            let off1 = px(bezier_ctrl_offset(dx1));
            let mut gb = gpui::PathBuilder::stroke(glow_width);
            gb.move_to(from_pt);
            gb.cubic_bezier_to(
                wp_pt,
                gpui::point(from_pt.x + off1, from_pt.y),
                gpui::point(wp_pt.x - off1, wp_pt.y),
            );
            if let Ok(p) = gb.build() {
                window.paint_path(p, glow_color);
            }
            let dx2 = (to_pt.x - wp_pt.x).as_f32();
            let off2 = px(bezier_ctrl_offset(dx2));
            let mut gb2 = gpui::PathBuilder::stroke(glow_width);
            gb2.move_to(wp_pt);
            gb2.cubic_bezier_to(
                to_pt,
                gpui::point(wp_pt.x + off2, wp_pt.y),
                gpui::point(to_pt.x - off2, to_pt.y),
            );
            if let Ok(p) = gb2.build() {
                window.paint_path(p, glow_color);
            }
        }

        let dx1 = (wp_pt.x - from_pt.x).as_f32();
        let off1 = px(bezier_ctrl_offset(dx1));
        let ctrl_a1 = gpui::point(from_pt.x + off1, from_pt.y);
        let ctrl_b1 = gpui::point(wp_pt.x - off1, wp_pt.y);
        let mut b1 = gpui::PathBuilder::stroke(stroke_width);
        b1.move_to(from_pt);
        b1.cubic_bezier_to(wp_pt, ctrl_a1, ctrl_b1);
        if let Ok(p) = b1.build() {
            window.paint_path(p, edge_color);
        }

        let dx2 = (to_pt.x - wp_pt.x).as_f32();
        let off2 = px(bezier_ctrl_offset(dx2));
        let ctrl_a2 = gpui::point(wp_pt.x + off2, wp_pt.y);
        let ctrl_b2 = gpui::point(to_pt.x - off2, to_pt.y);
        let mut b2 = gpui::PathBuilder::stroke(stroke_width);
        b2.move_to(wp_pt);
        b2.cubic_bezier_to(to_pt, ctrl_a2, ctrl_b2);
        if let Ok(p) = b2.build() {
            window.paint_path(p, edge_color);
        }

        paint_arrowhead_directed(
            layout,
            to_pt,
            (to_pt.x - ctrl_b2.x).as_f32(),
            (to_pt.y - ctrl_b2.y).as_f32(),
            edge_color,
            window,
        );
    } else {
        let dx = (to_pt.x - from_pt.x).as_f32();
        let off = px(bezier_ctrl_offset(dx));
        let ctrl_a = gpui::point(from_pt.x + off, from_pt.y);
        let ctrl_b = gpui::point(to_pt.x - off, to_pt.y);
        if is_selected {
            let glow_color = gpui::rgba(0xffffff40);
            let glow_width = scaled(layout, 6.0);
            let mut glow_builder = gpui::PathBuilder::stroke(glow_width);
            glow_builder.move_to(from_pt);
            glow_builder.cubic_bezier_to(to_pt, ctrl_a, ctrl_b);
            if let Ok(glow_path) = glow_builder.build() {
                window.paint_path(glow_path, glow_color);
            }
        }
        let mut builder = gpui::PathBuilder::stroke(stroke_width);
        builder.move_to(from_pt);
        builder.cubic_bezier_to(to_pt, ctrl_a, ctrl_b);
        if let Ok(path) = builder.build() {
            window.paint_path(path, edge_color);
        }
        paint_arrowhead_directed(
            layout,
            to_pt,
            (to_pt.x - ctrl_b.x).as_f32(),
            (to_pt.y - ctrl_b.y).as_f32(),
            edge_color,
            window,
        );
    }
}


fn paint_arc_backward_edge(
    layout: &CanvasLayout,
    from_pt: Point<Pixels>,
    to_pt: Point<Pixels>,
    rail_y_canvas: f32,
    edge_color: gpui::Rgba,
    stroke_width: Pixels,
    is_selected: bool,
    origin: Point<Pixels>,
    window: &mut Window,
) {
    let h_gap = px(layout.zoom * 16.0);
    let rail_screen_y = to_screen_point(layout, 0.0, rail_y_canvas, origin).y;
    let corner_r = px(layout.zoom * 12.0);

    let p1 = gpui::point(from_pt.x + h_gap, from_pt.y);
    let p2 = gpui::point(from_pt.x + h_gap, rail_screen_y);
    let p3 = gpui::point(to_pt.x - h_gap, rail_screen_y);
    let p4 = gpui::point(to_pt.x - h_gap, to_pt.y);
    let pts = [from_pt, p1, p2, p3, p4, to_pt];

    if is_selected {
        let glow_color = gpui::rgba(0xffffff40);
        let glow_width = scaled(layout, 6.0);
        let mut glow_builder = gpui::PathBuilder::stroke(glow_width);
        paint_smoothstep_polyline(&mut glow_builder, &pts, corner_r);
        if let Ok(p) = glow_builder.build() {
            window.paint_path(p, glow_color);
        }
    }

    let mut builder = gpui::PathBuilder::stroke(stroke_width);
    paint_smoothstep_polyline(&mut builder, &pts, corner_r);
    if let Ok(path) = builder.build() {
        window.paint_path(path, edge_color);
    }
    paint_arrowhead_directed(layout, to_pt, 1.0, 0.0, edge_color, window);
}

// Polyline with rounded corners using cubic bezier approximation of quadratic arcs.
fn paint_smoothstep_polyline(
    builder: &mut gpui::PathBuilder,
    points: &[Point<Pixels>],
    corner_r: Pixels,
) {
    if points.len() < 2 {
        return;
    }
    builder.move_to(points[0]);
    for i in 0..points.len() - 1 {
        let curr = points[i];
        let next = points[i + 1];
        if i + 2 >= points.len() {
            builder.line_to(next);
            continue;
        }
        let after = points[i + 2];
        let seg1_len = {
            let dx = (next.x - curr.x).as_f32();
            let dy = (next.y - curr.y).as_f32();
            (dx * dx + dy * dy).sqrt()
        };
        let seg2_len = {
            let dx = (after.x - next.x).as_f32();
            let dy = (after.y - next.y).as_f32();
            (dx * dx + dy * dy).sqrt()
        };
        let r = corner_r
            .min(px(seg1_len * 0.45))
            .min(px(seg2_len * 0.45));
        if r.as_f32() < 0.5 {
            builder.line_to(next);
            continue;
        }
        let t1 = 1.0 - r.as_f32() / seg1_len.max(0.001);
        let corner_start = gpui::point(
            curr.x + (next.x - curr.x) * t1,
            curr.y + (next.y - curr.y) * t1,
        );
        let t2 = r.as_f32() / seg2_len.max(0.001);
        let corner_end = gpui::point(
            next.x + (after.x - next.x) * t2,
            next.y + (after.y - next.y) * t2,
        );
        builder.line_to(corner_start);
        // Both control points at the corner vertex approximates a quadratic arc
        builder.cubic_bezier_to(corner_end, next, next);
    }
}

fn paint_arrowhead_directed(
    layout: &CanvasLayout,
    tip: Point<Pixels>,
    dir_x: f32,
    dir_y: f32,
    color: gpui::Rgba,
    window: &mut Window,
) {
    let length = (dir_x * dir_x + dir_y * dir_y).sqrt();
    if length < 0.001 {
        return;
    }
    let ux = dir_x / length;
    let uy = dir_y / length;

    let size = layout.zoom * 10.0;
    let half = size * 0.5;

    let p1 = tip;
    let p2 = gpui::point(
        tip.x - px(size * ux) - px(half * uy),
        tip.y - px(size * uy) + px(half * ux),
    );
    let p3 = gpui::point(
        tip.x - px(size * ux) + px(half * uy),
        tip.y - px(size * uy) - px(half * ux),
    );

    let mut builder = gpui::PathBuilder::fill();
    builder.move_to(p1);
    builder.line_to(p2);
    builder.line_to(p3);
    builder.close();
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

fn cubic_bezier_point(
    from: Point<Pixels>,
    ctrl_a: Point<Pixels>,
    ctrl_b: Point<Pixels>,
    to: Point<Pixels>,
    t: f32,
) -> Point<Pixels> {
    let one_minus_t = 1.0 - t;
    let x = one_minus_t.powi(3) * from.x.as_f32()
        + 3.0 * one_minus_t.powi(2) * t * ctrl_a.x.as_f32()
        + 3.0 * one_minus_t * t.powi(2) * ctrl_b.x.as_f32()
        + t.powi(3) * to.x.as_f32();
    let y = one_minus_t.powi(3) * from.y.as_f32()
        + 3.0 * one_minus_t.powi(2) * t * ctrl_a.y.as_f32()
        + 3.0 * one_minus_t * t.powi(2) * ctrl_b.y.as_f32()
        + t.powi(3) * to.y.as_f32();
    gpui::point(px(x), px(y))
}

fn distance_to_segment(point: Point<Pixels>, start: Point<Pixels>, end: Point<Pixels>) -> f32 {
    let px_value = point.x.as_f32();
    let py_value = point.y.as_f32();
    let sx = start.x.as_f32();
    let sy = start.y.as_f32();
    let ex = end.x.as_f32();
    let ey = end.y.as_f32();
    let dx = ex - sx;
    let dy = ey - sy;
    let length_squared = dx * dx + dy * dy;
    if length_squared == 0.0 {
        return ((px_value - sx).powi(2) + (py_value - sy).powi(2)).sqrt();
    }

    let t = (((px_value - sx) * dx + (py_value - sy) * dy) / length_squared).clamp(0.0, 1.0);
    let projection_x = sx + t * dx;
    let projection_y = sy + t * dy;
    ((px_value - projection_x).powi(2) + (py_value - projection_y).powi(2)).sqrt()
}

fn edge_contains_screen_point(
    layout: &CanvasLayout,
    from: (f32, f32),
    to: (f32, f32),
    origin: Point<Pixels>,
    screen_point: Point<Pixels>,
) -> bool {
    let from_pt = to_screen_point(layout, from.0, from.1, origin);
    let to_pt = to_screen_point(layout, to.0, to.1, origin);
    let threshold = (EDGE_STROKE.as_f32() * layout.zoom + 8.0).max(8.0);

    let dx = (to_pt.x - from_pt.x).as_f32();
    let off = px(bezier_ctrl_offset(dx));
    let ctrl_a = gpui::point(from_pt.x + off, from_pt.y);
    let ctrl_b = gpui::point(to_pt.x - off, to_pt.y);
    let mut previous = from_pt;
    for step in 1..=20 {
        let t = step as f32 / 20.0;
        let current = cubic_bezier_point(from_pt, ctrl_a, ctrl_b, to_pt, t);
        if distance_to_segment(screen_point, previous, current) <= threshold {
            return true;
        }
        previous = current;
    }
    false
}

fn backward_arc_contains_screen_point(
    layout: &CanvasLayout,
    from: (f32, f32),
    to: (f32, f32),
    rail_y_canvas: f32,
    screen_point: Point<Pixels>,
    origin: Point<Pixels>,
) -> bool {
    let h_gap = layout.zoom * 16.0;
    let from_pt = to_screen_point(layout, from.0, from.1, origin);
    let to_pt = to_screen_point(layout, to.0, to.1, origin);
    let rail_screen_y = to_screen_point(layout, 0.0, rail_y_canvas, origin).y;
    let threshold = 8.0f32;

    let pts = [
        from_pt,
        gpui::point(from_pt.x + px(h_gap), from_pt.y),
        gpui::point(from_pt.x + px(h_gap), rail_screen_y),
        gpui::point(to_pt.x - px(h_gap), rail_screen_y),
        gpui::point(to_pt.x - px(h_gap), to_pt.y),
        to_pt,
    ];
    pts.windows(2)
        .any(|seg| distance_to_segment(screen_point, seg[0], seg[1]) <= threshold)
}

fn edge_canvas_midpoint(from: (f32, f32), to: (f32, f32)) -> (f32, f32) {
    let from_pt = gpui::point(px(from.0), px(from.1));
    let to_pt = gpui::point(px(to.0), px(to.1));
    let dx = to.0 - from.0;
    let off = bezier_ctrl_offset(dx);
    let ctrl_a = gpui::point(px(from.0 + off), px(from.1));
    let ctrl_b = gpui::point(px(to.0 - off), px(to.1));
    let mid = cubic_bezier_point(from_pt, ctrl_a, ctrl_b, to_pt, 0.5);
    (mid.x.as_f32(), mid.y.as_f32())
}

const DEFAULT_INPUT_PORTS: &[crate::client::WorkflowNodePort] =
    &[crate::client::WorkflowNodePort {
        id: String::new(),
        label: String::new(),
    }];

const DEFAULT_OUTPUT_PORTS: &[crate::client::WorkflowNodePort] =
    &[crate::client::WorkflowNodePort {
        id: String::new(),
        label: String::new(),
    }];

fn default_primitive_for_node(
    node_type: &str,
    legacy_category: Option<&WorkflowNodeTypeCategory>,
) -> WorkflowNodePrimitive {
    infer_workflow_node_primitive(node_type, legacy_category, None)
}

fn port_canvas_position(
    pos: &NodePos,
    input_side: bool,
    index: usize,
    port_count: usize,
) -> (f32, f32) {
    let port_count = port_count.max(1);
    let usable_height =
        (NODE_HEIGHT_F - NODE_PORTS_TOP_INSET_F - NODE_PORTS_BOTTOM_INSET_F).max(1.0);
    let spacing = usable_height / (port_count as f32 + 1.0);
    let port_y = pos.y + NODE_PORTS_TOP_INSET_F + spacing * (index as f32 + 1.0);
    let port_x = if input_side {
        pos.x
    } else {
        pos.x + NODE_WIDTH_F
    };
    (port_x, port_y)
}

fn port_label_canvas_x(pos: &NodePos, input_side: bool, label_width_canvas: f32) -> f32 {
    if input_side {
        pos.x + PORT_LABEL_X_INSET_F
    } else {
        pos.x + NODE_WIDTH_F - PORT_LABEL_X_INSET_F - label_width_canvas
    }
}

fn apply_canvas_zoom_at_position(
    layout: &mut CanvasLayout,
    origin: Point<Pixels>,
    position: Point<Pixels>,
    zoom_factor: f32,
) {
    let zoom_factor = zoom_factor.max(0.01);
    let previous_zoom = layout.zoom.max(0.1);
    let next_zoom = (previous_zoom * zoom_factor).clamp(0.1, 5.0);
    let canvas_x = (position.x - origin.x).as_f32() / previous_zoom - layout.viewport_offset.0;
    let canvas_y = (position.y - origin.y).as_f32() / previous_zoom - layout.viewport_offset.1;

    layout.zoom = next_zoom;
    layout.viewport_offset = (
        (position.x - origin.x).as_f32() / next_zoom - canvas_x,
        (position.y - origin.y).as_f32() / next_zoom - canvas_y,
    );
}

fn workflow_node_by_id<'a>(
    workflow: &'a WorkflowDefinitionRecord,
    node_id: &str,
) -> Option<&'a WorkflowNode> {
    workflow.nodes.iter().find(|node| node.id == node_id)
}

fn node_type_by_id<'a>(
    node_types: &'a [WorkflowNodeType],
    node_type_id: &str,
) -> Option<&'a WorkflowNodeType> {
    node_types
        .iter()
        .find(|node_type| node_type.id == node_type_id)
}

fn effective_ports_for_node(
    node_types: &[WorkflowNodeType],
    node: &WorkflowNode,
    input_side: bool,
) -> Vec<WorkflowNodePort> {
    let Some(node_type) = node_type_by_id(node_types, &node.node_type) else {
        return if input_side {
            DEFAULT_INPUT_PORTS.to_vec()
        } else {
            DEFAULT_OUTPUT_PORTS.to_vec()
        };
    };

    match node_type.primitive_kind() {
        WorkflowNodePrimitive::Conditional if !input_side => {
            conditional_output_ports(&node.configuration)
        }
        WorkflowNodePrimitive::Globals => Vec::new(),
        _ => {
            if input_side {
                node_type.inputs.clone()
            } else {
                node_type.outputs.clone()
            }
        }
    }
}

fn port_position_for_node(
    layout: &CanvasLayout,
    workflow: &WorkflowDefinitionRecord,
    node_types: &[WorkflowNodeType],
    node_id: &str,
    port_id: &str,
    input_side: bool,
) -> Option<(f32, f32)> {
    let node = workflow_node_by_id(workflow, node_id)?;
    let pos = layout.node_positions.get(node_id)?;
    let ports = effective_ports_for_node(node_types, node, input_side);
    let index = ports
        .iter()
        .position(|port| port.id == port_id)
        .unwrap_or_default();
    Some(port_canvas_position(pos, input_side, index, ports.len()))
}

fn run_failure_message(run: &TaskStatusResponse) -> Option<String> {
    if run.task.status != TaskLifecycleStatus::Failed {
        return None;
    }

    if let Some(message) = run.failure_message.as_ref().map(|message| message.trim())
        && !message.is_empty()
    {
        return Some(message.to_string());
    }

    if let Some(node) = run
        .nodes
        .iter()
        .find(|node| node.status == TaskLifecycleStatus::Failed)
        && let Some(output) = node.output.as_ref().map(|output| output.trim())
        && !output.is_empty()
    {
        return Some(format!("{} failed: {}", node.label, output));
    }

    Some(format!("{} failed", run.task.title))
}

struct WorkflowRunFailureToast;
struct WorkflowRunConversationToast;

fn canvas_matches_run_task_id(canvas: &WorkflowCanvas, task_id: Uuid) -> bool {
    canvas
        .run
        .as_ref()
        .map(|existing_run| existing_run.task.id == task_id)
        .unwrap_or(false)
}

fn show_run_failure_toast(
    workspace: &mut Workspace,
    task_id: Uuid,
    message: impl Into<String>,
    cx: &mut gpui::Context<Workspace>,
) {
    workspace.show_toast(
        Toast::new(
            NotificationId::composite::<WorkflowRunFailureToast>(task_id.to_string()),
            message.into(),
        )
        .autohide(),
        cx,
    );
}

impl gpui::Render for WorkflowCanvas {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        use ui::prelude::*;

        if self.has_running_nodes() {
            window.request_animation_frame();
            self.animation_phase = (self.animation_phase + 0.05) % 1.0;
        }

        let workflow = self.workflow.clone();
        let run = self.run.clone();
        let node_types = self.node_types.clone();
        let layout = self.layout.clone();
        let selection = self.selection.clone();
        let pending_connection = self.pending_connection.clone();
        let pending_connection_target = self.pending_connection_target;
        let is_edit = self.is_editable();
        let this_weak = cx.weak_entity();
        let (bbox_top, bbox_bottom) = nodes_bounding_box(&self.layout.node_positions, 40.0);
        let backward_rails = if self.workflow.is_some() {
            self.backward_edge_rail_assignments(bbox_top, bbox_bottom)
        } else {
            Default::default()
        };

        div()
            .size_full()
            .relative()
            .bg(cx.theme().colors().editor_background)
            .when(is_edit, |this| this.child(self.render_toolbar(cx)))
            .child(
                gpui::canvas(
                    move |bounds, _window, cx| {
                        this_weak
                            .update(cx, |canvas, _| {
                                canvas.canvas_bounds = Some(bounds);
                            })
                            .ok();
                        bounds
                    },
                    move |bounds, _prepaint, window, cx| {
                        let origin = bounds.origin;
                        if let Some(ref wf) = workflow {
                            let edge_draw_list: Vec<(WorkflowEdge, (f32, f32), (f32, f32))> = wf
                                .edges
                                .iter()
                                .filter_map(|edge| {
                                    Some((
                                        edge.clone(),
                                        port_position_for_node(
                                            &layout,
                                            wf,
                                            &node_types,
                                            &edge.from_node_id,
                                            &edge.from_output_id,
                                            false,
                                        )?,
                                        port_position_for_node(
                                            &layout,
                                            wf,
                                            &node_types,
                                            &edge.to_node_id,
                                            &edge.to_input_id,
                                            true,
                                        )?,
                                    ))
                                })
                                .collect();
                            for (edge, from_port, to_port) in &edge_draw_list {
                                if !is_backward_edge(*from_port, *to_port) {
                                    let is_selected = matches!(&selection,
                                        CanvasSelection::Edge(fn_id, fo_id, tn_id, ti_id)
                                        if fn_id == &edge.from_node_id
                                            && fo_id == &edge.from_output_id
                                            && tn_id == &edge.to_node_id
                                            && ti_id == &edge.to_input_id
                                    );
                                    let edge_key = (
                                        edge.from_node_id.clone(),
                                        edge.from_output_id.clone(),
                                        edge.to_node_id.clone(),
                                        edge.to_input_id.clone(),
                                    );
                                    let waypoint = layout.edge_waypoints.get(&edge_key).copied();
                                    paint_edge(&layout, *from_port, *to_port, waypoint, is_selected, origin, window);
                                }
                            }
                            for node in &wf.nodes {
                                if node.node_type == WORKFLOW_GLOBALS_NODE_TYPE_ID {
                                    continue;
                                }
                                let pos = layout
                                    .node_positions
                                    .get(&node.id)
                                    .copied()
                                    .unwrap_or(NodePos { x: 40.0, y: 40.0 });
                                let selected = matches!(&selection, CanvasSelection::Node(id) if *id == node.id);
                                let node_type_label = node_type_by_id(&node_types, &node.node_type)
                                    .map(|node_type| node_type.label.as_str())
                                    .unwrap_or(node.node_type.as_str());
                                let primitive = node_type_by_id(&node_types, &node.node_type)
                                    .map(|node_type| node_type.primitive_kind())
                                    .unwrap_or_else(|| default_primitive_for_node(&node.node_type, None));
                                let input_ports = effective_ports_for_node(&node_types, node, true);
                                let output_ports = effective_ports_for_node(&node_types, node, false);
                                paint_node(
                                    &layout,
                                    node,
                                    &primitive,
                                    node_type_label,
                                    &pos,
                                    &input_ports,
                                    &output_ports,
                                    selected,
                                    None,
                                    origin,
                                    window,
                                    cx,
                                );
                            }
                            for (edge, from_port, to_port) in &edge_draw_list {
                                if is_backward_edge(*from_port, *to_port) {
                                    let is_selected = matches!(&selection,
                                        CanvasSelection::Edge(fn_id, fo_id, tn_id, ti_id)
                                        if fn_id == &edge.from_node_id
                                            && fo_id == &edge.from_output_id
                                            && tn_id == &edge.to_node_id
                                            && ti_id == &edge.to_input_id
                                    );
                                    let edge_color = if is_selected {
                                        gpui::rgba(0xffffffff)
                                    } else {
                                        gpui::rgba(0x9ca3afff)
                                    };
                                    let stroke_width = scaled(&layout, if is_selected { 2.5 } else { EDGE_STROKE.as_f32() });
                                    let from_pt = to_screen_point(&layout, from_port.0, from_port.1, origin);
                                    let to_pt = to_screen_point(&layout, to_port.0, to_port.1, origin);
                                    let key = (
                                        edge.from_node_id.clone(),
                                        edge.from_output_id.clone(),
                                        edge.to_node_id.clone(),
                                        edge.to_input_id.clone(),
                                    );
                                    let (above, rail_index) = backward_rails.get(&key).copied().unwrap_or((true, 0));
                                    let rail_y = backward_edge_rail_y(bbox_top, bbox_bottom, rail_index, above);
                                    paint_arc_backward_edge(
                                        &layout, from_pt, to_pt, rail_y, edge_color, stroke_width, is_selected, origin, window,
                                    );
                                }
                            }
                            if is_edit {
                                if let CanvasSelection::Edge(ref fn_id, ref fo_id, ref tn_id, ref ti_id) = selection {
                                    if let Some(edge) = wf.edges.iter().find(|e| {
                                        e.from_node_id == *fn_id
                                            && e.from_output_id == *fo_id
                                            && e.to_node_id == *tn_id
                                            && e.to_input_id == *ti_id
                                    }) {
                                        let from_opt = port_position_for_node(&layout, wf, &node_types, fn_id, fo_id, false);
                                        let to_opt = port_position_for_node(&layout, wf, &node_types, tn_id, ti_id, true);
                                        if let (Some(from), Some(to)) = (from_opt, to_opt) {
                                            if !is_backward_edge(from, to) {
                                                let edge_key = (
                                                    edge.from_node_id.clone(),
                                                    edge.from_output_id.clone(),
                                                    edge.to_node_id.clone(),
                                                    edge.to_input_id.clone(),
                                                );
                                                let handle_canvas = layout
                                                    .edge_waypoints
                                                    .get(&edge_key)
                                                    .copied()
                                                    .unwrap_or_else(|| edge_canvas_midpoint(from, to));
                                                let handle_screen = to_screen_point(&layout, handle_canvas.0, handle_canvas.1, origin);
                                                let r = scaled(&layout, 5.0);
                                                let handle_bounds = gpui::Bounds {
                                                    origin: gpui::point(handle_screen.x - r, handle_screen.y - r),
                                                    size: gpui::size(r * 2.0, r * 2.0),
                                                };
                                                window.paint_quad(gpui::PaintQuad {
                                                    bounds: handle_bounds,
                                                    corner_radii: gpui::Corners::all(r),
                                                    background: gpui::rgba(0x1e3a5fff).into(),
                                                    border_widths: gpui::Edges::all(px(1.5)),
                                                    border_color: gpui::rgba(0xffffffff).into(),
                                                    border_style: gpui::BorderStyle::Solid,
                                                });
                                            }
                                        }
                                    }
                                }
                            }

                            if let (Some(source), Some(target)) =
                                (pending_connection.as_ref(), pending_connection_target)
                            {
                                if let Some(from_port) = port_position_for_node(
                                    &layout,
                                    wf,
                                    &node_types,
                                    &source.node_id,
                                    &source.port_id,
                                    false,
                                ) {
                                    paint_edge(&layout, from_port, target, None, false, origin, window);
                                }
                            }
                        } else if let Some(ref run_data) = run {
                            let mut backward_run_edges: Vec<((f32, f32), (f32, f32))> = Vec::new();
                            if let Some(ref wf) = run_data.workflow {
                                let edge_positions: Vec<((f32, f32), (f32, f32))> = wf
                                    .edges
                                    .iter()
                                    .filter_map(|edge| {
                                        Some((
                                            port_position_for_node(
                                                &layout,
                                                wf,
                                                &node_types,
                                                &edge.from_node_id,
                                                &edge.from_output_id,
                                                false,
                                            )?,
                                            port_position_for_node(
                                                &layout,
                                                wf,
                                                &node_types,
                                                &edge.to_node_id,
                                                &edge.to_input_id,
                                                true,
                                            )?,
                                        ))
                                    })
                                    .collect();
                                for (from_port, to_port) in &edge_positions {
                                    if is_backward_edge(*from_port, *to_port) {
                                        backward_run_edges.push((*from_port, *to_port));
                                    } else {
                                        paint_edge(&layout, *from_port, *to_port, None, false, origin, window);
                                    }
                                }
                            }
                            for node_status in &run_data.nodes {
                                if node_status.node_type == WORKFLOW_GLOBALS_NODE_TYPE_ID {
                                    continue;
                                }
                                let pos = layout
                                    .node_positions
                                    .get(&node_status.id)
                                    .copied()
                                    .unwrap_or(NodePos { x: 40.0, y: 40.0 });
                                let synthetic_node = WorkflowNode {
                                    id: node_status.id.clone(),
                                    node_type: node_status.node_type.clone(),
                                    label: node_status.label.clone(),
                                    configuration: serde_json::json!({}),
                                    runtime: serde_json::json!({}),
                                };
                                let node_type_label = node_type_by_id(&node_types, &synthetic_node.node_type)
                                    .map(|node_type| node_type.label.as_str())
                                    .unwrap_or(synthetic_node.node_type.as_str());
                                let primitive = node_type_by_id(&node_types, &synthetic_node.node_type)
                                    .map(|node_type| node_type.primitive_kind())
                                    .unwrap_or_else(|| node_status.primitive_kind());
                                let input_ports =
                                    effective_ports_for_node(&node_types, &synthetic_node, true);
                                let output_ports =
                                    effective_ports_for_node(&node_types, &synthetic_node, false);
                                paint_node(
                                    &layout,
                                    &synthetic_node,
                                    &primitive,
                                    node_type_label,
                                    &pos,
                                    &input_ports,
                                    &output_ports,
                                    false,
                                    Some(&node_status.status),
                                    origin,
                                    window,
                                    cx,
                                );
                            }
                            {
                                let run_rail_assignments =
                                    compute_backward_edge_rails(&backward_run_edges, bbox_top, bbox_bottom);
                                let edge_color = gpui::rgba(0x9ca3afff);
                                let stroke_width = scaled(&layout, EDGE_STROKE.as_f32());
                                for ((from_port, to_port), (above, rail_index)) in
                                    backward_run_edges.iter().zip(run_rail_assignments)
                                {
                                    let rail_y = backward_edge_rail_y(bbox_top, bbox_bottom, rail_index, above);
                                    let from_pt = to_screen_point(&layout, from_port.0, from_port.1, origin);
                                    let to_pt = to_screen_point(&layout, to_port.0, to_port.1, origin);
                                    paint_arc_backward_edge(
                                        &layout, from_pt, to_pt, rail_y, edge_color, stroke_width, false, origin, window,
                                    );
                                }
                            }
                        }
                    },
                )
                .size_full(),
            )
            .key_context("WorkflowCanvas")
            .track_focus(&self.focus_handle)
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(Self::handle_mouse_down),
            )
            .on_mouse_down(
                gpui::MouseButton::Right,
                cx.listener(Self::handle_secondary_mouse_down),
            )
            .on_mouse_move(cx.listener(Self::handle_mouse_move))
            .on_mouse_up(gpui::MouseButton::Left, cx.listener(Self::handle_mouse_up))
            .on_scroll_wheel(cx.listener(Self::handle_scroll_wheel))
            .on_pinch(cx.listener(Self::handle_pinch))
            .on_key_down(cx.listener(Self::handle_key_down))
            .children(self.context_menu.as_ref().map(|(menu, position, _)| {
                gpui::deferred(
                    gpui::anchored()
                        .position(*position)
                        .anchor(Corner::TopLeft)
                        .child(menu.clone()),
                )
                .with_priority(1)
            }))
    }
}

pub fn open_workflow(
    workflow: WorkflowDefinitionRecord,
    client: Arc<WorkflowClient>,
    workspace: &mut workspace::Workspace,
    window: &mut Window,
    cx: &mut gpui::Context<workspace::Workspace>,
) {
    let existing_canvas = if workflow.id.is_nil() {
        None
    } else {
        workspace
            .items_of_type::<WorkflowCanvas>(cx)
            .find(|canvas| {
                canvas.read_with(cx, |canvas, _cx| {
                    canvas
                        .workflow
                        .as_ref()
                        .map(|existing_workflow| existing_workflow.id == workflow.id)
                        .unwrap_or(false)
                })
            })
    };

    if let Some(existing_canvas) = existing_canvas {
        existing_canvas.update(cx, |canvas, cx| {
            canvas.workflow = Some(workflow.clone());
            cx.notify();
        });
        workspace.activate_item(&existing_canvas, true, true, window, cx);
        sync_node_inspector_panel(workspace, &existing_canvas, None, window, cx);
        return;
    }

    let canvas = cx.new(|cx| WorkflowCanvas::new_edit(workflow, client, cx));
    sync_node_inspector_panel(workspace, &canvas, None, window, cx);
    cx.subscribe_in(
        &canvas,
        window,
        |workspace, canvas, event, window, cx| match event {
            WorkflowCanvasEvent::NodeSelected(selected_node_id) => {
                sync_node_inspector_panel(workspace, canvas, selected_node_id.clone(), window, cx);
            }
            WorkflowCanvasEvent::WorkflowSaved => {
                let selected_node_id =
                    canvas.read_with(cx, |canvas, _cx| match &canvas.selection {
                        CanvasSelection::Node(node_id) => Some(node_id.clone()),
                        _ => None,
                    });
                sync_node_inspector_panel(workspace, canvas, selected_node_id, window, cx);
            }
            WorkflowCanvasEvent::RunFailed { .. } => {}
            WorkflowCanvasEvent::NodeActivated(_) => {}
        },
    )
    .detach();
    workspace.add_item_to_center(Box::new(canvas), window, cx);
}

fn sync_node_inspector_panel(
    workspace: &mut Workspace,
    canvas: &gpui::Entity<WorkflowCanvas>,
    selected_node_id: Option<String>,
    window: &mut Window,
    cx: &mut gpui::Context<Workspace>,
) {
    let Some(panel) = workspace.panel::<NodeInspectorPanel>(cx) else {
        return;
    };
    let Some(workflow) = canvas.read_with(cx, |canvas, _cx| canvas.workflow.clone()) else {
        return;
    };

    panel.update(cx, |panel, cx| {
        panel.set_active_canvas(canvas);
        panel.set_workflow(workflow.clone(), cx);
        panel.set_node(selected_node_id.clone(), window, cx);
    });

    if selected_node_id.is_some() {
        workspace.open_panel::<NodeInspectorPanel>(window, cx);
    }
}

pub fn open_run(
    run: TaskStatusResponse,
    client: Arc<WorkflowClient>,
    workspace: &mut workspace::Workspace,
    window: &mut Window,
    cx: &mut gpui::Context<workspace::Workspace>,
) {
    if let Some(message) = run_failure_message(&run) {
        show_run_failure_toast(workspace, run.task.id, message, cx);
    }

    let existing_canvas = workspace
        .items_of_type::<WorkflowCanvas>(cx)
        .find(|canvas| {
            canvas.read_with(cx, |canvas, _cx| {
                canvas_matches_run_task_id(canvas, run.task.id)
            })
        });

    if let Some(existing_canvas) = existing_canvas {
        existing_canvas.update(cx, |canvas, cx| {
            canvas.workflow = run.workflow.clone();
            canvas.run = Some(run.clone());
            cx.notify();
        });
        workspace.activate_item(&existing_canvas, true, true, window, cx);
        return;
    }

    let canvas = cx.new(|cx| WorkflowCanvas::new_run(run, client, cx));
    cx.subscribe_in(
        &canvas,
        window,
        |workspace, canvas, event, window, cx| match event {
            WorkflowCanvasEvent::RunFailed { task_id, message } => {
                show_run_failure_toast(workspace, *task_id, message.clone(), cx);
            }
            WorkflowCanvasEvent::NodeActivated(node_id) => {
                open_run_node_conversation(
                    canvas.downgrade(),
                    workspace.weak_handle(),
                    node_id.clone(),
                    window,
                    cx,
                );
            }
            WorkflowCanvasEvent::NodeSelected(_) | WorkflowCanvasEvent::WorkflowSaved => {}
        },
    )
    .detach();
    workspace.add_item_to_center(Box::new(canvas), window, cx);
}

fn open_run_node_conversation(
    canvas: gpui::WeakEntity<WorkflowCanvas>,
    workspace: gpui::WeakEntity<Workspace>,
    node_id: String,
    window: &mut Window,
    cx: &mut App,
) {
    let Ok(Some((run_title, task_id, node, client))) = canvas.read_with(cx, |canvas, _cx| {
        let run = canvas.run.as_ref()?;
        let node = run.nodes.iter().find(|node| node.id == node_id)?.clone();
        Some((
            run.task.title.clone(),
            run.task.id,
            node,
            canvas.client.clone(),
        ))
    }) else {
        return;
    };

    window
        .spawn(cx, async move |cx| {
            let conversation = client.get_task_node_conversation(task_id, &node.id).await;

            match conversation {
                Ok(conversation) => {
                    workspace.update_in(cx, |workspace, window, cx| {
                        let title = format!("{run_title} / {} - conversation", node.label);
                        let work_dirs =
                            conversation_work_dirs(conversation.workspace_path.as_deref());

                        if try_open_run_node_in_center(
                            cx.entity(),
                            &conversation.session_id,
                            work_dirs.clone(),
                            conversation.remote_target.clone(),
                            title.clone().into(),
                            window,
                            cx,
                        ) {
                            return;
                        }

                        open_markdown_conversation(
                            title,
                            conversation.markdown,
                            workspace.weak_handle(),
                            workspace.project().clone(),
                            workspace.app_state().languages.clone(),
                            window,
                            cx,
                        );
                    })?;
                }
                Err(error) => {
                    workspace.update_in(cx, |workspace, _window, cx| {
                        workspace.show_toast(
                            Toast::new(
                                NotificationId::composite::<WorkflowRunConversationToast>(format!(
                                    "{task_id}:{}",
                                    node.id
                                )),
                                format!("Failed to load node conversation: {error}"),
                            )
                            .autohide(),
                            cx,
                        );
                    })?;
                }
            }

            anyhow::Ok(())
        })
        .detach_and_log_err(cx);
}

fn try_open_run_node_in_center(
    current_workspace: gpui::Entity<Workspace>,
    session_id: &str,
    work_dirs: Option<PathList>,
    remote_target: Option<TaskRemoteTarget>,
    title: gpui::SharedString,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    let Some(session_id) = normalized_session_id(session_id) else {
        return false;
    };
    let session_id = session_id.to_string();
    let current_window = window.window_handle().downcast::<MultiWorkspace>();

    if let Some(target) =
        find_run_node_workspace_target(&current_workspace, work_dirs.as_ref(), window, cx)
    {
        return open_run_node_session_in_target(
            target,
            session_id,
            work_dirs,
            title,
            current_window,
            cx,
        );
    }

    if let Some(remote_attachment) =
        run_node_remote_attachment(remote_target.as_ref(), work_dirs.as_ref())
    {
        return open_run_node_session_in_remote_workspace(
            current_workspace.downgrade(),
            current_workspace.read(cx).app_state().clone(),
            current_window,
            session_id,
            work_dirs.expect("remote attachment requires work dirs"),
            remote_attachment,
            title,
            window,
            cx,
        );
    }

    if let (Some(current_window), Some(work_dirs)) = (current_window, work_dirs) {
        return open_run_node_session_in_new_workspace(
            current_workspace.downgrade(),
            current_window,
            session_id,
            work_dirs,
            title,
            window,
            cx,
        );
    }

    false
}

fn normalized_session_id(session_id: &str) -> Option<&str> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return None;
    }

    Some(session_id)
}

#[derive(Clone)]
struct RunNodeWorkspaceTarget {
    window: WindowHandle<MultiWorkspace>,
    workspace: gpui::Entity<Workspace>,
}

#[derive(Clone)]
struct RunNodeRemoteAttachment {
    connection_options: RemoteConnectionOptions,
    paths: Vec<PathBuf>,
}

fn find_run_node_workspace_target(
    current_workspace: &gpui::Entity<Workspace>,
    work_dirs: Option<&PathList>,
    window: &mut Window,
    cx: &App,
) -> Option<RunNodeWorkspaceTarget> {
    let current_window = window.window_handle().downcast::<MultiWorkspace>()?;
    let Some(work_dirs) = work_dirs else {
        return Some(RunNodeWorkspaceTarget {
            window: current_window,
            workspace: current_workspace.clone(),
        });
    };

    if workspace_path_list(current_workspace, cx).paths() == work_dirs.paths() {
        return Some(RunNodeWorkspaceTarget {
            window: current_window,
            workspace: current_workspace.clone(),
        });
    }

    if let Some(workspace) = find_workspace_in_window(&current_window, work_dirs, cx) {
        return Some(RunNodeWorkspaceTarget {
            window: current_window,
            workspace,
        });
    }

    find_workspace_across_windows(work_dirs, cx).map(|(target_window, workspace)| {
        RunNodeWorkspaceTarget {
            window: target_window,
            workspace,
        }
    })
}

fn open_run_node_session_in_target(
    target: RunNodeWorkspaceTarget,
    session_id: String,
    work_dirs: Option<PathList>,
    title: gpui::SharedString,
    current_window: Option<WindowHandle<MultiWorkspace>>,
    cx: &mut Context<Workspace>,
) -> bool {
    let target_window = target.window;
    let target_workspace = target.workspace;
    let session_work_dirs = work_dirs.clone();

    target_window
        .update(cx, move |multi_workspace, window, cx| {
            if current_window.as_ref() != Some(&target_window) {
                window.activate_window();
            }
            multi_workspace.activate(target_workspace.clone(), cx);
            target_workspace.update(cx, |workspace, cx| {
                agent_ui::open_thread_in_center(
                    workspace,
                    agent_client_protocol::SessionId::new(session_id.clone()),
                    session_work_dirs.clone(),
                    Some(title.clone()),
                    window,
                    cx,
                );
            })
        })
        .log_err()
        .is_some()
}

fn open_run_node_session_in_new_workspace(
    current_workspace: gpui::WeakEntity<Workspace>,
    current_window: WindowHandle<MultiWorkspace>,
    session_id: String,
    work_dirs: PathList,
    title: gpui::SharedString,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    let paths: Vec<PathBuf> = work_dirs
        .paths()
        .iter()
        .map(|path| path.to_path_buf())
        .collect();
    let Some(open_task) = current_window
        .update(cx, |multi_workspace, window, cx| {
            multi_workspace.open_project(paths, window, cx)
        })
        .log_err()
    else {
        return false;
    };

    cx.spawn_in(window, async move |_, cx| {
        match open_task.await {
            Ok(workspace) => {
                current_window.update(cx, |multi_workspace, window, cx| {
                    multi_workspace.activate(workspace.clone(), cx);
                    workspace.update(cx, |workspace, cx| {
                        agent_ui::open_thread_in_center(
                            workspace,
                            agent_client_protocol::SessionId::new(session_id.clone()),
                            Some(work_dirs.clone()),
                            Some(title.clone()),
                            window,
                            cx,
                        );
                    })
                })?;
            }
            Err(error) => {
                current_workspace.update(cx, |workspace, cx| {
                    workspace.show_toast(
                        Toast::new(
                            NotificationId::composite::<WorkflowRunConversationToast>(format!(
                                "workspace-open:{session_id}"
                            )),
                            format!("Failed to open run workspace: {error}"),
                        )
                        .autohide(),
                        cx,
                    );
                })?;
            }
        }

        anyhow::Ok(())
    })
    .detach_and_log_err(cx);

    true
}

fn open_run_node_session_in_remote_workspace(
    current_workspace: gpui::WeakEntity<Workspace>,
    app_state: Arc<workspace::AppState>,
    current_window: Option<WindowHandle<MultiWorkspace>>,
    session_id: String,
    work_dirs: PathList,
    remote_attachment: RunNodeRemoteAttachment,
    title: gpui::SharedString,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> bool {
    cx.spawn_in(window, async move |_, cx| {
        if let Err(error) = open_remote_project(
            remote_attachment.connection_options,
            remote_attachment.paths,
            app_state,
            workspace::OpenOptions::default(),
            cx,
        )
        .await
        {
            show_run_node_workspace_error(
                &current_workspace,
                format!("remote-attach:{session_id}"),
                format!("Failed to attach remote workspace: {error}"),
                cx,
            )?;
            return anyhow::Ok(());
        }

        let target = cx.update(|_window, cx| find_workspace_across_windows(&work_dirs, cx))?;
        if let Some((target_window, target_workspace)) = target {
            target_window.update(cx, |multi_workspace, window, cx| {
                if current_window.as_ref() != Some(&target_window) {
                    window.activate_window();
                }
                multi_workspace.activate(target_workspace.clone(), cx);
                target_workspace.update(cx, |workspace, cx| {
                    agent_ui::open_thread_in_center(
                        workspace,
                        agent_client_protocol::SessionId::new(session_id.clone()),
                        Some(work_dirs.clone()),
                        Some(title.clone()),
                        window,
                        cx,
                    );
                })
            })?;
        } else {
            show_run_node_workspace_error(
                &current_workspace,
                format!("remote-workspace-missing:{session_id}"),
                "Attached the remote runtime, but could not locate the workspace in Neo Zed"
                    .to_string(),
                cx,
            )?;
        }

        anyhow::Ok(())
    })
    .detach_and_log_err(cx);

    true
}

fn run_node_remote_attachment(
    remote_target: Option<&TaskRemoteTarget>,
    work_dirs: Option<&PathList>,
) -> Option<RunNodeRemoteAttachment> {
    let remote_target = remote_target?;
    let paths: Vec<PathBuf> = work_dirs?.ordered_paths().cloned().collect();
    if paths.is_empty() {
        return None;
    }

    let connection_options = match remote_target {
        TaskRemoteTarget::Docker(target) => {
            RemoteConnectionOptions::Docker(DockerConnectionOptions {
                name: target.name.clone(),
                container_id: target.container_id.clone(),
                remote_user: target.remote_user.clone(),
                upload_binary_over_docker_exec: false,
                use_podman: target.use_podman,
            })
        }
    };

    Some(RunNodeRemoteAttachment {
        connection_options,
        paths,
    })
}

fn show_run_node_workspace_error(
    workspace: &gpui::WeakEntity<Workspace>,
    id: String,
    message: String,
    cx: &mut gpui::AsyncApp,
) -> anyhow::Result<()> {
    workspace.update(cx, |workspace, cx| {
        workspace.show_toast(
            Toast::new(
                NotificationId::composite::<WorkflowRunConversationToast>(id),
                message,
            )
            .autohide(),
            cx,
        );
    })?;
    Ok(())
}

fn workspace_path_list(workspace: &gpui::Entity<Workspace>, cx: &App) -> PathList {
    PathList::new(&workspace.read(cx).root_paths(cx))
}

fn find_workspace_in_window(
    target_window: &WindowHandle<MultiWorkspace>,
    path_list: &PathList,
    cx: &App,
) -> Option<gpui::Entity<Workspace>> {
    let multi_workspace = target_window.read(cx).ok()?;
    multi_workspace
        .workspaces()
        .iter()
        .find(|workspace| workspace_path_list(workspace, cx).paths() == path_list.paths())
        .cloned()
}

fn find_workspace_across_windows(
    path_list: &PathList,
    cx: &App,
) -> Option<(WindowHandle<MultiWorkspace>, gpui::Entity<Workspace>)> {
    cx.windows()
        .into_iter()
        .filter_map(|window| window.downcast::<MultiWorkspace>())
        .find_map(|window| {
            find_workspace_in_window(&window, path_list, cx).map(|workspace| (window, workspace))
        })
}

fn conversation_work_dirs(workspace_path: Option<&str>) -> Option<PathList> {
    let workspace_path = workspace_path?.trim();
    if workspace_path.is_empty() {
        return None;
    }

    Some(PathList::new(&[PathBuf::from(workspace_path)]))
}

fn open_markdown_conversation(
    title: String,
    content: String,
    workspace: gpui::WeakEntity<Workspace>,
    project: gpui::Entity<project::Project>,
    languages: std::sync::Arc<language::LanguageRegistry>,
    window: &mut Window,
    cx: &mut App,
) {
    let markdown_language = languages.language_for_name("Markdown");

    window
        .spawn(cx, async move |cx| {
            let markdown_language = markdown_language.await?;
            let buffer = project
                .update(cx, |project, cx| {
                    project.create_buffer(Some(markdown_language), false, cx)
                })
                .await?;

            buffer.update(cx, |buffer, cx| {
                buffer.set_text(content, cx);
                buffer.set_capability(language::Capability::ReadOnly, cx);
            });

            workspace.update_in(cx, |workspace, window, cx| {
                let multibuffer =
                    cx.new(|cx| MultiBuffer::singleton(buffer, cx).with_title(title.clone()));
                workspace.add_item_to_active_pane(
                    Box::new(cx.new(|cx| {
                        let mut editor =
                            Editor::for_multibuffer(multibuffer, Some(project.clone()), window, cx);
                        editor.set_read_only(true);
                        editor
                    })),
                    None,
                    true,
                    window,
                    cx,
                );
            })?;

            anyhow::Ok(())
        })
        .detach_and_log_err(cx);
}

impl workspace::Item for WorkflowCanvas {
    type Event = WorkflowCanvasEvent;

    fn tab_content_text(&self, _detail: usize, _cx: &App) -> gpui::SharedString {
        if let Some(ref run) = self.run {
            format!("Run: {}", run.task.title).into()
        } else if let Some(ref wf) = self.workflow {
            format!("Workflow: {}", wf.name).into()
        } else {
            "Workflow Canvas".into()
        }
    }

    fn to_item_events(_event: &Self::Event, _f: &mut dyn FnMut(workspace::item::ItemEvent)) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn init_test(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme::init(theme::LoadThemes::JustBase, cx);
        });
    }

    fn sample_run(task_id: Uuid) -> TaskStatusResponse {
        TaskStatusResponse {
            task: crate::client::TaskRecord {
                id: task_id,
                title: "Sample run".into(),
                source_repo: "/tmp/demo".into(),
                status: TaskLifecycleStatus::Running,
                workflow_id: None,
                task_description: None,
            },
            workflow: Some(WorkflowDefinitionRecord {
                id: Uuid::new_v4(),
                name: "Sample workflow".into(),
                nodes: vec![WorkflowNode {
                    id: "node-1".into(),
                    node_type: "task".into(),
                    label: "Build".into(),
                    configuration: serde_json::json!({}),
                    runtime: serde_json::json!({}),
                }],
                edges: vec![],
                node_policies: vec![],
                retry_behavior: crate::client::RetryBehavior::default(),
                validation_policy_ref: None,
                trigger_metadata: Default::default(),
            }),
            workspace_path: Some("/tmp/demo".into()),
            remote_target: None,
            nodes: vec![crate::client::TaskNodeStatus {
                id: "node-1".into(),
                node_type: "task".into(),
                primitive: None,
                category: Some(WorkflowNodeTypeCategory::Task),
                label: "Build".into(),
                status: TaskLifecycleStatus::Running,
                output: None,
                session_id: Some("session-1".into()),
                artifacts: Default::default(),
            }],
            outcome: None,
            agent: None,
            lease: None,
            validation: None,
            integration: None,
            failure_message: None,
            agents: None,
        }
    }

    fn sample_node_types() -> Vec<WorkflowNodeType> {
        editor_node_types(vec![
            WorkflowNodeType {
                id: "task".into(),
                label: "Task".into(),
                primitive: Some(crate::client::WorkflowNodePrimitive::Llm),
                category: None,
                is_primitive: false,
                inputs: vec![crate::client::WorkflowNodePort {
                    id: "default".into(),
                    label: "Input".into(),
                }],
                outputs: vec![crate::client::WorkflowNodePort {
                    id: "success".into(),
                    label: "Success".into(),
                }],
                configure_time_fields: vec![],
                runtime_fields: vec![],
            },
            WorkflowNodeType {
                id: "validation".into(),
                label: "Validation".into(),
                primitive: Some(crate::client::WorkflowNodePrimitive::Conditional),
                category: None,
                is_primitive: false,
                inputs: vec![crate::client::WorkflowNodePort {
                    id: "default".into(),
                    label: "Input".into(),
                }],
                outputs: vec![crate::client::WorkflowNodePort {
                    id: "passed".into(),
                    label: "Passed".into(),
                }],
                configure_time_fields: vec![],
                runtime_fields: vec![],
            },
        ])
    }

    #[test]
    fn test_auto_layout_single_node() {
        let nodes = vec![WorkflowNode {
            id: "a".into(),
            node_type: "task".into(),
            label: "A".into(),
            configuration: serde_json::json!({}),
            runtime: serde_json::json!({}),
        }];
        let edges = vec![];
        let layout = auto_layout(&nodes, &edges);
        let pos = layout["a"];
        assert!(pos.x >= 0.0);
        assert!(pos.y >= 0.0);
    }

    #[test]
    fn test_auto_layout_chain() {
        let nodes = vec![
            WorkflowNode {
                id: "a".into(),
                node_type: "task".into(),
                label: "A".into(),
                configuration: serde_json::json!({}),
                runtime: serde_json::json!({}),
            },
            WorkflowNode {
                id: "b".into(),
                node_type: "validation".into(),
                label: "B".into(),
                configuration: serde_json::json!({}),
                runtime: serde_json::json!({}),
            },
            WorkflowNode {
                id: "c".into(),
                node_type: "integration".into(),
                label: "C".into(),
                configuration: serde_json::json!({}),
                runtime: serde_json::json!({}),
            },
        ];
        let edges = vec![
            WorkflowEdge {
                from_node_id: "a".into(),
                from_output_id: "success".into(),
                to_node_id: "b".into(),
                to_input_id: "default".into(),
            },
            WorkflowEdge {
                from_node_id: "b".into(),
                from_output_id: "passed".into(),
                to_node_id: "c".into(),
                to_input_id: "default".into(),
            },
        ];
        let layout = auto_layout(&nodes, &edges);
        assert!(layout["a"].x < layout["b"].x);
        assert!(layout["b"].x < layout["c"].x);
    }

    #[test]
    fn test_canvas_layout_default_zoom_is_one() {
        let layout = CanvasLayout::default();
        assert_eq!(layout.zoom, 1.0);
    }

    #[test]
    fn test_to_screen_no_transform() {
        let layout = CanvasLayout::default();
        let origin = gpui::point(px(10.0), px(20.0));
        let screen = to_screen_point(&layout, 100.0, 50.0, origin);
        assert_eq!(screen.x, px(110.0));
        assert_eq!(screen.y, px(70.0));
    }

    #[test]
    fn test_to_screen_with_zoom() {
        let layout = CanvasLayout {
            zoom: 2.0,
            viewport_offset: (0.0, 0.0),
            ..Default::default()
        };
        let origin = gpui::point(px(0.0), px(0.0));
        let screen = to_screen_point(&layout, 50.0, 25.0, origin);
        assert_eq!(screen.x, px(100.0));
        assert_eq!(screen.y, px(50.0));
    }

    #[test]
    fn test_run_failure_message_prefers_runtime_failure_message() {
        let run = TaskStatusResponse {
            task: crate::client::TaskRecord {
                id: Uuid::new_v4(),
                title: "Broken run".into(),
                source_repo: "/tmp/demo".into(),
                status: TaskLifecycleStatus::Failed,
                workflow_id: None,
                task_description: None,
            },
            workflow: None,
            workspace_path: Some("/tmp/demo".into()),
            remote_target: None,
            nodes: vec![crate::client::TaskNodeStatus {
                id: "node-1".into(),
                node_type: "task".into(),
                primitive: None,
                category: Some(WorkflowNodeTypeCategory::Task),
                label: "Build".into(),
                status: TaskLifecycleStatus::Failed,
                output: Some("node output".into()),
                session_id: None,
                artifacts: Default::default(),
            }],
            outcome: None,
            agent: None,
            lease: None,
            validation: None,
            integration: None,
            failure_message: Some("codex execution failed: timeout".into()),
            agents: None,
        };

        assert_eq!(
            run_failure_message(&run).as_deref(),
            Some("codex execution failed: timeout")
        );
    }

    #[test]
    fn test_run_failure_message_falls_back_to_failed_node_output() {
        let run = TaskStatusResponse {
            task: crate::client::TaskRecord {
                id: Uuid::new_v4(),
                title: "Broken run".into(),
                source_repo: "/tmp/demo".into(),
                status: TaskLifecycleStatus::Failed,
                workflow_id: None,
                task_description: None,
            },
            workflow: None,
            workspace_path: Some("/tmp/demo".into()),
            remote_target: None,
            nodes: vec![crate::client::TaskNodeStatus {
                id: "node-1".into(),
                node_type: "task".into(),
                primitive: None,
                category: Some(WorkflowNodeTypeCategory::Task),
                label: "Build".into(),
                status: TaskLifecycleStatus::Failed,
                output: Some("missing green background".into()),
                session_id: None,
                artifacts: Default::default(),
            }],
            outcome: None,
            agent: None,
            lease: None,
            validation: None,
            integration: None,
            failure_message: None,
            agents: None,
        };

        assert_eq!(
            run_failure_message(&run).as_deref(),
            Some("Build failed: missing green background")
        );
    }

    #[test]
    fn test_running_status_badge_uses_visible_chip_colors() {
        let (task_fill, _) = node_fill_and_border(
            &crate::client::WorkflowNodePrimitive::Llm,
            gpui::WindowAppearance::Dark,
        );
        let badge_fill = status_badge_color(&TaskLifecycleStatus::Running);
        let badge_text = status_badge_text_color();

        assert_ne!(badge_fill, task_fill);
        assert_ne!(badge_text, badge_fill);
    }

    #[test]
    fn test_try_open_run_node_in_center_skips_blank_session_id() {
        assert_eq!(normalized_session_id("   "), None);
    }

    #[test]
    fn test_try_open_run_node_in_center_passes_trimmed_session_id_to_opener() {
        assert_eq!(normalized_session_id(" session-1 "), Some("session-1"));
    }

    #[test]
    fn test_conversation_work_dirs_uses_workspace_path() {
        let work_dirs = conversation_work_dirs(Some(" /workspaces/task-1/workspace "));

        assert_eq!(
            work_dirs.unwrap().ordered_paths().collect::<Vec<_>>(),
            vec![&std::path::PathBuf::from("/workspaces/task-1/workspace")]
        );
    }

    #[test]
    fn test_run_node_remote_attachment_builds_docker_connection_options() {
        let work_dirs = PathList::new(&[PathBuf::from("/workspaces/demo/runtime-task")]);
        let attachment = run_node_remote_attachment(
            Some(&TaskRemoteTarget::Docker(
                crate::client::TaskDockerRemoteTarget {
                    name: "runtime-dev-container".to_string(),
                    container_id: "runtime-dev-container".to_string(),
                    remote_user: "root".to_string(),
                    use_podman: false,
                },
            )),
            Some(&work_dirs),
        )
        .expect("docker attachment");

        assert_eq!(
            attachment.paths,
            vec![PathBuf::from("/workspaces/demo/runtime-task")]
        );
        assert!(matches!(
            attachment.connection_options,
            RemoteConnectionOptions::Docker(DockerConnectionOptions {
                container_id,
                remote_user,
                ..
            }) if container_id == "runtime-dev-container" && remote_user == "root"
        ));
    }

    #[test]
    fn test_canvas_nodes_are_sized_for_visible_port_contracts() {
        assert!(NODE_WIDTH_F >= 280.0);
        assert!(NODE_HEIGHT_F >= 120.0);
    }

    #[test]
    fn test_port_canvas_position_reserves_vertical_room_for_contract_labels() {
        let pos = NodePos { x: 0.0, y: 0.0 };
        let first = port_canvas_position(&pos, true, 0, 3);
        let second = port_canvas_position(&pos, true, 1, 3);

        assert!(first.1 >= 48.0);
        assert!(second.1 - first.1 >= 24.0);
    }

    #[test]
    fn test_output_port_labels_are_positioned_inside_the_node() {
        let pos = NodePos { x: 40.0, y: 40.0 };
        let label_width = 64.0;
        let label_x = port_label_canvas_x(&pos, false, label_width);

        assert!(label_x >= pos.x);
        assert!(label_x + label_width <= pos.x + NODE_WIDTH_F - PORT_LABEL_X_INSET_F + 0.1);
    }

    #[test]
    fn test_apply_canvas_zoom_at_position_updates_zoom_and_viewport_offset() {
        let mut layout = CanvasLayout::default();
        let origin = gpui::point(px(0.0), px(0.0));
        let position = gpui::point(px(200.0), px(160.0));
        let canvas_before = to_canvas_point(&layout, position.x, position.y, origin);

        apply_canvas_zoom_at_position(&mut layout, origin, position, 1.5);
        let canvas_after = to_canvas_point(&layout, position.x, position.y, origin);

        assert_eq!(layout.zoom, 1.5);
        assert!((canvas_after.0 - canvas_before.0).abs() < 0.01);
        assert!((canvas_after.1 - canvas_before.1).abs() < 0.01);
    }

    #[test]
    fn test_effective_ports_for_conditional_node_use_configuration_branches() {
        let node_types = sample_node_types();
        let node = WorkflowNode {
            id: "conditional-1".into(),
            node_type: "validation".into(),
            label: "Conditional".into(),
            configuration: serde_json::json!({
                "branches": [
                    {
                        "output_id": "if_1",
                        "kind": "when",
                        "condition": {
                            "mode": "all",
                            "children": [{"kind": "predicate"}]
                        }
                    },
                    {
                        "output_id": "if_2",
                        "kind": "when",
                        "condition": {
                            "mode": "any",
                            "children": [{"kind": "predicate"}]
                        }
                    },
                    {
                        "output_id": "else",
                        "kind": "else"
                    }
                ]
            }),
            runtime: serde_json::json!({}),
        };

        let output_ports = effective_ports_for_node(&node_types, &node, false);

        assert_eq!(output_ports.len(), 3);
        assert_eq!(output_ports[0].id, "if_1");
        assert_eq!(output_ports[0].label, "If");
        assert_eq!(output_ports[1].label, "Else If 1");
        assert_eq!(output_ports[2].label, "Else");
    }

    #[test]
    fn test_effective_ports_for_globals_node_are_empty() {
        let node_types = sample_node_types();
        let node = WorkflowNode {
            id: "globals-1".into(),
            node_type: WORKFLOW_GLOBALS_NODE_TYPE_ID.into(),
            label: "Globals".into(),
            configuration: serde_json::json!({}),
            runtime: serde_json::json!({}),
        };

        assert!(effective_ports_for_node(&node_types, &node, true).is_empty());
        assert!(effective_ports_for_node(&node_types, &node, false).is_empty());
    }

    #[test]
    fn test_legacy_node_category_falls_back_to_primitive_palette() {
        assert_eq!(
            default_primitive_for_node("legacy-task", Some(&WorkflowNodeTypeCategory::Task),),
            crate::client::WorkflowNodePrimitive::Llm
        );
        assert_eq!(
            default_primitive_for_node(
                "legacy-integration",
                Some(&WorkflowNodeTypeCategory::Integration),
            ),
            crate::client::WorkflowNodePrimitive::ExecuteShellCommand
        );
        assert_eq!(
            default_primitive_for_node("conditional", None,),
            crate::client::WorkflowNodePrimitive::Conditional
        );
    }

    #[gpui::test]
    async fn test_canvas_matches_run_task_id(cx: &mut gpui::TestAppContext) {
        init_test(cx);

        let task_id = Uuid::new_v4();
        let other_task_id = Uuid::new_v4();
        let (canvas, cx) = cx.add_window_view(|_window, cx| {
            WorkflowCanvas::new_run(sample_run(task_id), WorkflowClient::new(), cx)
        });

        canvas.read_with(cx, |canvas, _cx| {
            assert!(canvas_matches_run_task_id(canvas, task_id));
            assert!(!canvas_matches_run_task_id(canvas, other_task_id));
        });
    }

    #[gpui::test]
    async fn test_mouse_select_emits_node_selected_event(cx: &mut gpui::TestAppContext) {
        use gpui::{Bounds, Modifiers, MouseButton, MouseDownEvent, size};

        init_test(cx);

        let selected_node_ids = Rc::new(RefCell::new(Vec::new()));
        let selected_node_ids_for_subscription = selected_node_ids.clone();
        let workflow = WorkflowDefinitionRecord {
            id: Uuid::nil(),
            name: "Workflow".into(),
            nodes: vec![WorkflowNode {
                id: "node-1".into(),
                node_type: "task".into(),
                label: "Task".into(),
                configuration: serde_json::json!({}),
                runtime: serde_json::json!({}),
            }],
            edges: vec![],
            node_policies: vec![],
            retry_behavior: crate::client::RetryBehavior::default(),
            validation_policy_ref: None,
            trigger_metadata: Default::default(),
        };

        let (canvas, cx) = cx.add_window_view(|_window, cx| {
            let canvas = WorkflowCanvas::new_edit(workflow, WorkflowClient::new(), cx);
            cx.subscribe(&cx.entity(), move |_, _, event, _| {
                if let WorkflowCanvasEvent::NodeSelected(node_id) = event {
                    selected_node_ids_for_subscription
                        .borrow_mut()
                        .push(node_id.clone());
                }
            })
            .detach();
            canvas
        });

        canvas.update_in(cx, |canvas, window, cx| {
            canvas.node_types = sample_node_types();
            canvas.canvas_bounds = Some(Bounds::new(
                gpui::point(px(0.0), px(0.0)),
                size(px(800.0), px(600.0)),
            ));
            canvas.handle_mouse_down(
                &MouseDownEvent {
                    position: gpui::point(px(60.0), px(60.0)),
                    button: MouseButton::Left,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                },
                window,
                cx,
            );
        });

        assert_eq!(
            selected_node_ids.borrow().as_slice(),
            &[Some("node-1".to_string())]
        );
    }

    #[gpui::test]
    async fn test_dragging_empty_canvas_pans_viewport(cx: &mut gpui::TestAppContext) {
        use gpui::{
            Bounds, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, size,
        };

        init_test(cx);

        let workflow = WorkflowDefinitionRecord {
            id: Uuid::nil(),
            name: "Workflow".into(),
            nodes: vec![],
            edges: vec![],
            node_policies: vec![],
            retry_behavior: crate::client::RetryBehavior::default(),
            validation_policy_ref: None,
            trigger_metadata: Default::default(),
        };

        let (canvas, cx) = cx.add_window_view(|_window, cx| {
            WorkflowCanvas::new_edit(workflow, WorkflowClient::new(), cx)
        });

        canvas.update_in(cx, |canvas, window, cx| {
            canvas.node_types = sample_node_types();
            canvas.canvas_bounds = Some(Bounds::new(
                gpui::point(px(0.0), px(0.0)),
                size(px(800.0), px(600.0)),
            ));
            canvas.handle_mouse_down(
                &MouseDownEvent {
                    position: gpui::point(px(300.0), px(300.0)),
                    button: MouseButton::Left,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                },
                window,
                cx,
            );
            canvas.handle_mouse_move(
                &MouseMoveEvent {
                    position: gpui::point(px(360.0), px(345.0)),
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Modifiers::default(),
                },
                window,
                cx,
            );
            canvas.handle_mouse_up(
                &MouseUpEvent {
                    button: MouseButton::Left,
                    position: gpui::point(px(360.0), px(345.0)),
                    modifiers: Modifiers::default(),
                    click_count: 1,
                },
                window,
                cx,
            );
            assert_eq!(canvas.layout.viewport_offset, (60.0, 45.0));
        });
    }

    #[gpui::test]
    async fn test_mouse_down_outside_canvas_does_not_start_pan(cx: &mut gpui::TestAppContext) {
        use gpui::{Bounds, Modifiers, MouseButton, MouseDownEvent, size};

        init_test(cx);

        let workflow = WorkflowDefinitionRecord {
            id: Uuid::nil(),
            name: "Workflow".into(),
            nodes: vec![],
            edges: vec![],
            node_policies: vec![],
            retry_behavior: crate::client::RetryBehavior::default(),
            validation_policy_ref: None,
            trigger_metadata: Default::default(),
        };

        let (canvas, cx) = cx.add_window_view(|_window, cx| {
            WorkflowCanvas::new_edit(workflow, WorkflowClient::new(), cx)
        });

        canvas.update_in(cx, |canvas, window, cx| {
            canvas.node_types = sample_node_types();
            canvas.canvas_bounds = Some(Bounds::new(
                gpui::point(px(0.0), px(40.0)),
                size(px(800.0), px(560.0)),
            ));
            canvas.handle_mouse_down(
                &MouseDownEvent {
                    position: gpui::point(px(50.0), px(20.0)),
                    button: MouseButton::Left,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                },
                window,
                cx,
            );

            assert!(canvas.pan_mouse_start.is_none());
            assert_eq!(canvas.layout.viewport_offset, (0.0, 0.0));
        });
    }

    #[gpui::test]
    async fn test_dragging_from_output_port_to_input_port_creates_edge(
        cx: &mut gpui::TestAppContext,
    ) {
        use gpui::{
            Bounds, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, size,
        };

        init_test(cx);

        let workflow = WorkflowDefinitionRecord {
            id: Uuid::nil(),
            name: "Workflow".into(),
            nodes: vec![
                WorkflowNode {
                    id: "node-1".into(),
                    node_type: "task".into(),
                    label: "Task".into(),
                    configuration: serde_json::json!({}),
                    runtime: serde_json::json!({}),
                },
                WorkflowNode {
                    id: "node-2".into(),
                    node_type: "validation".into(),
                    label: "Validation".into(),
                    configuration: serde_json::json!({}),
                    runtime: serde_json::json!({}),
                },
            ],
            edges: vec![],
            node_policies: vec![],
            retry_behavior: crate::client::RetryBehavior::default(),
            validation_policy_ref: None,
            trigger_metadata: Default::default(),
        };

        let (canvas, cx) = cx.add_window_view(|_window, cx| {
            WorkflowCanvas::new_edit(workflow, WorkflowClient::new(), cx)
        });

        canvas.update_in(cx, |canvas, window, cx| {
            canvas.node_types = sample_node_types();
            canvas.canvas_bounds = Some(Bounds::new(
                gpui::point(px(0.0), px(0.0)),
                size(px(800.0), px(600.0)),
            ));
            canvas
                .layout
                .node_positions
                .insert("node-1".into(), NodePos { x: 40.0, y: 40.0 });
            canvas
                .layout
                .node_positions
                .insert("node-2".into(), NodePos { x: 460.0, y: 40.0 });

            let output_port = port_canvas_position(
                canvas.layout.node_positions.get("node-1").unwrap(),
                false,
                0,
                1,
            );
            let input_port = port_canvas_position(
                canvas.layout.node_positions.get("node-2").unwrap(),
                true,
                0,
                1,
            );

            canvas.handle_mouse_down(
                &MouseDownEvent {
                    position: gpui::point(px(output_port.0), px(output_port.1)),
                    button: MouseButton::Left,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                },
                window,
                cx,
            );
            canvas.handle_mouse_move(
                &MouseMoveEvent {
                    position: gpui::point(px(input_port.0), px(input_port.1)),
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Modifiers::default(),
                },
                window,
                cx,
            );
            canvas.handle_mouse_up(
                &MouseUpEvent {
                    button: MouseButton::Left,
                    position: gpui::point(px(input_port.0), px(input_port.1)),
                    modifiers: Modifiers::default(),
                    click_count: 1,
                },
                window,
                cx,
            );

            let workflow = canvas.workflow.as_ref().unwrap();
            assert_eq!(workflow.edges.len(), 1);
            assert_eq!(workflow.edges[0].from_node_id, "node-1");
            assert_eq!(workflow.edges[0].from_output_id, "success");
            assert_eq!(workflow.edges[0].to_node_id, "node-2");
            assert_eq!(workflow.edges[0].to_input_id, "default");
        });
    }

    #[gpui::test]
    async fn test_dragging_run_node_selects_without_moving(cx: &mut gpui::TestAppContext) {
        use gpui::{
            Bounds, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, size,
        };

        init_test(cx);

        let task_id = Uuid::new_v4();
        let (canvas, cx) = cx.add_window_view(|_window, cx| {
            WorkflowCanvas::new_run(sample_run(task_id), WorkflowClient::new(), cx)
        });

        canvas.update_in(cx, |canvas, window, cx| {
            canvas.node_types = sample_node_types();
            canvas.canvas_bounds = Some(Bounds::new(
                gpui::point(px(0.0), px(0.0)),
                size(px(800.0), px(600.0)),
            ));
            canvas
                .layout
                .node_positions
                .insert("node-1".into(), NodePos { x: 40.0, y: 40.0 });

            canvas.handle_mouse_down(
                &MouseDownEvent {
                    position: gpui::point(px(60.0), px(60.0)),
                    button: MouseButton::Left,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                },
                window,
                cx,
            );
            canvas.handle_mouse_move(
                &MouseMoveEvent {
                    position: gpui::point(px(180.0), px(180.0)),
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Modifiers::default(),
                },
                window,
                cx,
            );
            canvas.handle_mouse_up(
                &MouseUpEvent {
                    button: MouseButton::Left,
                    position: gpui::point(px(180.0), px(180.0)),
                    modifiers: Modifiers::default(),
                    click_count: 1,
                },
                window,
                cx,
            );

            assert_eq!(canvas.selection, CanvasSelection::Node("node-1".into()));
            let position = canvas.layout.node_positions.get("node-1").unwrap();
            assert_eq!(position.x, 40.0);
            assert_eq!(position.y, 40.0);
        });
    }

    #[gpui::test]
    async fn test_dragging_between_ports_in_run_does_not_create_edge(
        cx: &mut gpui::TestAppContext,
    ) {
        use gpui::{
            Bounds, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, size,
        };

        init_test(cx);

        let task_id = Uuid::new_v4();
        let mut run = sample_run(task_id);
        if let Some(workflow) = run.workflow.as_mut() {
            workflow.nodes.push(WorkflowNode {
                id: "node-2".into(),
                node_type: "validation".into(),
                label: "Validation".into(),
                configuration: serde_json::json!({}),
                runtime: serde_json::json!({}),
            });
        }
        run.nodes.push(crate::client::TaskNodeStatus {
            id: "node-2".into(),
            node_type: "validation".into(),
            primitive: None,
            category: Some(WorkflowNodeTypeCategory::Validation),
            label: "Validation".into(),
            status: TaskLifecycleStatus::Queued,
            output: None,
            session_id: None,
            artifacts: Default::default(),
        });

        let (canvas, cx) = cx
            .add_window_view(|_window, cx| WorkflowCanvas::new_run(run, WorkflowClient::new(), cx));

        canvas.update_in(cx, |canvas, window, cx| {
            canvas.node_types = sample_node_types();
            canvas.canvas_bounds = Some(Bounds::new(
                gpui::point(px(0.0), px(0.0)),
                size(px(800.0), px(600.0)),
            ));
            canvas
                .layout
                .node_positions
                .insert("node-1".into(), NodePos { x: 40.0, y: 40.0 });
            canvas
                .layout
                .node_positions
                .insert("node-2".into(), NodePos { x: 460.0, y: 40.0 });

            let output_port = port_canvas_position(
                canvas.layout.node_positions.get("node-1").unwrap(),
                false,
                0,
                1,
            );
            let input_port = port_canvas_position(
                canvas.layout.node_positions.get("node-2").unwrap(),
                true,
                0,
                1,
            );

            canvas.handle_mouse_down(
                &MouseDownEvent {
                    position: gpui::point(px(output_port.0), px(output_port.1)),
                    button: MouseButton::Left,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                },
                window,
                cx,
            );
            canvas.handle_mouse_move(
                &MouseMoveEvent {
                    position: gpui::point(px(input_port.0), px(input_port.1)),
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Modifiers::default(),
                },
                window,
                cx,
            );
            canvas.handle_mouse_up(
                &MouseUpEvent {
                    button: MouseButton::Left,
                    position: gpui::point(px(input_port.0), px(input_port.1)),
                    modifiers: Modifiers::default(),
                    click_count: 1,
                },
                window,
                cx,
            );

            let workflow = canvas.workflow.as_ref().unwrap();
            assert!(workflow.edges.is_empty());
        });
    }

    #[gpui::test]
    async fn test_backspace_deletes_selected_node_and_connected_edges(
        cx: &mut gpui::TestAppContext,
    ) {
        init_test(cx);

        let workflow = WorkflowDefinitionRecord {
            id: Uuid::nil(),
            name: "Workflow".into(),
            nodes: vec![
                WorkflowNode {
                    id: "node-1".into(),
                    node_type: "task".into(),
                    label: "Task".into(),
                    configuration: serde_json::json!({}),
                    runtime: serde_json::json!({}),
                },
                WorkflowNode {
                    id: "node-2".into(),
                    node_type: "validation".into(),
                    label: "Validation".into(),
                    configuration: serde_json::json!({}),
                    runtime: serde_json::json!({}),
                },
            ],
            edges: vec![WorkflowEdge {
                from_node_id: "node-1".into(),
                from_output_id: "success".into(),
                to_node_id: "node-2".into(),
                to_input_id: "default".into(),
            }],
            node_policies: vec![],
            retry_behavior: crate::client::RetryBehavior::default(),
            validation_policy_ref: None,
            trigger_metadata: Default::default(),
        };

        let (canvas, cx) = cx.add_window_view(|_window, cx| {
            WorkflowCanvas::new_edit(workflow, WorkflowClient::new(), cx)
        });

        canvas.update_in(cx, |canvas, window, cx| {
            canvas.node_types = sample_node_types();
            canvas.selection = CanvasSelection::Node("node-1".into());
            canvas.handle_key_down(
                &gpui::KeyDownEvent {
                    keystroke: gpui::Keystroke::parse("backspace").unwrap(),
                    is_held: false,
                    prefer_character_input: false,
                },
                window,
                cx,
            );

            let workflow = canvas.workflow.as_ref().unwrap();
            assert_eq!(workflow.nodes.len(), 1);
            assert_eq!(workflow.nodes[0].id, "node-2");
            assert!(workflow.edges.is_empty());
            assert_eq!(canvas.selection, CanvasSelection::None);
        });
    }

    #[gpui::test]
    async fn test_delete_key_does_not_delete_selected_run_node(cx: &mut gpui::TestAppContext) {
        init_test(cx);

        let task_id = Uuid::new_v4();
        let (canvas, cx) = cx.add_window_view(|_window, cx| {
            WorkflowCanvas::new_run(sample_run(task_id), WorkflowClient::new(), cx)
        });

        canvas.update_in(cx, |canvas, window, cx| {
            canvas.node_types = sample_node_types();
            canvas.selection = CanvasSelection::Node("node-1".into());
            canvas.handle_key_down(
                &gpui::KeyDownEvent {
                    keystroke: gpui::Keystroke::parse("delete").unwrap(),
                    is_held: false,
                    prefer_character_input: false,
                },
                window,
                cx,
            );

            let workflow = canvas.workflow.as_ref().unwrap();
            assert_eq!(workflow.nodes.len(), 1);
            assert_eq!(workflow.nodes[0].id, "node-1");
            assert_eq!(canvas.selection, CanvasSelection::Node("node-1".into()));
        });
    }

    #[gpui::test]
    async fn test_right_clicking_node_selects_it_and_opens_context_menu(
        cx: &mut gpui::TestAppContext,
    ) {
        use gpui::{Bounds, Modifiers, MouseButton, MouseDownEvent, size};

        init_test(cx);

        let workflow = WorkflowDefinitionRecord {
            id: Uuid::nil(),
            name: "Workflow".into(),
            nodes: vec![WorkflowNode {
                id: "node-1".into(),
                node_type: "task".into(),
                label: "Task".into(),
                configuration: serde_json::json!({}),
                runtime: serde_json::json!({}),
            }],
            edges: vec![],
            node_policies: vec![],
            retry_behavior: crate::client::RetryBehavior::default(),
            validation_policy_ref: None,
            trigger_metadata: Default::default(),
        };

        let (canvas, cx) = cx.add_window_view(|_window, cx| {
            WorkflowCanvas::new_edit(workflow, WorkflowClient::new(), cx)
        });

        canvas.update_in(cx, |canvas, window, cx| {
            canvas.node_types = sample_node_types();
            canvas.canvas_bounds = Some(Bounds::new(
                gpui::point(px(0.0), px(0.0)),
                size(px(800.0), px(600.0)),
            ));
            canvas.handle_secondary_mouse_down(
                &MouseDownEvent {
                    position: gpui::point(px(60.0), px(60.0)),
                    button: MouseButton::Right,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                },
                window,
                cx,
            );

            assert_eq!(canvas.selection, CanvasSelection::Node("node-1".into()));
            assert!(canvas.context_menu.is_some());
        });
    }

    #[gpui::test]
    async fn test_right_clicking_edge_selects_it_and_delete_key_removes_it(
        cx: &mut gpui::TestAppContext,
    ) {
        use gpui::{Bounds, Modifiers, MouseButton, MouseDownEvent, size};

        init_test(cx);

        let workflow = WorkflowDefinitionRecord {
            id: Uuid::nil(),
            name: "Workflow".into(),
            nodes: vec![
                WorkflowNode {
                    id: "node-1".into(),
                    node_type: "task".into(),
                    label: "Task".into(),
                    configuration: serde_json::json!({}),
                    runtime: serde_json::json!({}),
                },
                WorkflowNode {
                    id: "node-2".into(),
                    node_type: "validation".into(),
                    label: "Validation".into(),
                    configuration: serde_json::json!({}),
                    runtime: serde_json::json!({}),
                },
            ],
            edges: vec![WorkflowEdge {
                from_node_id: "node-1".into(),
                from_output_id: "success".into(),
                to_node_id: "node-2".into(),
                to_input_id: "default".into(),
            }],
            node_policies: vec![],
            retry_behavior: crate::client::RetryBehavior::default(),
            validation_policy_ref: None,
            trigger_metadata: Default::default(),
        };

        let (canvas, cx) = cx.add_window_view(|_window, cx| {
            WorkflowCanvas::new_edit(workflow, WorkflowClient::new(), cx)
        });

        canvas.update_in(cx, |canvas, window, cx| {
            canvas.node_types = sample_node_types();
            canvas.canvas_bounds = Some(Bounds::new(
                gpui::point(px(0.0), px(0.0)),
                size(px(800.0), px(600.0)),
            ));
            canvas
                .layout
                .node_positions
                .insert("node-1".into(), NodePos { x: 40.0, y: 40.0 });
            canvas
                .layout
                .node_positions
                .insert("node-2".into(), NodePos { x: 520.0, y: 40.0 });

            let from = port_position_for_node(
                &canvas.layout,
                canvas.workflow.as_ref().unwrap(),
                &canvas.node_types,
                "node-1",
                "success",
                false,
            )
            .unwrap();
            let to = port_position_for_node(
                &canvas.layout,
                canvas.workflow.as_ref().unwrap(),
                &canvas.node_types,
                "node-2",
                "default",
                true,
            )
            .unwrap();
            let edge_midpoint = edge_canvas_midpoint(from, to);

            canvas.handle_secondary_mouse_down(
                &MouseDownEvent {
                    position: gpui::point(px(edge_midpoint.0), px(edge_midpoint.1)),
                    button: MouseButton::Right,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                },
                window,
                cx,
            );

            assert_eq!(
                canvas.selection,
                CanvasSelection::Edge(
                    "node-1".into(),
                    "success".into(),
                    "node-2".into(),
                    "default".into()
                )
            );
            assert!(canvas.context_menu.is_some());

            canvas.handle_key_down(
                &gpui::KeyDownEvent {
                    keystroke: gpui::Keystroke::parse("delete").unwrap(),
                    is_held: false,
                    prefer_character_input: false,
                },
                window,
                cx,
            );

            assert!(canvas.workflow.as_ref().unwrap().edges.is_empty());
            assert_eq!(canvas.selection, CanvasSelection::None);
        });
    }

    #[test]
    fn test_nodes_bounding_box() {
        let mut positions = HashMap::new();
        positions.insert("a".to_string(), NodePos { x: 0.0, y: 50.0 });
        positions.insert("b".to_string(), NodePos { x: 200.0, y: 100.0 });
        let (top, bottom) = nodes_bounding_box(&positions, 40.0);
        assert_eq!(top, 10.0);
        assert_eq!(bottom, 296.0);
    }

    #[test]
    fn test_backward_edge_rail_y() {
        assert_eq!(backward_edge_rail_y(10.0, 296.0, 0, true), 10.0);
        assert_eq!(backward_edge_rail_y(10.0, 296.0, 1, true), -2.0);
        assert_eq!(backward_edge_rail_y(10.0, 296.0, 0, false), 296.0);
        assert_eq!(backward_edge_rail_y(10.0, 296.0, 2, false), 320.0);
    }
}
