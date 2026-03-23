# Workflow Canvas

**Date:** 2026-03-23
**Workstream:** 2 of 5 — GPUI graph canvas (most critical)
**New crate:** `crates/workflow_ui` (canvas module)
**Depends on:** Workstream 5 (HTTP client models)

---

## Goal

Implement a GPUI `WorkspaceItem` that renders a directed workflow graph (nodes + edges)
using the GPUI `canvas()` element and `PathBuilder`. Supports two modes: **edit**
(workflow definitions) and **run** (execution status).

---

## Data structures

```rust
use gpui::{px, Pixels, Point};
use std::collections::HashMap;
use uuid::Uuid;

/// Position of a node on the canvas (canvas-space coordinates)
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct NodePos {
    pub x: f32,
    pub y: f32,
}

/// Local canvas layout — persisted per workflow
#[derive(serde::Serialize, serde::Deserialize)]
pub struct CanvasLayout {
    pub node_positions: HashMap<String, NodePos>,
    pub viewport_offset: (f32, f32),
    pub zoom: f32,  // 1.0 = 100%, range 0.25–4.0
}

/// Manual Default impl — zoom must be 1.0, not 0.0
impl Default for CanvasLayout {
    fn default() -> Self {
        Self {
            node_positions: HashMap::new(),
            viewport_offset: (0.0, 0.0),
            zoom: 1.0,
        }
    }
}

/// Which element is selected
#[derive(Clone, Debug, PartialEq)]
pub enum CanvasSelection {
    None,
    Node(String),
    Edge(String, String),  // from_id, to_id
}

/// Canvas interaction mode
#[derive(Clone, Debug, PartialEq)]
pub enum CanvasMode {
    Select,
    Connect,  // click source then target to create an edge
    Pan,      // space + drag
}
```

---

## Visual constants

All pixel-dimension constants must be typed as `Pixels` using `px()`:

```rust
use gpui::px;

const NODE_WIDTH: gpui::Pixels = px(200.0);
const NODE_HEIGHT: gpui::Pixels = px(72.0);
const NODE_RADIUS: gpui::Pixels = px(8.0);
const EDGE_STROKE: gpui::Pixels = px(2.0);
const ARROWHEAD_SIZE: gpui::Pixels = px(8.0);
const STATUS_DOT_RADIUS: gpui::Pixels = px(6.0);
// These are in logical (canvas-space) f32 for position arithmetic:
const NODE_WIDTH_F: f32 = 200.0;
const NODE_HEIGHT_F: f32 = 72.0;
const NODE_H_GAP: f32 = 80.0;
const NODE_V_GAP: f32 = 60.0;
```

---

## Coordinate transform

`paint_layer` does **not** accept a transform — it only clips. All zoom/pan must be
applied manually to every coordinate before it reaches any draw call:

```rust
fn to_screen(&self, canvas_x: f32, canvas_y: f32, origin: gpui::Point<Pixels>) -> gpui::Point<Pixels> {
    let (ox, oy) = self.layout.viewport_offset;
    let z = self.layout.zoom;
    gpui::point(
        origin.x + px((canvas_x + ox) * z),
        origin.y + px((canvas_y + oy) * z),
    )
}

fn scaled(&self, canvas_val: f32) -> Pixels {
    px(canvas_val * self.layout.zoom)
}
```

All node positions, node dimensions, edge anchor points, and arrowhead sizes are
passed through `to_screen` / `scaled` before any draw call.

---

## `WorkflowCanvas` struct

```rust
pub struct WorkflowCanvas {
    pub workflow: Option<WorkflowDefinition>,
    pub run: Option<TaskStatusResponse>,
    pub layout: CanvasLayout,
    pub selection: CanvasSelection,
    pub mode: CanvasMode,
    connect_source: Option<String>,
    drag_node: Option<String>,
    drag_node_start_pos: Option<NodePos>,
    drag_mouse_start: Option<gpui::Point<Pixels>>,
    pan_mouse_start: Option<gpui::Point<Pixels>>,
    pan_viewport_start: Option<(f32, f32)>,
    focus_handle: gpui::FocusHandle,
    on_node_selected: Option<Box<dyn Fn(Option<String>, &mut gpui::Window, &mut gpui::App)>>,
    on_node_activated: Option<Box<dyn Fn(String, &mut gpui::Window, &mut gpui::App)>>,
    _poll_task: Option<gpui::Task<()>>,
    client: std::sync::Arc<WorkflowClient>,
}
```

---

## Node colors by kind

Use `theme::ActiveTheme` for text; hardcode fill/border colors as constants.
Check `window.appearance()` for light vs dark:

```rust
use theme::ActiveTheme as _;

fn node_colors(kind: &WorkflowNodeKind, appearance: gpui::WindowAppearance) -> (gpui::Rgba, gpui::Rgba) {
    // (fill, border)
    match (kind, appearance) {
        (WorkflowNodeKind::Task, gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark) =>
            (gpui::rgba(0x1a3a5cff), gpui::rgba(0x3b82f6ff)),
        (WorkflowNodeKind::Task, _) =>
            (gpui::rgba(0xdbeafeff), gpui::rgba(0x3b82f6ff)),
        (WorkflowNodeKind::Validation, gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark) =>
            (gpui::rgba(0x3a2e00ff), gpui::rgba(0xf59e0bff)),
        (WorkflowNodeKind::Validation, _) =>
            (gpui::rgba(0xfef3c7ff), gpui::rgba(0xf59e0bff)),
        (WorkflowNodeKind::Review, gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark) =>
            (gpui::rgba(0x2d1a00ff), gpui::rgba(0xf97316ff)),
        (WorkflowNodeKind::Review, _) =>
            (gpui::rgba(0xffedd5ff), gpui::rgba(0xf97316ff)),
        (WorkflowNodeKind::Integration, gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark) =>
            (gpui::rgba(0x0d2e1aff), gpui::rgba(0x22c55eff)),
        (WorkflowNodeKind::Integration, _) =>
            (gpui::rgba(0xdcfce7ff), gpui::rgba(0x22c55eff)),
    }
}
```

---

## Edge rendering (correct PathBuilder usage)

`PathBuilder` methods are `&mut self` — they are NOT chainable. Use this pattern:

```rust
fn paint_edge(
    &self,
    from_pos: &NodePos,
    to_pos: &NodePos,
    canvas_origin: gpui::Point<Pixels>,
    window: &mut gpui::Window,
) {
    // Anchor points in canvas space
    let src_x = from_pos.x + NODE_WIDTH_F;
    let src_y = from_pos.y + NODE_HEIGHT_F / 2.0;
    let dst_x = to_pos.x;
    let dst_y = to_pos.y + NODE_HEIGHT_F / 2.0;

    let src = self.to_screen(src_x, src_y, canvas_origin);
    let dst = self.to_screen(dst_x, dst_y, canvas_origin);
    let cp1 = self.to_screen(src_x + 60.0, src_y, canvas_origin);
    let cp2 = self.to_screen(dst_x - 60.0, dst_y, canvas_origin);

    let mut builder = gpui::PathBuilder::stroke(EDGE_STROKE);
    builder.move_to(src);
    // cubic_bezier_to(to, control_a, control_b) — destination comes FIRST
    builder.cubic_bezier_to(dst, cp1, cp2);
    if let Ok(path) = builder.build() {
        window.paint_path(path, gpui::rgba(0x888888cc));
    }

    // Arrowhead: small filled triangle at dst pointing right
    self.paint_arrowhead(dst, canvas_origin, window);
}
```

---

## Node rendering (correct paint_quad usage)

`paint_quad` accepts a `PaintQuad` (not a `Quad` struct literal). Use the
`gpui::fill`, `gpui::outline`, or `gpui::quad` free functions:

```rust
fn paint_node(
    &self,
    node: &WorkflowNode,
    pos: &NodePos,
    canvas_origin: gpui::Point<Pixels>,
    selected: bool,
    window: &mut gpui::Window,
    cx: &gpui::App,
) {
    let appearance = window.appearance();
    let (fill, border) = node_colors(&node.kind, appearance);
    let tl = self.to_screen(pos.x, pos.y, canvas_origin);
    let bounds = gpui::Bounds {
        origin: tl,
        size: gpui::size(self.scaled(NODE_WIDTH_F), self.scaled(NODE_HEIGHT_F)),
    };
    let radii = gpui::Corners::all(self.scaled(8.0));
    let border_width = if selected { px(3.0) } else { px(1.5) };
    let border_color = if selected {
        cx.theme().colors().border_focused
    } else {
        border.into()
    };

    window.paint_quad(gpui::quad(
        bounds,
        radii,
        gpui::Background::Color(fill.into()),
        gpui::Edges::all(border_width),
        border_color,
        gpui::BorderStyle::default(),
    ));

    // Label text: use text_system to shape, then paint
    self.paint_node_label(&node.label, bounds, window, cx);
}
```

---

## Text rendering (correct approach — no `window.draw_text`)

There is no `window.draw_text`. Use the two-step shape-then-paint approach:

```rust
fn paint_node_label(
    &self,
    label: &str,
    bounds: gpui::Bounds<Pixels>,
    window: &mut gpui::Window,
    cx: &gpui::App,
) {
    use gpui::{FontStyle, FontWeight, TextRun};

    let font_size = gpui::rems(0.875);
    let text_color = cx.theme().colors().text;
    let runs = [TextRun {
        len: label.len(),
        font: gpui::Font {
            family: theme::setup_ui_font(window, cx).family,
            features: Default::default(),
            fallbacks: Default::default(),
            weight: FontWeight::MEDIUM,
            style: FontStyle::Normal,
        },
        color: text_color,
        background_color: None,
        underline: None,
        strikethrough: None,
    }];
    let Ok(shaped) = window
        .text_system()
        .shape_line(label.into(), font_size, &runs)
    else {
        return;
    };

    // Center the label inside the node bounds
    let text_width = shaped.width;
    let x = bounds.origin.x + (bounds.size.width - text_width) / 2.0;
    let y = bounds.origin.y + bounds.size.height / 2.0 - font_size.to_pixels(window.rem_size()) / 2.0;
    shaped.paint(gpui::point(x, y), gpui::px(20.0), window).ok();
}
```

---

## Run-mode status dot

```rust
fn paint_status_dot(
    &self,
    status: &TaskLifecycleStatus,
    node_bounds: gpui::Bounds<Pixels>,
    window: &mut gpui::Window,
) {
    let color: gpui::Rgba = match status {
        TaskLifecycleStatus::Queued    => gpui::rgba(0x6b7280ff),
        TaskLifecycleStatus::Running   => gpui::rgba(0x3b82f6ff),
        TaskLifecycleStatus::Completed => gpui::rgba(0x22c55eff),
        TaskLifecycleStatus::Failed    => gpui::rgba(0xef4444ff),
    };
    let cx_pt = gpui::point(
        node_bounds.origin.x + node_bounds.size.width - STATUS_DOT_RADIUS - px(6.0),
        node_bounds.origin.y + STATUS_DOT_RADIUS + px(6.0),
    );
    let mut builder = gpui::PathBuilder::fill();
    builder.arc(cx_pt, STATUS_DOT_RADIUS, 0.0, std::f32::consts::TAU);
    if let Ok(path) = builder.build() {
        window.paint_path(path, color.into());
    }
}
```

For the pulsing "running" animation, call `window.request_animation_frame(cx)` from
inside `Render::render` and store an animation phase `f32` in the struct to modulate
opacity on each frame.

---

## Rendering via `canvas()` element

```rust
impl Render for WorkflowCanvas {
    fn render(&mut self, window: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        use gpui::IntoElement as _;

        // Kick off animation frame for running nodes
        if self.has_running_nodes() {
            window.request_animation_frame(cx);
        }

        let this = cx.entity().downgrade();
        let workflow = self.workflow.clone();
        let run = self.run.clone();
        let layout = self.layout.clone();
        let selection = self.selection.clone();

        gpui::div()
            .size_full()
            .relative()
            // toolbar overlay (add/connect/fit buttons) — positioned absolute top
            .child(self.render_toolbar(cx))
            .child(
                gpui::canvas(
                    // prepaint: compute bounds only
                    move |bounds, _window, _cx| bounds,
                    // paint: draw graph
                    move |bounds, _prepaint, window, cx| {
                        let origin = bounds.origin;
                        // Draw edges first (under nodes)
                        if let Some(ref wf) = workflow {
                            for edge in &wf.edges {
                                let from_pos = layout.node_positions.get(&edge.from);
                                let to_pos = layout.node_positions.get(&edge.to);
                                if let (Some(fp), Some(tp)) = (from_pos, to_pos) {
                                    // call paint_edge equivalent inline or via a fn
                                    Self::paint_edge_static(&layout, fp, tp, origin, window);
                                }
                            }
                            for node in &wf.nodes {
                                let pos = layout.node_positions.get(&node.id)
                                    .copied()
                                    .unwrap_or(NodePos { x: 0.0, y: 0.0 });
                                let selected = matches!(&selection, CanvasSelection::Node(id) if id == &node.id);
                                Self::paint_node_static(&layout, node, &pos, selected, None, origin, window, cx);
                            }
                        } else if let Some(ref run_data) = run {
                            // run mode: same but with status overlays
                            for node_status in &run_data.nodes {
                                let pos = layout.node_positions.get(&node_status.id)
                                    .copied()
                                    .unwrap_or(NodePos { x: 0.0, y: 0.0 });
                                Self::paint_node_static(&layout, &WorkflowNode {
                                    id: node_status.id.clone(),
                                    kind: node_status.kind.clone(),
                                    label: node_status.label.clone(),
                                }, &pos, false, Some(&node_status.status), origin, window, cx);
                            }
                        }
                    },
                )
                .size_full()
            )
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(Self::handle_mouse_down))
            .on_mouse_move(cx.listener(Self::handle_mouse_move))
            .on_mouse_up(gpui::MouseButton::Left, cx.listener(Self::handle_mouse_up))
            .on_scroll_wheel(cx.listener(Self::handle_scroll))
            .key_context("WorkflowCanvas")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
    }
}
```

> **Note:** Since `canvas()` closures capture by move and `window`/`cx` are
> provided as parameters in the `paint` closure, instance methods that need
> `&self` must either be made `static` (taking layout/selection by value) or
> the needed data must be cloned into the closure before it captures.

---

## Input handling

**Hit testing** (canvas space from screen coordinates):

```rust
fn hit_node(&self, screen_pt: gpui::Point<Pixels>, canvas_origin: gpui::Point<Pixels>) -> Option<String> {
    let (ox, oy) = self.layout.viewport_offset;
    let z = self.layout.zoom;
    // Convert screen → canvas space
    let cx_coord = (screen_pt.x - canvas_origin.x).0 / z - ox;
    let cy_coord = (screen_pt.y - canvas_origin.y).0 / z - oy;
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
```

**Mouse down** — `handle_mouse_down`:
- Select mode: hit-test → set selection + start drag OR start pan if background
- Connect mode: first click sets `connect_source`; second click creates edge + clears source

**Mouse move** — `handle_mouse_move`:
- If dragging: update `layout.node_positions[id]`, `cx.notify()`
- If panning: update `layout.viewport_offset`, `cx.notify()`

**Mouse up** — `handle_mouse_up`:
- End drag/pan; schedule layout save (debounced via `cx.background_executor().timer`)

**Scroll wheel** — `handle_scroll`:
- Zoom toward cursor; adjust `viewport_offset` to keep cursor point fixed in canvas space

**Key down** — `handle_key_down`:
- `Delete` / `Backspace`: remove selected node/edge (edit mode only)
- `Escape`: clear selection, cancel connect mode

---

## Default auto-layout

When a workflow has no saved positions, auto-layout before first paint:

```rust
fn auto_layout(nodes: &[WorkflowNode], edges: &[WorkflowEdge]) -> HashMap<String, NodePos> {
    // Topological sort (BFS from roots)
    // Assign column = depth from root, row = index within column
    // x = col * (NODE_WIDTH_F + NODE_H_GAP), y = row * (NODE_HEIGHT_F + NODE_V_GAP)
    // ...
    todo!()  // full implementation left to implementor
}
```

Call `auto_layout` in `WorkflowCanvas::new_edit` / `new_run` if
`layout.node_positions.is_empty()`.

---

## Layout persistence

```rust
fn layout_path(workflow_id: Uuid) -> std::path::PathBuf {
    paths::support_dir()
        .join("workflow-layouts")
        .join(format!("{workflow_id}.json"))
}

fn save_layout_async(layout: &CanvasLayout, workflow_id: Uuid, cx: &mut gpui::Context<Self>) {
    let json = match serde_json::to_string(layout) {
        Ok(j) => j,
        Err(e) => { log::error!("failed to serialize layout: {e}"); return; }
    };
    cx.background_spawn(async move {
        let path = layout_path(workflow_id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).log_err();
        }
        std::fs::write(&path, json).log_err();
    }).detach();
}

fn load_layout(workflow_id: Uuid, cx: &mut gpui::Context<Self>) -> gpui::Task<Option<CanvasLayout>> {
    cx.background_spawn(async move {
        let path = layout_path(workflow_id);
        let json = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&json).ok()
    })
}
```

---

## `workspace::Item` implementation

```rust
pub enum WorkflowCanvasEvent {
    NodeSelected(Option<String>),
}

impl gpui::EventEmitter<WorkflowCanvasEvent> for WorkflowCanvas {}

impl workspace::Item for WorkflowCanvas {
    type Event = WorkflowCanvasEvent;

    fn tab_content_text(&self, _cx: &gpui::App) -> Option<gpui::SharedString> {
        let name = self.workflow.as_ref().map(|w| w.name.as_str())
            .or_else(|| self.run.as_ref().and_then(|r| r.workflow.as_ref().map(|w| w.name.as_str())))
            .unwrap_or("Workflow");
        Some(name.to_string().into())
    }

    fn to_item_events(event: &WorkflowCanvasEvent, mut f: impl FnMut(workspace::ItemEvent)) {
        match event {
            WorkflowCanvasEvent::NodeSelected(_) => f(workspace::ItemEvent::UpdateTab),
        }
    }
}
```

Opening a canvas:
```rust
pub fn open_workflow(
    workflow: WorkflowDefinitionRecord,
    workspace: &mut workspace::Workspace,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<workspace::Workspace>,
) {
    let canvas = cx.new(|cx| WorkflowCanvas::new_edit(workflow, window, cx));
    workspace.add_item_to_center(Box::new(canvas), window, cx);
}
```

---

## Run mode polling

```rust
fn start_polling(&mut self, task_id: Uuid, cx: &mut gpui::Context<Self>) {
    let client = self.client.clone();
    self._poll_task = Some(cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(std::time::Duration::from_secs(2)).await;
            let Ok(status) = client.get_task_status(task_id).await else { continue };
            let is_terminal = status.task.status.is_terminal();
            this.update(cx, |canvas, cx| {   // NOTE: cx not &mut cx
                canvas.run = Some(status);
                cx.notify();
            }).ok();
            if is_terminal { break; }
        }
    }));
}
```

---

## Add Node (edit mode toolbar)

Toolbar at top of canvas (absolutely-positioned `div` overlay):
- Buttons: "＋ Task", "＋ Validation", "＋ Review", "＋ Integration"
- "Connect" toggle (sets `mode = CanvasMode::Connect`)
- "Fit" button (resets zoom/pan to show all nodes)

Clicking a kind button:
1. Generates `id = uuid::Uuid::new_v4().to_string()`
2. Appends `WorkflowNode { id, kind, label: "New Node".into() }` to workflow
3. Positions at center of current viewport in canvas space:
   `canvas_center_x = -viewport_offset.x + visible_width / (2.0 * zoom)`
4. Sets `selection = CanvasSelection::Node(id)`, fires `on_node_selected`
5. Calls `cx.notify()`

---

## Testing checklist

- Nodes render at correct screen positions after zoom/pan transform
- Edges draw cubic beziers between correct anchor points (arrowhead at destination)
- `cubic_bezier_to` arg order: destination first, then cp1, cp2
- `PathBuilder::build()` result handled with `if let Ok`
- Dragging a node updates position in real time
- Zoom via scroll wheel, zooms toward cursor
- `CanvasLayout::default()` has `zoom = 1.0`
- Auto-layout produces readable graph for a 3-node chain
- Layout saves to disk on mouse-up (no `unwrap()` on path.parent())
- Layout loads and restores positions on reopen
- Run mode: status dots appear; polling updates every 2s; stops on terminal
- Click node in run mode fires `on_node_activated`
- Add node: appears at viewport center, gets selected
- Connect mode: two node clicks create edge
- Delete key removes selected node/edge in edit mode only
- Text labels render inside node bounds (shape + paint pattern)
