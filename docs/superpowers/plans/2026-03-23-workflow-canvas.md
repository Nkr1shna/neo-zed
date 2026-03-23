# Workflow Canvas — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement a GPUI `WorkspaceItem` that renders a directed workflow graph as an interactive canvas with zoom/pan, drag-to-move nodes, and two modes: edit (workflow definitions) and run (execution status with polling).

**Architecture:** `WorkflowCanvas` is an `Entity<T>` that implements `workspace::Item`, opening in the center pane like a file editor. All rendering uses GPUI's `canvas()` element with `PathBuilder` for edges and `paint_quad` for nodes. Zoom/pan are applied manually per-coordinate (no transform layer). Node positions persist to `~/.config/zed/workflow-layouts/{id}.json`.

**Tech Stack:** Rust, GPUI canvas API (PathBuilder, paint_quad, text_system), workspace::Item trait, reqwest (via WorkflowClient)

**Spec:** `docs/superpowers/specs/2026-03-23-workflow-canvas.md`

**Prerequisite:** Workstream 5 (workflow_ui crate with WorkflowClient) must be complete.

---

## File Map

| Action | Path | Responsibility |
|--------|------|----------------|
| Modify | `crates/workflow_ui/canvas.rs` | All canvas code |
| Modify | `crates/workflow_ui/workflow_ui.rs` | Export canvas types, register item |

---

### Task 1: Data structures and layout utilities

**Files:**
- Modify: `crates/workflow_ui/canvas.rs`

- [ ] **Step 1: Write failing test for auto-layout**

At the top of `canvas.rs` add a `#[cfg(test)]` block:

```rust
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
        // Single node should be positioned at the origin area
        assert!(pos.x >= 0.0);
        assert!(pos.y >= 0.0);
    }

    #[test]
    fn test_auto_layout_chain() {
        // a → b → c should produce three distinct x-columns
        let nodes = vec![
            WorkflowNode { id: "a".into(), kind: WorkflowNodeKind::Task, label: "A".into() },
            WorkflowNode { id: "b".into(), kind: WorkflowNodeKind::Validation, label: "B".into() },
            WorkflowNode { id: "c".into(), kind: WorkflowNodeKind::Integration, label: "C".into() },
        ];
        let edges = vec![
            WorkflowEdge { from: "a".into(), to: "b".into() },
            WorkflowEdge { from: "b".into(), to: "c".into() },
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
    fn test_task_lifecycle_status_terminal() {
        assert!(TaskLifecycleStatus::Completed.is_terminal());
        assert!(TaskLifecycleStatus::Failed.is_terminal());
        assert!(!TaskLifecycleStatus::Running.is_terminal());
        assert!(!TaskLifecycleStatus::Queued.is_terminal());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p workflow_ui 2>&1 | head -30
```

Expected: compile error (types not yet defined).

- [ ] **Step 3: Implement data structures in `canvas.rs`**

```rust
use crate::client::{
    TaskLifecycleStatus, TaskStatusResponse, WorkflowClient, WorkflowDefinitionRecord,
    WorkflowEdge, WorkflowNode, WorkflowNodeKind,
};
use gpui::{px, App, Context, Entity, FocusHandle, Pixels, Point, Task, WeakEntity, Window};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

const NODE_WIDTH_F: f32 = 200.0;
const NODE_HEIGHT_F: f32 = 72.0;
const NODE_H_GAP: f32 = 80.0;
const NODE_V_GAP: f32 = 60.0;
const EDGE_STROKE: Pixels = px(2.0);
const NODE_CORNER_RADIUS: Pixels = px(8.0);
const STATUS_DOT_RADIUS: Pixels = px(6.0);
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

/// Auto-layout: topological sort, assign columns left-to-right
pub fn auto_layout(
    nodes: &[WorkflowNode],
    edges: &[WorkflowEdge],
) -> HashMap<String, NodePos> {
    // Build adjacency and in-degree
    let mut in_degree: HashMap<&str, usize> = nodes.iter().map(|n| (n.id.as_str(), 0)).collect();
    let mut adj: HashMap<&str, Vec<&str>> = nodes.iter().map(|n| (n.id.as_str(), vec![])).collect();
    for e in edges {
        *in_degree.entry(e.to.as_str()).or_insert(0) += 1;
        adj.entry(e.from.as_str()).or_default().push(e.to.as_str());
    }

    // BFS from zero-in-degree nodes to assign depth (column)
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

    // Group nodes by depth column, assign row within column
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
```

- [ ] **Step 4: Run tests — should pass**

```bash
cargo test -p workflow_ui 2>&1 | head -30
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/workflow_ui/canvas.rs
git commit -m "workflow_ui: Add canvas data structures and auto-layout"
```

---

### Task 2: `WorkflowCanvas` struct and coordinate helpers

**Files:**
- Modify: `crates/workflow_ui/canvas.rs`

- [ ] **Step 1: Add coordinate transform tests**

```rust
#[test]
fn test_to_screen_no_transform() {
    // With offset (0,0) and zoom 1.0, canvas coords == screen coords relative to origin
    let layout = CanvasLayout::default();
    let origin = gpui::point(px(10.0), px(20.0));
    let screen = to_screen_point(&layout, 100.0, 50.0, origin);
    assert_eq!(screen.x, px(110.0));
    assert_eq!(screen.y, px(70.0));
}

#[test]
fn test_to_screen_with_zoom() {
    let layout = CanvasLayout { zoom: 2.0, viewport_offset: (0.0, 0.0), ..Default::default() };
    let origin = gpui::point(px(0.0), px(0.0));
    let screen = to_screen_point(&layout, 50.0, 25.0, origin);
    assert_eq!(screen.x, px(100.0));
    assert_eq!(screen.y, px(50.0));
}
```

- [ ] **Step 2: Run to verify fail**

```bash
cargo test -p workflow_ui 2>&1 | head -30
```

- [ ] **Step 3: Implement `WorkflowCanvas` struct and coordinate helpers**

```rust
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

/// Convert screen coordinates back to canvas space
pub fn to_canvas_point(
    layout: &CanvasLayout,
    screen_x: Pixels,
    screen_y: Pixels,
    origin: Point<Pixels>,
) -> (f32, f32) {
    let (ox, oy) = layout.viewport_offset;
    let z = layout.zoom;
    (
        (screen_x - origin.x).0 / z - ox,
        (screen_y - origin.y).0 / z - oy,
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
    on_node_selected: Option<Box<dyn Fn(Option<String>, &mut Window, &mut App)>>,
    on_node_activated: Option<Box<dyn Fn(String, &mut Window, &mut App)>>,
    _poll_task: Option<Task<()>>,
    client: Arc<WorkflowClient>,
}

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
            // Build synthetic nodes from node statuses for layout
            let nodes: Vec<WorkflowNode> = run.nodes.iter().map(|n| WorkflowNode {
                id: n.id.clone(),
                kind: n.kind.clone(),
                label: n.label.clone(),
            }).collect();
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
        };
        canvas.start_polling(task_id, cx);
        canvas
    }

    fn has_running_nodes(&self) -> bool {
        self.run.as_ref().map_or(false, |r| {
            r.nodes.iter().any(|n| n.status == TaskLifecycleStatus::Running)
        }) || self.run.as_ref().map_or(false, |r| {
            r.task.status == TaskLifecycleStatus::Running
        })
    }

    fn hit_test_node(&self, screen_pt: Point<Pixels>, canvas_origin: Point<Pixels>) -> Option<String> {
        let (cx, cy) = to_canvas_point(&self.layout, screen_pt.x, screen_pt.y, canvas_origin);
        for (id, pos) in &self.layout.node_positions {
            if cx >= pos.x && cx <= pos.x + NODE_WIDTH_F
                && cy >= pos.y && cy <= pos.y + NODE_HEIGHT_F
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
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p workflow_ui 2>&1 | head -30
```

Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/workflow_ui/canvas.rs
git commit -m "workflow_ui: Add WorkflowCanvas struct and coordinate helpers"
```

---

### Task 3: Node and edge painting

**Files:**
- Modify: `crates/workflow_ui/canvas.rs`

- [ ] **Step 1: Implement node color helper**

Add to `canvas.rs`:

```rust
fn node_fill_and_border(
    kind: &WorkflowNodeKind,
    appearance: gpui::WindowAppearance,
) -> (gpui::Rgba, gpui::Rgba) {
    use gpui::WindowAppearance::*;
    match (kind, appearance) {
        (WorkflowNodeKind::Task, Dark | VibrantDark) =>
            (gpui::rgba(0x1a3a5cff), gpui::rgba(0x3b82f6ff)),
        (WorkflowNodeKind::Task, _) =>
            (gpui::rgba(0xdbeafeff), gpui::rgba(0x3b82f6ff)),
        (WorkflowNodeKind::Validation, Dark | VibrantDark) =>
            (gpui::rgba(0x3a2e00ff), gpui::rgba(0xf59e0bff)),
        (WorkflowNodeKind::Validation, _) =>
            (gpui::rgba(0xfef3c7ff), gpui::rgba(0xf59e0bff)),
        (WorkflowNodeKind::Review, Dark | VibrantDark) =>
            (gpui::rgba(0x2d1a00ff), gpui::rgba(0xf97316ff)),
        (WorkflowNodeKind::Review, _) =>
            (gpui::rgba(0xffedd5ff), gpui::rgba(0xf97316ff)),
        (WorkflowNodeKind::Integration, Dark | VibrantDark) =>
            (gpui::rgba(0x0d2e1aff), gpui::rgba(0x22c55eff)),
        (WorkflowNodeKind::Integration, _) =>
            (gpui::rgba(0xdcfce7ff), gpui::rgba(0x22c55eff)),
    }
}

fn status_dot_color(status: &TaskLifecycleStatus) -> gpui::Rgba {
    match status {
        TaskLifecycleStatus::Queued    => gpui::rgba(0x6b7280ff),
        TaskLifecycleStatus::Running   => gpui::rgba(0x3b82f6ff),
        TaskLifecycleStatus::Completed => gpui::rgba(0x22c55eff),
        TaskLifecycleStatus::Failed    => gpui::rgba(0xef4444ff),
    }
}
```

- [ ] **Step 2: Implement static paint helpers (called from canvas closure)**

These are free functions (not methods) because `canvas()` closures own their captures:

```rust
pub fn paint_edge(
    layout: &CanvasLayout,
    from_pos: &NodePos,
    to_pos: &NodePos,
    origin: Point<Pixels>,
    window: &mut Window,
) {
    let src = to_screen_point(layout, from_pos.x + NODE_WIDTH_F, from_pos.y + NODE_HEIGHT_F / 2.0, origin);
    let dst = to_screen_point(layout, to_pos.x,                  to_pos.y   + NODE_HEIGHT_F / 2.0, origin);
    let cp1 = to_screen_point(layout, from_pos.x + NODE_WIDTH_F + 60.0, from_pos.y + NODE_HEIGHT_F / 2.0, origin);
    let cp2 = to_screen_point(layout, to_pos.x - 60.0,                  to_pos.y   + NODE_HEIGHT_F / 2.0, origin);

    let mut builder = gpui::PathBuilder::stroke(EDGE_STROKE);
    builder.move_to(src);
    // cubic_bezier_to(destination, control_a, control_b) — destination FIRST
    builder.cubic_bezier_to(dst, cp1, cp2);
    if let Ok(path) = builder.build() {
        window.paint_path(path, gpui::rgba(0x888888ccu32).into());
    }

    // Arrowhead: small filled triangle pointing right at dst
    paint_arrowhead(dst, layout.zoom, window);
}

fn paint_arrowhead(tip: Point<Pixels>, zoom: f32, window: &mut Window) {
    let size = px(8.0 * zoom);
    let mut builder = gpui::PathBuilder::fill();
    builder.move_to(tip);
    builder.line_to(gpui::point(tip.x - size, tip.y - size * px(0.5)));
    builder.line_to(gpui::point(tip.x - size, tip.y + size * px(0.5)));
    builder.close();
    if let Ok(path) = builder.build() {
        window.paint_path(path, gpui::rgba(0x888888ccу32).into());
    }
}

pub fn paint_node(
    layout: &CanvasLayout,
    node: &WorkflowNode,
    pos: &NodePos,
    selected: bool,
    status: Option<&TaskLifecycleStatus>,
    origin: Point<Pixels>,
    window: &mut Window,
    cx: &App,
) {
    use theme::ActiveTheme as _;

    let appearance = window.appearance();
    let (fill, border) = node_fill_and_border(&node.kind, appearance);
    let tl = to_screen_point(layout, pos.x, pos.y, origin);
    let bounds = gpui::Bounds {
        origin: tl,
        size: gpui::size(scaled(layout, NODE_WIDTH_F), scaled(layout, NODE_HEIGHT_F)),
    };
    let radii = gpui::Corners::all(scaled(layout, 8.0));
    let border_width = if selected { BORDER_WIDTH_SELECTED } else { BORDER_WIDTH_NORMAL };
    let border_color: gpui::Hsla = if selected {
        cx.theme().colors().border_focused
    } else {
        border.into()
    };

    window.paint_quad(gpui::quad(
        bounds,
        radii,
        fill.into(),
        gpui::Edges::all(border_width),
        border_color,
        gpui::BorderStyle::Solid,
    ));

    // Status dot (run mode)
    if let Some(s) = status {
        paint_status_dot(s, bounds, window);
    }

    // Label text
    paint_label(&node.label, bounds, window, cx);
}

fn paint_status_dot(status: &TaskLifecycleStatus, node_bounds: gpui::Bounds<Pixels>, window: &mut Window) {
    let color = status_dot_color(status);
    let center = gpui::point(
        node_bounds.origin.x + node_bounds.size.width - STATUS_DOT_RADIUS - px(6.0),
        node_bounds.origin.y + STATUS_DOT_RADIUS + px(6.0),
    );
    let mut builder = gpui::PathBuilder::fill();
    // Arc approximated by 4 cubic beziers — use gpui PathBuilder::arc if available,
    // otherwise fall back to fill circle via move_to + arc segments
    builder.arc(center, STATUS_DOT_RADIUS, 0.0, std::f32::consts::TAU);
    if let Ok(path) = builder.build() {
        window.paint_path(path, color.into());
    }
}

fn paint_label(label: &str, bounds: gpui::Bounds<Pixels>, window: &mut Window, cx: &App) {
    use gpui::{FontStyle, FontWeight, TextRun};
    use theme::ActiveTheme as _;

    let font_size = gpui::rems(0.875);
    let color = cx.theme().colors().text;
    let runs = [TextRun {
        len: label.len(),
        font: gpui::Font {
            family: theme::setup_ui_font(window, cx).family,
            features: Default::default(),
            fallbacks: Default::default(),
            weight: FontWeight::MEDIUM,
            style: FontStyle::Normal,
        },
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    }];
    let Ok(shaped) = window.text_system().shape_line(label.into(), font_size, &runs) else {
        return;
    };
    let line_height = font_size.to_pixels(window.rem_size());
    let x = bounds.origin.x + (bounds.size.width - shaped.width) / 2.0;
    let y = bounds.origin.y + (bounds.size.height - line_height) / 2.0;
    shaped.paint(gpui::point(x, y), line_height, window).ok();
}
```

- [ ] **Step 3: Verify compile**

```bash
cargo check -p workflow_ui 2>&1 | head -40
```

Fix any API mismatches. Key things to check:
- `gpui::quad` function signature (check `crates/gpui/src/window.rs`)
- `gpui::BorderStyle` variant name (may be `Solid` or `default()`)
- `PathBuilder::arc` existence (check `crates/gpui/src/path_builder.rs`)
- `ShapedLine::paint` signature

- [ ] **Step 4: Commit**

```bash
git add crates/workflow_ui/canvas.rs
git commit -m "workflow_ui: Add node and edge paint helpers"
```

---

### Task 4: `Render` implementation

**Files:**
- Modify: `crates/workflow_ui/canvas.rs`

- [ ] **Step 1: Implement `Render` for `WorkflowCanvas`**

```rust
use gpui::{IntoElement, Render};
use ui::{prelude::*, IconButton, IconName, IconSize};

impl Render for WorkflowCanvas {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.has_running_nodes() {
            window.request_animation_frame(cx);
            self.animation_phase = (self.animation_phase + 0.05) % 1.0;
        }

        let workflow = self.workflow.clone();
        let run = self.run.clone();
        let layout = self.layout.clone();
        let selection = self.selection.clone();
        let is_edit = self.run.is_none();

        div()
            .size_full()
            .relative()
            .bg(cx.theme().colors().editor_background)
            .when(is_edit, |this| this.child(self.render_toolbar(cx)))
            .child(
                gpui::canvas(
                    |bounds, _window, _cx| bounds,
                    move |bounds, _prepaint, window, cx| {
                        let origin = bounds.origin;

                        if let Some(ref wf) = workflow {
                            // Draw edges first (behind nodes)
                            for edge in &wf.edges {
                                if let (Some(fp), Some(tp)) = (
                                    layout.node_positions.get(&edge.from),
                                    layout.node_positions.get(&edge.to),
                                ) {
                                    paint_edge(&layout, fp, tp, origin, window);
                                }
                            }
                            // Draw nodes
                            for node in &wf.nodes {
                                let pos = layout.node_positions.get(&node.id)
                                    .copied()
                                    .unwrap_or(NodePos { x: 40.0, y: 40.0 });
                                let selected = matches!(&selection, CanvasSelection::Node(id) if *id == node.id);
                                paint_node(&layout, node, &pos, selected, None, origin, window, cx);
                            }
                        } else if let Some(ref run_data) = run {
                            // Run mode: draw edges from workflow if available
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
                            // Draw nodes with status
                            for node_status in &run_data.nodes {
                                let pos = layout.node_positions.get(&node_status.id)
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
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(Self::handle_mouse_down))
            .on_mouse_move(cx.listener(Self::handle_mouse_move))
            .on_mouse_up(gpui::MouseButton::Left, cx.listener(Self::handle_mouse_up))
            .on_scroll_wheel(cx.listener(Self::handle_scroll_wheel))
            .on_key_down(cx.listener(Self::handle_key_down))
    }
}
```

- [ ] **Step 2: Implement toolbar render**

```rust
impl WorkflowCanvas {
    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use ui::{IconButton, IconName, IconSize, Tooltip};
        let is_connect = self.mode == CanvasMode::Connect;

        div()
            .absolute()
            .top_2()
            .left_2()
            .z_index(10)
            .flex()
            .flex_row()
            .gap_1()
            .p_1()
            .rounded_md()
            .bg(cx.theme().colors().surface_background)
            .border_1()
            .border_color(cx.theme().colors().border)
            .child(
                IconButton::new("add-task", IconName::Plus)
                    .icon_size(IconSize::Small)
                    .tooltip(Tooltip::text("Add Task Node"))
                    .on_click(cx.listener(|this, _, _, cx| this.add_node(WorkflowNodeKind::Task, cx))),
            )
            .child(
                IconButton::new("connect", IconName::ArrowRight)
                    .icon_size(IconSize::Small)
                    .selected(is_connect)
                    .tooltip(Tooltip::text("Connect Nodes"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.mode = if this.mode == CanvasMode::Connect {
                            CanvasMode::Select
                        } else {
                            CanvasMode::Connect
                        };
                        cx.notify();
                    })),
            )
            .child(
                IconButton::new("fit", IconName::Maximize)
                    .icon_size(IconSize::Small)
                    .tooltip(Tooltip::text("Fit to screen"))
                    .on_click(cx.listener(|this, _, _, cx| { this.fit_to_screen(); cx.notify(); })),
            )
    }

    fn add_node(&mut self, kind: WorkflowNodeKind, cx: &mut Context<Self>) {
        let Some(ref mut workflow) = self.workflow else { return };
        let id = uuid::Uuid::new_v4().to_string();
        let (ox, oy) = self.layout.viewport_offset;
        // Position at viewport center
        let cx_coord = -ox + 300.0 / self.layout.zoom;
        let cy_coord = -oy + 200.0 / self.layout.zoom;
        workflow.nodes.push(WorkflowNode { id: id.clone(), kind, label: "New Node".into() });
        self.layout.node_positions.insert(id.clone(), NodePos { x: cx_coord, y: cy_coord });
        self.selection = CanvasSelection::Node(id.clone());
        cx.emit(WorkflowCanvasEvent::NodeSelected(Some(id)));
        cx.notify();
    }

    fn fit_to_screen(&mut self) {
        if self.layout.node_positions.is_empty() { return; }
        let min_x = self.layout.node_positions.values().map(|p| p.x).fold(f32::MAX, f32::min);
        let min_y = self.layout.node_positions.values().map(|p| p.y).fold(f32::MAX, f32::min);
        self.layout.viewport_offset = (-min_x + 40.0, -min_y + 40.0);
        self.layout.zoom = 1.0;
    }
}
```

- [ ] **Step 3: Implement input handlers**

```rust
impl WorkflowCanvas {
    fn handle_mouse_down(
        &mut self,
        event: &gpui::MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // We need the canvas bounds to do hit testing; use a stored bounds approach
        // For now use (0,0) as origin — in practice, bounds come from prepaint
        // The canvas element must store its bounds; see Task 5 for that refinement
        let origin = gpui::point(px(0.0), px(0.0)); // placeholder; see Task 5
        let pt = event.position;

        match self.mode {
            CanvasMode::Select => {
                if let Some(node_id) = self.hit_test_node(pt, origin) {
                    self.selection = CanvasSelection::Node(node_id.clone());
                    self.drag_node = Some(node_id.clone());
                    self.drag_mouse_start = Some(pt);
                    self.drag_node_start_pos = self.layout.node_positions.get(&node_id).copied();
                    cx.emit(WorkflowCanvasEvent::NodeSelected(Some(node_id)));
                } else {
                    self.selection = CanvasSelection::None;
                    self.pan_mouse_start = Some(pt);
                    self.pan_viewport_start = Some(self.layout.viewport_offset);
                    cx.emit(WorkflowCanvasEvent::NodeSelected(None));
                }
                cx.notify();
            }
            CanvasMode::Connect => {
                if let Some(node_id) = self.hit_test_node(pt, origin) {
                    if let Some(src) = self.connect_source.take() {
                        if src != node_id {
                            if let Some(ref mut wf) = self.workflow {
                                wf.edges.push(WorkflowEdge { from: src, to: node_id });
                            }
                        }
                        self.mode = CanvasMode::Select;
                    } else {
                        self.connect_source = Some(node_id);
                    }
                    cx.notify();
                }
            }
            CanvasMode::Pan => {
                self.pan_mouse_start = Some(pt);
                self.pan_viewport_start = Some(self.layout.viewport_offset);
            }
        }
        window.focus(&self.focus_handle);
    }

    fn handle_mouse_move(
        &mut self,
        event: &gpui::MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pt = event.position;
        if let (Some(drag_id), Some(start_mouse), Some(start_pos)) = (
            self.drag_node.clone(),
            self.drag_mouse_start,
            self.drag_node_start_pos,
        ) {
            let dx = (pt.x - start_mouse.x).0 / self.layout.zoom;
            let dy = (pt.y - start_mouse.y).0 / self.layout.zoom;
            self.layout.node_positions.insert(
                drag_id,
                NodePos { x: start_pos.x + dx, y: start_pos.y + dy },
            );
            cx.notify();
        } else if let (Some(pan_start), Some(vp_start)) =
            (self.pan_mouse_start, self.pan_viewport_start)
        {
            let dx = (pt.x - pan_start.x).0 / self.layout.zoom;
            let dy = (pt.y - pan_start.y).0 / self.layout.zoom;
            self.layout.viewport_offset = (vp_start.0 + dx, vp_start.1 + dy);
            cx.notify();
        }
    }

    fn handle_mouse_up(
        &mut self,
        _event: &gpui::MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_dragging = self.drag_node.is_some();
        self.drag_node = None;
        self.drag_node_start_pos = None;
        self.drag_mouse_start = None;
        self.pan_mouse_start = None;
        self.pan_viewport_start = None;

        if was_dragging {
            if let Some(ref workflow) = self.workflow {
                let workflow_id = workflow.id;
                let layout = self.layout.clone();
                self.save_layout_async(workflow_id, cx);
            }
        }
    }

    fn handle_scroll_wheel(
        &mut self,
        event: &gpui::ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = event.delta.pixel_delta(px(1.0));
        let zoom_factor = 1.0 - delta.y.0 * 0.001;
        let new_zoom = (self.layout.zoom * zoom_factor).clamp(0.25, 4.0);
        // Adjust offset to zoom toward cursor
        let cursor = event.position;
        let (ox, oy) = self.layout.viewport_offset;
        let z_old = self.layout.zoom;
        let z_new = new_zoom;
        // canvas_x = (screen_x / z_old) - ox  should equal  (screen_x / z_new) - new_ox
        // new_ox = (screen_x / z_new) - (screen_x / z_old) + ox
        let new_ox = cursor.x.0 / z_new - cursor.x.0 / z_old + ox;
        let new_oy = cursor.y.0 / z_new - cursor.y.0 / z_old + oy;
        self.layout.zoom = new_zoom;
        self.layout.viewport_offset = (new_ox, new_oy);
        cx.notify();
    }

    fn handle_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.run.is_some() { return; } // no editing in run mode
        match event.keystroke.key.as_str() {
            "delete" | "backspace" => {
                match self.selection.clone() {
                    CanvasSelection::Node(id) => {
                        if let Some(ref mut wf) = self.workflow {
                            wf.nodes.retain(|n| n.id != id);
                            wf.edges.retain(|e| e.from != id && e.to != id);
                            self.layout.node_positions.remove(&id);
                            self.selection = CanvasSelection::None;
                            cx.emit(WorkflowCanvasEvent::NodeSelected(None));
                            cx.notify();
                        }
                    }
                    CanvasSelection::Edge(from, to) => {
                        if let Some(ref mut wf) = self.workflow {
                            wf.edges.retain(|e| !(e.from == from && e.to == to));
                            self.selection = CanvasSelection::None;
                            cx.notify();
                        }
                    }
                    CanvasSelection::None => {}
                }
            }
            "escape" => {
                self.selection = CanvasSelection::None;
                self.connect_source = None;
                self.mode = CanvasMode::Select;
                cx.emit(WorkflowCanvasEvent::NodeSelected(None));
                cx.notify();
            }
            _ => {}
        }
    }
}
```

- [ ] **Step 4: Verify compile**

```bash
cargo check -p workflow_ui 2>&1 | head -40
```

- [ ] **Step 5: Commit**

```bash
git add crates/workflow_ui/canvas.rs
git commit -m "workflow_ui: Add canvas Render impl and input handlers"
```

---

### Task 5: Layout persistence and `workspace::Item` implementation

**Files:**
- Modify: `crates/workflow_ui/canvas.rs`
- Modify: `crates/workflow_ui/workflow_ui.rs`

- [ ] **Step 1: Implement layout persistence**

```rust
impl WorkflowCanvas {
    fn layout_path(workflow_id: Uuid) -> std::path::PathBuf {
        paths::support_dir()
            .join("workflow-layouts")
            .join(format!("{workflow_id}.json"))
    }

    fn save_layout_async(&self, workflow_id: Uuid, cx: &mut Context<Self>) {
        let layout = self.layout.clone();
        let json = match serde_json::to_string(&layout) {
            Ok(j) => j,
            Err(e) => { log::error!("workflow_ui: failed to serialize layout: {e}"); return; }
        };
        cx.background_spawn(async move {
            let path = Self::layout_path(workflow_id);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).log_err();
            }
            std::fs::write(&path, json).log_err();
        }).detach();
    }

    pub fn load_layout_async(workflow_id: Uuid, cx: &mut Context<Self>) -> gpui::Task<Option<CanvasLayout>> {
        cx.background_spawn(async move {
            let path = Self::layout_path(workflow_id);
            let json = std::fs::read_to_string(path).ok()?;
            serde_json::from_str(&json).ok()
        })
    }
}
```

- [ ] **Step 2: Implement `workspace::Item`**

```rust
use workspace::{Item, ItemEvent};

#[derive(Clone, Debug)]
pub enum WorkflowCanvasEvent {
    NodeSelected(Option<String>),
}

impl gpui::EventEmitter<WorkflowCanvasEvent> for WorkflowCanvas {}

impl gpui::Focusable for WorkflowCanvas {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Item for WorkflowCanvas {
    type Event = WorkflowCanvasEvent;

    fn tab_content_text(&self, _cx: &App) -> Option<gpui::SharedString> {
        let name = self
            .workflow
            .as_ref()
            .map(|w| w.name.as_str())
            .or_else(|| {
                self.run
                    .as_ref()
                    .and_then(|r| r.workflow.as_ref().map(|w| w.name.as_str()))
            })
            .unwrap_or("Workflow");
        let suffix = if self.run.is_some() { " (run)" } else { "" };
        Some(format!("{name}{suffix}").into())
    }

    fn to_item_events(event: &WorkflowCanvasEvent, mut f: impl FnMut(ItemEvent)) {
        match event {
            WorkflowCanvasEvent::NodeSelected(_) => f(ItemEvent::UpdateTab),
        }
    }
}
```

- [ ] **Step 3: Add `open_workflow` and `open_run` helper functions**

```rust
pub fn open_workflow(
    workflow: WorkflowDefinitionRecord,
    client: Arc<WorkflowClient>,
    workspace: &mut workspace::Workspace,
    window: &mut Window,
    cx: &mut Context<workspace::Workspace>,
) {
    let canvas = cx.new(|cx| WorkflowCanvas::new_edit(workflow, client, cx));
    workspace.add_item_to_center(Box::new(canvas), window, cx);
}

pub fn open_run(
    run: TaskStatusResponse,
    client: Arc<WorkflowClient>,
    workspace: &mut workspace::Workspace,
    window: &mut Window,
    cx: &mut Context<workspace::Workspace>,
) {
    let canvas = cx.new(|cx| WorkflowCanvas::new_run(run, client, cx));
    workspace.add_item_to_center(Box::new(canvas), window, cx);
}
```

- [ ] **Step 4: Export from `workflow_ui.rs`**

In `workflow_ui.rs`, update:
```rust
pub use canvas::{open_run, open_workflow, CanvasSelection, WorkflowCanvas, WorkflowCanvasEvent};
```

And add canvas item registration in `register`:
```rust
pub fn register(
    workspace: &mut Workspace,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<Workspace>,
) {
    workspace::register_item_type::<WorkflowCanvas>(cx);
    inspector::register(workspace, window, cx);
    runs::register(workspace, window, cx);
    canvas::register(workspace, window, cx);
}
```

- [ ] **Step 5: Verify compile**

```bash
cargo check -p workflow_ui 2>&1 | head -40
```

Check `workspace::register_item_type` exists; if not, search for the correct registration pattern:
```bash
grep -r "register_item_type\|deserialize_item\|register.*Item" crates/workspace/src/ | head -20
```

- [ ] **Step 6: Commit**

```bash
git add crates/workflow_ui/canvas.rs crates/workflow_ui/workflow_ui.rs
git commit -m "workflow_ui: Add layout persistence and workspace::Item impl for canvas"
```
