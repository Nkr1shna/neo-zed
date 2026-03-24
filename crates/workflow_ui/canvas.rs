use crate::client::{
    TaskLifecycleStatus, TaskNodeStatus, TaskStatusResponse, WorkflowClient,
    WorkflowDefinitionRecord, WorkflowEdge, WorkflowNode, WorkflowNodeKind,
};
use editor::Editor;
use gpui::{App, AppContext, Context, FocusHandle, Pixels, Point, Task, Window, px};
use multi_buffer::MultiBuffer;
use std::collections::HashMap;
use std::sync::Arc;
use util::ResultExt;
use uuid::Uuid;
use workspace::Workspace;

const NODE_WIDTH_F: f32 = 200.0;
const NODE_HEIGHT_F: f32 = 72.0;
const NODE_H_GAP: f32 = 80.0;
const NODE_V_GAP: f32 = 60.0;
const EDGE_STROKE: Pixels = px(2.0);
const NODE_CORNER_RADIUS: Pixels = px(8.0);
const STATUS_DOT_RADIUS: f32 = 6.0;
const BORDER_WIDTH_NORMAL: Pixels = px(1.5);
const BORDER_WIDTH_SELECTED: Pixels = px(3.0);

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
}

impl Default for CanvasLayout {
    fn default() -> Self {
        Self {
            node_positions: HashMap::new(),
            viewport_offset: (0.0, 0.0),
            zoom: 1.0,
        }
    }
}

impl Clone for CanvasLayout {
    fn clone(&self) -> Self {
        Self {
            node_positions: self.node_positions.clone(),
            viewport_offset: self.viewport_offset,
            zoom: self.zoom,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CanvasSelection {
    None,
    Node(String),
    Edge(String, String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum CanvasMode {
    Select,
    Connect,
    Pan,
}

pub enum WorkflowCanvasEvent {
    NodeSelected(Option<String>),
    NodeActivated(String),
}

pub fn auto_layout(nodes: &[WorkflowNode], edges: &[WorkflowEdge]) -> HashMap<String, NodePos> {
    let mut in_degree: HashMap<&str, usize> = nodes.iter().map(|n| (n.id.as_str(), 0)).collect();
    let mut adj: HashMap<&str, Vec<&str>> = nodes.iter().map(|n| (n.id.as_str(), vec![])).collect();
    for edge in edges {
        *in_degree.entry(edge.to.as_str()).or_insert(0) += 1;
        adj.entry(edge.from.as_str())
            .or_default()
            .push(edge.to.as_str());
    }
    let mut queue: std::collections::VecDeque<&str> = in_degree
        .iter()
        .filter_map(|(id, &deg)| if deg == 0 { Some(*id) } else { None })
        .collect();
    let mut depth: HashMap<&str, usize> = HashMap::new();
    while let Some(node) = queue.pop_front() {
        let d = depth.get(node).copied().unwrap_or(0);
        for &next in adj.get(node).unwrap_or(&vec![]) {
            let next_depth = d + 1;
            let entry = depth.entry(next).or_insert(0);
            if next_depth > *entry {
                *entry = next_depth;
            }
            queue.push_back(next);
        }
    }
    let mut columns: HashMap<usize, Vec<&str>> = HashMap::new();
    for node in nodes {
        let col = depth.get(node.id.as_str()).copied().unwrap_or(0);
        columns.entry(col).or_default().push(node.id.as_str());
    }
    let mut positions = HashMap::new();
    for (col, node_ids) in &columns {
        for (row, &node_id) in node_ids.iter().enumerate() {
            positions.insert(
                node_id.to_string(),
                NodePos {
                    x: *col as f32 * (NODE_WIDTH_F + NODE_H_GAP) + 40.0,
                    y: row as f32 * (NODE_HEIGHT_F + NODE_V_GAP) + 40.0,
                },
            );
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

pub struct WorkflowCanvas {
    pub workflow: Option<WorkflowDefinitionRecord>,
    pub run: Option<TaskStatusResponse>,
    pub layout: CanvasLayout,
    pub selection: CanvasSelection,
    pub mode: CanvasMode,
    connect_source: Option<String>,
    drag_node: Option<String>,
    drag_node_start_pos: Option<NodePos>,
    drag_mouse_start: Option<Point<Pixels>>,
    pan_mouse_start: Option<Point<Pixels>>,
    pan_viewport_start: Option<(f32, f32)>,
    animation_phase: f32,
    focus_handle: FocusHandle,
    pub on_node_selected: Option<Box<dyn Fn(Option<String>, &mut Window, &mut App)>>,
    pub on_node_activated: Option<Box<dyn Fn(String, &mut Window, &mut App)>>,
    _poll_task: Option<Task<()>>,
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
        Self {
            workflow: Some(workflow),
            run: None,
            layout,
            selection: CanvasSelection::None,
            mode: CanvasMode::Select,
            connect_source: None,
            drag_node: None,
            drag_node_start_pos: None,
            drag_mouse_start: None,
            pan_mouse_start: None,
            pan_viewport_start: None,
            animation_phase: 0.0,
            focus_handle: cx.focus_handle(),
            on_node_selected: None,
            on_node_activated: None,
            _poll_task: None,
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
                    kind: n.kind.clone(),
                    label: n.label.clone(),
                })
                .collect();
            layout.node_positions = auto_layout(&nodes, &[]);
        }
        let task_id = run.task.id;
        let mut canvas = Self {
            workflow: run.workflow.clone(),
            run: Some(run),
            layout,
            selection: CanvasSelection::None,
            mode: CanvasMode::Select,
            connect_source: None,
            drag_node: None,
            drag_node_start_pos: None,
            drag_mouse_start: None,
            pan_mouse_start: None,
            pan_viewport_start: None,
            animation_phase: 0.0,
            focus_handle: cx.focus_handle(),
            on_node_selected: None,
            on_node_activated: None,
            _poll_task: None,
            client,
            canvas_bounds: None,
        };
        canvas.start_polling(task_id, cx);
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

    fn hit_test_node(
        &self,
        screen_pt: Point<Pixels>,
        canvas_origin: Point<Pixels>,
    ) -> Option<String> {
        let (cx_coord, cy_coord) =
            to_canvas_point(&self.layout, screen_pt.x, screen_pt.y, canvas_origin);
        for (id, pos) in &self.layout.node_positions {
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
                let is_terminal = status.task.status.is_terminal();
                this.update(cx, |canvas, cx| {
                    canvas.run = Some(status);
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

    fn handle_mouse_down(
        &mut self,
        event: &gpui::MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let origin = self.canvas_origin();
        let position = event.position;

        match self.mode {
            CanvasMode::Pan => {
                self.pan_mouse_start = Some(position);
                self.pan_viewport_start = Some(self.layout.viewport_offset);
            }
            CanvasMode::Select => {
                if let Some(node_id) = self.hit_test_node(position, origin) {
                    if event.click_count == 2 {
                        if let Some(ref callback) = self.on_node_activated {
                            callback(node_id.clone(), window, cx);
                        }
                        cx.emit(WorkflowCanvasEvent::NodeActivated(node_id));
                    } else {
                        self.drag_node = Some(node_id.clone());
                        self.drag_mouse_start = Some(position);
                        self.drag_node_start_pos =
                            self.layout.node_positions.get(&node_id).copied();
                        self.selection = CanvasSelection::Node(node_id.clone());
                        if let Some(ref callback) = self.on_node_selected {
                            callback(Some(node_id), window, cx);
                        }
                    }
                } else {
                    self.selection = CanvasSelection::None;
                    if let Some(ref callback) = self.on_node_selected {
                        callback(None, window, cx);
                    }
                }
                cx.notify();
            }
            CanvasMode::Connect => {
                if let Some(node_id) = self.hit_test_node(position, origin) {
                    if self.connect_source.is_none() {
                        self.connect_source = Some(node_id);
                    } else {
                        self.connect_source = None;
                    }
                    cx.notify();
                }
            }
        }
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
    }

    fn handle_mouse_up(
        &mut self,
        _event: &gpui::MouseUpEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.drag_node = None;
        self.drag_mouse_start = None;
        self.drag_node_start_pos = None;
        self.pan_mouse_start = None;
        self.pan_viewport_start = None;
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
            let new_zoom = (self.layout.zoom * zoom_delta).clamp(0.1, 5.0);
            let mouse_canvas_x = (event.position.x - origin.x).as_f32() / self.layout.zoom
                - self.layout.viewport_offset.0;
            let mouse_canvas_y = (event.position.y - origin.y).as_f32() / self.layout.zoom
                - self.layout.viewport_offset.1;
            self.layout.zoom = new_zoom;
            self.layout.viewport_offset = (
                (event.position.x - origin.x).as_f32() / new_zoom - mouse_canvas_x,
                (event.position.y - origin.y).as_f32() / new_zoom - mouse_canvas_y,
            );
        } else {
            let z = self.layout.zoom;
            self.layout.viewport_offset.0 += delta.x.as_f32() / z;
            self.layout.viewport_offset.1 += delta.y.as_f32() / z;
        }
        cx.notify();
    }

    fn handle_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.keystroke.key.as_str() {
            "escape" => {
                self.selection = CanvasSelection::None;
                self.connect_source = None;
                self.mode = CanvasMode::Select;
                cx.notify();
            }
            "s" => {
                self.mode = CanvasMode::Select;
                cx.notify();
            }
            "c" => {
                self.mode = CanvasMode::Connect;
                cx.notify();
            }
            "p" => {
                self.mode = CanvasMode::Pan;
                cx.notify();
            }
            _ => {}
        }
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        use ui::prelude::*;
        h_flex()
            .gap_2()
            .p_2()
            .child(
                Button::new("mode-select", "Select")
                    .style(if self.mode == CanvasMode::Select {
                        ButtonStyle::Filled
                    } else {
                        ButtonStyle::Subtle
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.mode = CanvasMode::Select;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("mode-connect", "Connect")
                    .style(if self.mode == CanvasMode::Connect {
                        ButtonStyle::Filled
                    } else {
                        ButtonStyle::Subtle
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.mode = CanvasMode::Connect;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("mode-pan", "Pan")
                    .style(if self.mode == CanvasMode::Pan {
                        ButtonStyle::Filled
                    } else {
                        ButtonStyle::Subtle
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.mode = CanvasMode::Pan;
                        cx.notify();
                    })),
            )
    }
}

impl gpui::Focusable for WorkflowCanvas {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn node_fill_and_border(
    kind: &WorkflowNodeKind,
    appearance: gpui::WindowAppearance,
) -> (gpui::Rgba, gpui::Rgba) {
    use gpui::WindowAppearance::*;
    match (kind, appearance) {
        (WorkflowNodeKind::Task, Dark | VibrantDark) => {
            (gpui::rgba(0x1a3a5cff), gpui::rgba(0x3b82f6ff))
        }
        (WorkflowNodeKind::Task, _) => (gpui::rgba(0xdbeafeff), gpui::rgba(0x3b82f6ff)),
        (WorkflowNodeKind::Validation, Dark | VibrantDark) => {
            (gpui::rgba(0x3a2e00ff), gpui::rgba(0xf59e0bff))
        }
        (WorkflowNodeKind::Validation, _) => (gpui::rgba(0xfef3c7ff), gpui::rgba(0xf59e0bff)),
        (WorkflowNodeKind::Review, Dark | VibrantDark) => {
            (gpui::rgba(0x2d1a00ff), gpui::rgba(0xf97316ff))
        }
        (WorkflowNodeKind::Review, _) => (gpui::rgba(0xffedd5ff), gpui::rgba(0xf97316ff)),
        (WorkflowNodeKind::Integration, Dark | VibrantDark) => {
            (gpui::rgba(0x0d2e1aff), gpui::rgba(0x22c55eff))
        }
        (WorkflowNodeKind::Integration, _) => (gpui::rgba(0xdcfce7ff), gpui::rgba(0x22c55eff)),
    }
}

fn status_dot_color(status: &TaskLifecycleStatus) -> gpui::Rgba {
    match status {
        TaskLifecycleStatus::Queued => gpui::rgba(0x6b7280ff),
        TaskLifecycleStatus::Running => gpui::rgba(0x3b82f6ff),
        TaskLifecycleStatus::Completed => gpui::rgba(0x22c55eff),
        TaskLifecycleStatus::Failed => gpui::rgba(0xef4444ff),
    }
}

fn paint_node(
    layout: &CanvasLayout,
    node: &WorkflowNode,
    pos: &NodePos,
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
    let (fill_color, border_color) = node_fill_and_border(&node.kind, appearance);

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
        paint_status_dot(layout, pos, status, origin, window);
    }

    paint_label(layout, node, pos, origin, window, cx);
}

fn paint_status_dot(
    layout: &CanvasLayout,
    pos: &NodePos,
    status: &TaskLifecycleStatus,
    origin: Point<Pixels>,
    window: &mut Window,
) {
    let dot_radius = STATUS_DOT_RADIUS * layout.zoom;
    let center_x = pos.x + NODE_WIDTH_F - STATUS_DOT_RADIUS - 8.0;
    let center_y = pos.y + STATUS_DOT_RADIUS + 8.0;
    let center = to_screen_point(layout, center_x, center_y, origin);

    let color = status_dot_color(status);
    let kappa = 0.5523_f32;
    let r = dot_radius;

    let mut builder = gpui::PathBuilder::fill();
    builder.move_to(gpui::point(center.x, center.y - px(r)));
    builder.cubic_bezier_to(
        gpui::point(center.x + px(r), center.y),
        gpui::point(center.x + px(r * kappa), center.y - px(r)),
        gpui::point(center.x + px(r), center.y - px(r * kappa)),
    );
    builder.cubic_bezier_to(
        gpui::point(center.x, center.y + px(r)),
        gpui::point(center.x + px(r), center.y + px(r * kappa)),
        gpui::point(center.x + px(r * kappa), center.y + px(r)),
    );
    builder.cubic_bezier_to(
        gpui::point(center.x - px(r), center.y),
        gpui::point(center.x - px(r * kappa), center.y + px(r)),
        gpui::point(center.x - px(r), center.y + px(r * kappa)),
    );
    builder.cubic_bezier_to(
        gpui::point(center.x, center.y - px(r)),
        gpui::point(center.x - px(r), center.y - px(r * kappa)),
        gpui::point(center.x - px(r * kappa), center.y - px(r)),
    );
    builder.close();

    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

fn paint_label(
    layout: &CanvasLayout,
    node: &WorkflowNode,
    pos: &NodePos,
    origin: Point<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let font_size = scaled(layout, 13.0);
    let label_text: gpui::SharedString = node.label.clone().into();
    let text_color = gpui::Hsla {
        h: 0.0,
        s: 0.0,
        l: 0.15,
        a: 1.0,
    };
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
    let label_x = pos.x + 12.0;
    let label_y = pos.y + NODE_HEIGHT_F / 2.0 - 8.0;
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

    let kind_text: gpui::SharedString = node.kind.display_name().into();
    let kind_color = gpui::Hsla {
        h: 0.0,
        s: 0.0,
        l: 0.45,
        a: 1.0,
    };
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
    let kind_y = pos.y + NODE_HEIGHT_F / 2.0 + 8.0;
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

fn paint_edge(
    layout: &CanvasLayout,
    from: &NodePos,
    to: &NodePos,
    origin: Point<Pixels>,
    window: &mut Window,
) {
    let from_x = from.x + NODE_WIDTH_F;
    let from_y = from.y + NODE_HEIGHT_F / 2.0;
    let to_x = to.x;
    let to_y = to.y + NODE_HEIGHT_F / 2.0;

    let from_pt = to_screen_point(layout, from_x, from_y, origin);
    let to_pt = to_screen_point(layout, to_x, to_y, origin);

    let ctrl_offset = scaled(layout, 60.0);
    let ctrl_a = gpui::point(from_pt.x + ctrl_offset, from_pt.y);
    let ctrl_b = gpui::point(to_pt.x - ctrl_offset, to_pt.y);

    let stroke_width = scaled(layout, EDGE_STROKE.as_f32());
    let edge_color = gpui::rgba(0x9ca3afff);

    let mut builder = gpui::PathBuilder::stroke(stroke_width);
    builder.move_to(from_pt);
    builder.cubic_bezier_to(to_pt, ctrl_a, ctrl_b);
    if let Ok(path) = builder.build() {
        window.paint_path(path, edge_color);
    }

    paint_arrowhead(layout, to_x, to_y, origin, window);
}

fn paint_arrowhead(
    layout: &CanvasLayout,
    tip_x: f32,
    tip_y: f32,
    origin: Point<Pixels>,
    window: &mut Window,
) {
    let tip = to_screen_point(layout, tip_x, tip_y, origin);
    let size = layout.zoom * 10.0;
    let half = size * 0.5;

    let p1 = gpui::point(tip.x, tip.y);
    let p2 = gpui::point(tip.x - px(size), tip.y - px(half));
    let p3 = gpui::point(tip.x - px(size), tip.y + px(half));

    let arrow_color = gpui::rgba(0x9ca3afff);
    let mut builder = gpui::PathBuilder::fill();
    builder.move_to(p1);
    builder.line_to(p2);
    builder.line_to(p3);
    builder.close();
    if let Ok(path) = builder.build() {
        window.paint_path(path, arrow_color);
    }
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
        let layout = self.layout.clone();
        let selection = self.selection.clone();
        let is_edit = self.run.is_none();
        let this_weak = cx.weak_entity();

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
                            for edge in &wf.edges {
                                if let (Some(fp), Some(tp)) = (
                                    layout.node_positions.get(&edge.from),
                                    layout.node_positions.get(&edge.to),
                                ) {
                                    paint_edge(&layout, fp, tp, origin, window);
                                }
                            }
                            for node in &wf.nodes {
                                let pos = layout
                                    .node_positions
                                    .get(&node.id)
                                    .copied()
                                    .unwrap_or(NodePos { x: 40.0, y: 40.0 });
                                let selected = matches!(&selection, CanvasSelection::Node(id) if *id == node.id);
                                paint_node(
                                    &layout, node, &pos, selected, None, origin, window, cx,
                                );
                            }
                        } else if let Some(ref run_data) = run {
                            if let Some(ref wf) = run_data.workflow {
                                for edge in &wf.edges {
                                    if let (Some(fp), Some(tp)) = (
                                        layout.node_positions.get(&edge.from),
                                        layout.node_positions.get(&edge.to),
                                    ) {
                                        paint_edge(&layout, fp, tp, origin, window);
                                    }
                                }
                            }
                            for node_status in &run_data.nodes {
                                let pos = layout
                                    .node_positions
                                    .get(&node_status.id)
                                    .copied()
                                    .unwrap_or(NodePos { x: 40.0, y: 40.0 });
                                let synthetic_node = WorkflowNode {
                                    id: node_status.id.clone(),
                                    kind: node_status.kind.clone(),
                                    label: node_status.label.clone(),
                                };
                                paint_node(
                                    &layout,
                                    &synthetic_node,
                                    &pos,
                                    false,
                                    Some(&node_status.status),
                                    origin,
                                    window,
                                    cx,
                                );
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
            .on_mouse_move(cx.listener(Self::handle_mouse_move))
            .on_mouse_up(gpui::MouseButton::Left, cx.listener(Self::handle_mouse_up))
            .on_scroll_wheel(cx.listener(Self::handle_scroll_wheel))
            .on_key_down(cx.listener(Self::handle_key_down))
    }
}

pub fn open_workflow(
    workflow: WorkflowDefinitionRecord,
    client: Arc<WorkflowClient>,
    workspace: &mut workspace::Workspace,
    window: &mut Window,
    cx: &mut gpui::Context<workspace::Workspace>,
) {
    let canvas = cx.new(|cx| WorkflowCanvas::new_edit(workflow, client, cx));
    workspace.add_item_to_center(Box::new(canvas), window, cx);
}

pub fn open_run(
    run: TaskStatusResponse,
    client: Arc<WorkflowClient>,
    workspace: &mut workspace::Workspace,
    window: &mut Window,
    cx: &mut gpui::Context<workspace::Workspace>,
) {
    let canvas = cx.new(|cx| WorkflowCanvas::new_run(run, client, cx));
    let canvas_handle = canvas.downgrade();
    let workspace_handle = workspace.weak_handle();
    canvas.update(cx, |canvas, _cx| {
        canvas.on_node_activated = Some(Box::new(move |node_id, window, cx| {
            open_run_node_conversation(
                canvas_handle.clone(),
                workspace_handle.clone(),
                node_id,
                window,
                cx,
            );
        }));
    });
    workspace.add_item_to_center(Box::new(canvas), window, cx);
}

fn open_run_node_conversation(
    canvas: gpui::WeakEntity<WorkflowCanvas>,
    workspace: gpui::WeakEntity<Workspace>,
    node_id: String,
    window: &mut Window,
    cx: &mut App,
) {
    let Ok(Some((run_title, node))) = canvas.read_with(cx, |canvas, _cx| {
        let run = canvas.run.as_ref()?;
        let node = run.nodes.iter().find(|node| node.id == node_id)?.clone();
        Some((run.task.title.clone(), node))
    }) else {
        return;
    };

    open_node_conversation(node, run_title, workspace, window, cx);
}

fn open_node_conversation(
    node: TaskNodeStatus,
    run_title: String,
    workspace: gpui::WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(workspace) = workspace.upgrade() else {
        return;
    };
    let markdown_language = workspace
        .read(cx)
        .app_state()
        .languages
        .language_for_name("Markdown");
    let project = workspace.read(cx).project().clone();
    let title = format!("{run_title} / {} — conversation", node.label);
    let content = format!(
        "# {} — {}\n\n**Status:** {}\n\n---\n\n{}",
        run_title,
        node.label,
        node.status.display_name(),
        node.output.as_deref().unwrap_or("*(No output yet)*"),
    );

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

    #[test]
    fn test_auto_layout_single_node() {
        let nodes = vec![WorkflowNode {
            id: "a".into(),
            kind: WorkflowNodeKind::Task,
            label: "A".into(),
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
                kind: WorkflowNodeKind::Task,
                label: "A".into(),
            },
            WorkflowNode {
                id: "b".into(),
                kind: WorkflowNodeKind::Validation,
                label: "B".into(),
            },
            WorkflowNode {
                id: "c".into(),
                kind: WorkflowNodeKind::Integration,
                label: "C".into(),
            },
        ];
        let edges = vec![
            WorkflowEdge {
                from: "a".into(),
                to: "b".into(),
            },
            WorkflowEdge {
                from: "b".into(),
                to: "c".into(),
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
}
