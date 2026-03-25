# Workflow Canvas Edge & Globals Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Four improvements to the workflow canvas: hide the globals node (show it in the inspector on background click), route backward edges above/below nodes, add white-glow edge selection, and add a draggable waypoint handle on selected forward edges.

**Architecture:** All changes are in `crates/workflow_ui/canvas.rs`. Features 1–3 are independent. Feature 4 (waypoints) depends on Feature 3's `edge_draw_list` type change. Tasks are ordered by dependency: globals → edge_draw_list/glow → backward routing → waypoints.

**Tech Stack:** Rust, GPUI canvas painting (`gpui::PathBuilder`, `window.paint_path`), `HashMap`, serde.

---

## File Map

| File | Changes |
|------|---------|
| `crates/workflow_ui/canvas.rs` | All changes — hide globals, background click, arc routing, glow, waypoints |

No new files needed.

---

## Task 1: Hide globals node and open globals inspector on background click

**Files:**
- Modify: `crates/workflow_ui/canvas.rs`

The globals node (`node_type == "workflow_globals"`) must be invisible and non-interactive on the canvas. Clicking the canvas background should open it in the inspector via the existing `NodeSelected` event.

### Step 1.1: Add a helper to find the globals node ID from the current workflow

- [ ] In `canvas.rs`, add this helper method on `WorkflowCanvas` (place it near `hit_test_node`):

```rust
fn globals_node_id(&self) -> Option<String> {
    let wf = self.workflow.as_ref()?;
    wf.nodes
        .iter()
        .find(|n| n.node_type == WORKFLOW_GLOBALS_NODE_TYPE_ID)
        .map(|n| n.id.clone())
}
```

### Step 1.2: Skip the globals node in `hit_test_node`

- [ ] In `hit_test_node` (line 442), add a skip before returning a hit. Change the loop body to:

```rust
for (id, pos) in &self.layout.node_positions {
    // Globals node is hidden from canvas interaction
    if self.globals_node_id().as_deref() == Some(id.as_str()) {
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
```

### Step 1.3: Skip the globals node in `hit_test_port`

- [ ] In `hit_test_port` (line 469), add a skip at the top of the `for node in &workflow.nodes` loop:

```rust
for node in &workflow.nodes {
    if node.node_type == WORKFLOW_GLOBALS_NODE_TYPE_ID {
        continue;
    }
    // ... rest of loop unchanged
```

### Step 1.4: Skip rendering the globals node in the editable paint loop

- [ ] In the render loop at line 1968 (`for node in &wf.nodes`), add a skip:

```rust
for node in &wf.nodes {
    if node.node_type == WORKFLOW_GLOBALS_NODE_TYPE_ID {
        continue;
    }
    // ... rest of node rendering unchanged
```

### Step 1.5: Skip rendering the globals node in the run-view paint loop

- [ ] Locate the second node rendering loop (run-view path, after line 2018). Apply the same skip:

```rust
if node.node_type == WORKFLOW_GLOBALS_NODE_TYPE_ID {
    continue;
}
```

### Step 1.6: Add a background click field and track mouse-down position

- [ ] Add a field to `WorkflowCanvas` near `pan_mouse_start` (line 273):

```rust
background_click_start: Option<Point<Pixels>>,
```

- [ ] In `handle_mouse_down`, in the `else` branch (line 629, no node hit), set it alongside `pan_mouse_start`:

```rust
} else {
    self.clear_selection(window, cx);
    self.pending_connection = None;
    self.pending_connection_target = None;
    self.pan_mouse_start = Some(position);
    self.pan_viewport_start = Some(self.layout.viewport_offset);
    self.background_click_start = Some(position);  // add this line
}
```

### Step 1.7: Emit `NodeSelected` on background mouse-up if not a pan

- [ ] In `handle_mouse_up` (line 736), after clearing drag state (around line 778), add:

```rust
if let Some(click_start) = self.background_click_start.take() {
    let dx = (event.position.x - click_start.x).as_f32();
    let dy = (event.position.y - click_start.y).as_f32();
    let dist_sq = dx * dx + dy * dy;
    if dist_sq < 4.0 * 4.0 {
        // Short click on background — open globals in inspector
        cx.emit(WorkflowCanvasEvent::NodeSelected(self.globals_node_id()));
    }
}
```

Also clear it in the normal pan/clear path: add `self.background_click_start = None;` alongside the other drag-state clears at line 775.

### Step 1.8: Build and verify

- [ ] Run: `./script/clippy`
- [ ] Expected: no errors. The globals node should no longer appear on canvas; clicking the background should open the inspector showing globals.

### Step 1.9: Commit

```bash
git add crates/workflow_ui/canvas.rs
git commit -m "workflow_ui: Hide globals node from canvas, open on background click"
```

---

## Task 2: Edge selection white glow (requires edge_draw_list to carry WorkflowEdge)

**Files:**
- Modify: `crates/workflow_ui/canvas.rs` (lines 1939–2001, 1474–1507)

Currently `edge_draw_list` carries only port positions. To know whether an edge is selected we need its identity at paint time.

### Step 2.1: Change `edge_draw_list` type to carry the edge

- [ ] At line 1939, change the type and construction:

```rust
let edge_draw_list: Vec<(WorkflowEdge, (f32, f32), (f32, f32))> = wf
    .edges
    .iter()
    .filter_map(|edge| {
        Some((
            edge.clone(),
            port_position_for_node(
                &layout, wf, &node_types, &edge.from_node_id, &edge.from_output_id, false,
            )?,
            port_position_for_node(
                &layout, wf, &node_types, &edge.to_node_id, &edge.to_input_id, true,
            )?,
        ))
    })
    .collect();
```

- [ ] Update the forward-edge paint loop (line 1963):

```rust
for (edge, from_port, to_port) in &edge_draw_list {
    if !is_backward_edge(*from_port, *to_port) {
        let is_selected = matches!(&selection,
            CanvasSelection::Edge(fn_id, fo_id, tn_id, ti_id)
            if fn_id == &edge.from_node_id
                && fo_id == &edge.from_output_id
                && tn_id == &edge.to_node_id
                && ti_id == &edge.to_input_id
        );
        paint_edge(&layout, *from_port, *to_port, is_selected, origin, window);
    }
}
```

- [ ] Update the backward-edge paint loop (line 1998):

```rust
for (edge, from_port, to_port) in &edge_draw_list {
    if is_backward_edge(*from_port, *to_port) {
        let is_selected = matches!(&selection,
            CanvasSelection::Edge(fn_id, fo_id, tn_id, ti_id)
            if fn_id == &edge.from_node_id
                && fo_id == &edge.from_output_id
                && tn_id == &edge.to_node_id
                && ti_id == &edge.to_input_id
        );
        paint_edge(&layout, *from_port, *to_port, is_selected, origin, window);
    }
}
```

### Step 2.2: Add `is_selected` to `paint_edge` signature and implement glow

- [ ] Change `paint_edge` signature (line 1474):

```rust
fn paint_edge(
    layout: &CanvasLayout,
    from: (f32, f32),
    to: (f32, f32),
    is_selected: bool,
    origin: Point<Pixels>,
    window: &mut Window,
) {
```

- [ ] Replace the color and stroke setup with selection-aware versions:

```rust
let edge_color = if is_selected {
    gpui::rgba(0xffffffff)
} else {
    gpui::rgba(0x9ca3afff)
};
let stroke_width = scaled(layout, if is_selected { 2.5 } else { EDGE_STROKE.as_f32() });
```

- [ ] For the non-backward (forward bezier) path, add a glow pre-pass before the main stroke:

```rust
if is_selected {
    let glow_color = gpui::rgba(0xffffff40); // white at ~25% opacity
    let glow_width = scaled(layout, 6.0);
    let dx_g = (to_pt.x - from_pt.x).as_f32();
    let off_g = px(bezier_ctrl_offset(dx_g));
    let ctrl_a_g = gpui::point(from_pt.x + off_g, from_pt.y);
    let ctrl_b_g = gpui::point(to_pt.x - off_g, to_pt.y);
    let mut glow_builder = gpui::PathBuilder::stroke(glow_width);
    glow_builder.move_to(from_pt);
    glow_builder.cubic_bezier_to(to_pt, ctrl_a_g, ctrl_b_g);
    if let Ok(glow_path) = glow_builder.build() {
        window.paint_path(glow_path, glow_color);
    }
}
```

- [ ] Similarly, for the backward-edge call to `paint_smoothstep_edge`, pass the glow layer inside `paint_smoothstep_edge` by adding an `is_selected: bool` parameter and the same pre-pass pattern before the main stroke.

The updated `paint_smoothstep_edge` signature becomes:

```rust
fn paint_smoothstep_edge(
    layout: &CanvasLayout,
    from_pt: Point<Pixels>,
    to_pt: Point<Pixels>,
    edge_color: gpui::Rgba,
    stroke_width: Pixels,
    is_selected: bool,
    window: &mut Window,
)
```

Add inside before the main `builder`:
```rust
if is_selected {
    let glow_color = gpui::rgba(0xffffff40);
    let glow_width = scaled(layout, 6.0);
    let mut glow_builder = gpui::PathBuilder::stroke(glow_width);
    paint_smoothstep_polyline(&mut glow_builder, &[from_pt, p1, p2, p3, p4, to_pt], corner_r);
    if let Ok(glow_path) = glow_builder.build() {
        window.paint_path(glow_path, glow_color);
    }
}
```

### Step 2.3: Build and verify

- [ ] Run: `./script/clippy`
- [ ] Expected: compiles cleanly. Selecting an edge should render it white with a soft glow.

### Step 2.4: Commit

```bash
git add crates/workflow_ui/canvas.rs
git commit -m "workflow_ui: Add white glow selection indicator for edges"
```

---

## Task 3: Backward edge arc routing (above or below all nodes)

**Files:**
- Modify: `crates/workflow_ui/canvas.rs`

Replace the current elbow routing (which threads through nodes) with a bounding-box arc that goes above or below all nodes.

### Step 3.1: Add a pure `nodes_bounding_box` helper (with test)

- [ ] Add this function near the geometry helpers (after `to_canvas_point`):

```rust
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
```

- [ ] Add a test:

```rust
#[test]
fn test_nodes_bounding_box() {
    let mut positions = HashMap::new();
    positions.insert("a".to_string(), NodePos { x: 0.0, y: 50.0 });
    positions.insert("b".to_string(), NodePos { x: 200.0, y: 100.0 });
    let (top, bottom) = nodes_bounding_box(&positions, 40.0);
    assert_eq!(top, 10.0);   // 50 - 40
    assert_eq!(bottom, 296.0); // 100 + 156 + 40
}
```

- [ ] Run test: `cargo test -p workflow_ui test_nodes_bounding_box`
- [ ] Expected: PASS.

### Step 3.2: Add a pure `backward_edge_rail_y` helper (with test)

- [ ] Add after `nodes_bounding_box`:

```rust
/// Compute the rail Y for a backward edge on the given side.
/// `above=true` → above top, `above=false` → below bottom.
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
```

- [ ] Add a test:

```rust
#[test]
fn test_backward_edge_rail_y() {
    assert_eq!(backward_edge_rail_y(10.0, 296.0, 0, true), 10.0);
    assert_eq!(backward_edge_rail_y(10.0, 296.0, 1, true), -2.0);
    assert_eq!(backward_edge_rail_y(10.0, 296.0, 0, false), 296.0);
    assert_eq!(backward_edge_rail_y(10.0, 296.0, 2, false), 320.0);
}
```

- [ ] Run test: `cargo test -p workflow_ui test_backward_edge_rail_y`
- [ ] Expected: PASS.

### Step 3.3: Pre-compute arc routing data before the paint loop

The backward edge paint pass needs bounding box and per-edge rail assignments. These must be computed before the paint closure (or captured from outside).

- [ ] Before the `edge_draw_list` construction in the render closure, compute:

```rust
// Compute bounding box for backward edge arc routing
let (bbox_top, bbox_bottom) = nodes_bounding_box(&layout.node_positions, 40.0);

// Assign rail indices to backward edges
// Group by side (above/below), sort by span (longest = outermost)
let backward_edge_rails: HashMap<(String, String, String, String), (bool, usize)> = {
    let backward: Vec<_> = wf
        .edges
        .iter()
        .filter_map(|edge| {
            let from = port_position_for_node(&layout, wf, &node_types, &edge.from_node_id, &edge.from_output_id, false)?;
            let to = port_position_for_node(&layout, wf, &node_types, &edge.to_node_id, &edge.to_input_id, true)?;
            if !is_backward_edge(from, to) {
                return None;
            }
            let mid_y = (from.1 + to.1) / 2.0;
            let cost_above = mid_y - bbox_top;
            let cost_below = bbox_bottom - mid_y;
            let above = cost_above <= cost_below;
            let span = (from.0 - to.0).abs();
            Some((edge, above, span))
        })
        .collect();

    let mut above_edges: Vec<_> = backward.iter().filter(|(_, above, _)| *above).collect();
    let mut below_edges: Vec<_> = backward.iter().filter(|(_, above, _)| !*above).collect();
    // Sort by span descending (longest span = outermost = highest index)
    above_edges.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    below_edges.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    let mut result = HashMap::new();
    for (i, (edge, above, _)) in above_edges.iter().enumerate() {
        let key = (edge.from_node_id.clone(), edge.from_output_id.clone(), edge.to_node_id.clone(), edge.to_input_id.clone());
        result.insert(key, (*above, i));
    }
    for (i, (edge, above, _)) in below_edges.iter().enumerate() {
        let key = (edge.from_node_id.clone(), edge.from_output_id.clone(), edge.to_node_id.clone(), edge.to_input_id.clone());
        result.insert(key, (*above, i));
    }
    result
};
```

### Step 3.4: Replace `paint_smoothstep_edge` with arc routing

- [ ] Replace the function body of `paint_smoothstep_edge` (currently lines 1511–1535) with the arc implementation. Change the signature to accept the rail data:

```rust
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
    paint_arrowhead_directed(layout, to_pt, 1.0, 0.0, window);
}
```

- [ ] Update `paint_edge` to use the new function. The backward-edge branch becomes:

```rust
if is_backward_edge(from, to) {
    // rail_y must be supplied by caller; this overload is for callers without rail data
    paint_smoothstep_edge(layout, from_pt, to_pt, edge_color, stroke_width, is_selected, window);
} else {
    // ... forward bezier unchanged
}
```

Wait — `paint_edge` is called with `from`/`to` only, not rail data. The callers that have rail data need to call a different path.

- [ ] Restructure: introduce a `BackwardEdgeRailData` that gets passed to the canvas paint callback. Instead of passing it into `paint_edge`, call `paint_arc_backward_edge` directly in the backward-edge loop:

```rust
// In the backward edge loop:
for (edge, from_port, to_port) in &edge_draw_list {
    if is_backward_edge(*from_port, *to_port) {
        let is_selected = /* same check as before */;
        let edge_color = if is_selected { gpui::rgba(0xffffffff) } else { gpui::rgba(0x9ca3afff) };
        let stroke_width = scaled(&layout, if is_selected { 2.5 } else { EDGE_STROKE.as_f32() });
        let from_pt = to_screen_point(&layout, from_port.0, from_port.1, origin);
        let to_pt = to_screen_point(&layout, to_port.0, to_port.1, origin);
        let key = (edge.from_node_id.clone(), edge.from_output_id.clone(), edge.to_node_id.clone(), edge.to_input_id.clone());
        let (above, rail_index) = backward_edge_rails.get(&key).copied().unwrap_or((true, 0));
        let rail_y = backward_edge_rail_y(bbox_top, bbox_bottom, rail_index, above);
        paint_arc_backward_edge(
            &layout, from_pt, to_pt, rail_y, edge_color, stroke_width, is_selected, origin, window,
        );
    }
}
```

You can simplify `paint_edge` to only handle forward edges after this change, removing the `is_backward_edge` branch from it.

### Step 3.5: Update `edge_contains_screen_point` for arc geometry

- [ ] The backward-edge hit-test (lines 1677–1695) still uses the old elbow geometry. Replace with the arc polyline. The hit-test function needs access to the bounding box and rail assignments.

Add a `backward_arc_contains_screen_point` helper function and update `hit_test_edge` to call it. **Important:** the rail assignment logic (bbox computation + side selection + rail indexing) must live in a shared method `backward_edge_rail_assignments(&self) -> (f32, f32, HashMap<...>)` so the render closure and `hit_test_edge` both call the same logic — if they diverge, clicks on backward edges will miss. Call this method from both the render pre-computation step (Task 3.3) and from inside `hit_test_edge`.

```rust
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
    let threshold = px(8.0);

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
```

- [ ] Update `hit_test_edge` to use the above when the edge is backward. The function will need access to `backward_edge_rails` and `bbox_top`/`bbox_bottom`. Since `hit_test_edge` is a method on `WorkflowCanvas`, you can compute those inside it (it already has `self.layout` and `self.workflow`).

Add a helper method `backward_edge_rail_assignments` that returns the same `HashMap` built in the render closure, so the logic isn't duplicated. This method takes `&self` and returns `(f32, f32, HashMap<(String,String,String,String),(bool,usize)>)` (bbox_top, bbox_bottom, rail_map).

### Step 3.6: Build and verify

- [ ] Run: `./script/clippy`
- [ ] Expected: no errors. Backward edges should now arc above or below all nodes.

### Step 3.7: Commit

```bash
git add crates/workflow_ui/canvas.rs
git commit -m "workflow_ui: Route backward edges above/below nodes instead of through them"
```

---

## Task 4: Edge waypoint drag handle

**Files:**
- Modify: `crates/workflow_ui/canvas.rs`

Forward edges support an optional single waypoint stored in `CanvasLayout`. When an edge is selected, a drag handle appears at the midpoint. Dragging it bends the edge. Backward arc edges do not support waypoints.

### Step 4.1: Add `edge_waypoints` to `CanvasLayout`

- [ ] In the `CanvasLayout` struct (line 50):

```rust
#[derive(serde::Serialize, serde::Deserialize)]
pub struct CanvasLayout {
    pub node_positions: HashMap<String, NodePos>,
    pub viewport_offset: (f32, f32),
    pub zoom: f32,
    #[serde(skip)]
    pub edge_waypoints: HashMap<(String, String, String, String), (f32, f32)>,
}
```

- [ ] Update `CanvasLayout::default()` to include the field:

```rust
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
```

- [ ] Update the manual `Clone` impl (line 66):

```rust
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
```

### Step 4.2: Add `drag_edge_waypoint_key` to `WorkflowCanvas`

- [ ] Add near the other drag fields (line 270–274):

```rust
drag_edge_waypoint_key: Option<(String, String, String, String)>,
```

- [ ] Initialize to `None` wherever `WorkflowCanvas` is constructed (search for `WorkflowCanvas {` to find all construction sites).

### Step 4.3: Fix `edge_canvas_midpoint` and promote it from test-only

- [ ] Replace the current `#[cfg(test)]` function (line 1714) with a production version:

```rust
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
```

### Step 4.4: Add `hit_test_edge_handle` method

- [ ] Add after `hit_test_edge`:

```rust
/// Returns the edge key if the point is within 8px of the selected edge's waypoint handle.
/// Only returns Some when an edge is selected and editable.
fn hit_test_edge_handle(
    &self,
    screen_pt: Point<Pixels>,
    canvas_origin: Point<Pixels>,
) -> Option<(String, String, String, String)> {
    if !self.is_editable() {
        return None;
    }
    let CanvasSelection::Edge(ref fn_id, ref fo_id, ref tn_id, ref ti_id) = self.selection else {
        return None;
    };
    let wf = self.workflow.as_ref()?;
    let edge_key = (fn_id.clone(), fo_id.clone(), tn_id.clone(), ti_id.clone());
    let from = port_position_for_node(&self.layout, wf, &self.node_types, fn_id, fo_id, false)?;
    let to = port_position_for_node(&self.layout, wf, &self.node_types, tn_id, ti_id, true)?;

    // Only forward edges support waypoints
    if is_backward_edge(from, to) {
        return None;
    }

    let handle_canvas = self.layout.edge_waypoints
        .get(&edge_key)
        .copied()
        .unwrap_or_else(|| edge_canvas_midpoint(from, to));

    let handle_screen = to_screen_point(&self.layout, handle_canvas.0, handle_canvas.1, canvas_origin);
    let dx = (screen_pt.x - handle_screen.x).as_f32();
    let dy = (screen_pt.y - handle_screen.y).as_f32();
    if dx * dx + dy * dy <= (8.0 * self.layout.zoom).powi(2) {
        Some(edge_key)
    } else {
        None
    }
}
```

### Step 4.5: Handle waypoint drag in `handle_mouse_down`

- [ ] In `handle_mouse_down`, after the port hit-test guard and before `hit_test_node` (after line 617), add:

```rust
// Check for edge waypoint handle (after port test, before node test)
if let Some(edge_key) = self.hit_test_edge_handle(position, origin) {
    if event.click_count == 2 {
        // Double-click removes the waypoint
        self.layout.edge_waypoints.remove(&edge_key);
        cx.notify();
    } else {
        self.drag_edge_waypoint_key = Some(edge_key);
        cx.notify();
    }
    return;
}
```

### Step 4.6: Update waypoint position in `handle_mouse_move`

- [ ] In `handle_mouse_move` (find the method that handles `MouseMoveEvent`), add after the existing pan/drag-node logic:

```rust
if let Some(ref key) = self.drag_edge_waypoint_key {
    let origin = self.canvas_origin();
    let canvas_pos = to_canvas_point(&self.layout, event.position.x.as_f32(), event.position.y.as_f32(), origin);
    self.layout.edge_waypoints.insert(key.clone(), canvas_pos);
    cx.notify();
}
```

### Step 4.7: Clear drag state in `handle_mouse_up`

- [ ] In `handle_mouse_up`, alongside the other drag-state clears (lines 775–779):

```rust
self.drag_edge_waypoint_key = None;
```

### Step 4.8: Render waypoint path in `paint_edge` for forward edges

- [ ] `paint_edge` does not have access to `edge_waypoints` currently. Pass it as a parameter, or pass the optional waypoint position directly. The simplest: add `waypoint: Option<(f32, f32)>` parameter.

- [ ] Change `paint_edge` signature:

```rust
fn paint_edge(
    layout: &CanvasLayout,
    from: (f32, f32),
    to: (f32, f32),
    waypoint: Option<(f32, f32)>,
    is_selected: bool,
    origin: Point<Pixels>,
    window: &mut Window,
)
```

- [ ] In the forward-edge branch, replace the single bezier with:

```rust
if let Some(wp) = waypoint {
    // Split into two bezier segments: from → wp → to
    let wp_pt = to_screen_point(layout, wp.0, wp.1, origin);

    if is_selected {
        let glow_color = gpui::rgba(0xffffff40);
        let glow_width = scaled(layout, 6.0);
        // Glow segment 1
        let dx1 = (wp_pt.x - from_pt.x).as_f32();
        let off1 = px(bezier_ctrl_offset(dx1));
        let mut gb = gpui::PathBuilder::stroke(glow_width);
        gb.move_to(from_pt);
        gb.cubic_bezier_to(wp_pt, gpui::point(from_pt.x + off1, from_pt.y), gpui::point(wp_pt.x - off1, wp_pt.y));
        if let Ok(p) = gb.build() { window.paint_path(p, glow_color); }
        // Glow segment 2
        let dx2 = (to_pt.x - wp_pt.x).as_f32();
        let off2 = px(bezier_ctrl_offset(dx2));
        let mut gb2 = gpui::PathBuilder::stroke(glow_width);
        gb2.move_to(wp_pt);
        gb2.cubic_bezier_to(to_pt, gpui::point(wp_pt.x + off2, wp_pt.y), gpui::point(to_pt.x - off2, to_pt.y));
        if let Ok(p) = gb2.build() { window.paint_path(p, glow_color); }
    }

    // Segment 1: from → wp
    let dx1 = (wp_pt.x - from_pt.x).as_f32();
    let off1 = px(bezier_ctrl_offset(dx1));
    let ctrl_a1 = gpui::point(from_pt.x + off1, from_pt.y);
    let ctrl_b1 = gpui::point(wp_pt.x - off1, wp_pt.y);
    let mut b1 = gpui::PathBuilder::stroke(stroke_width);
    b1.move_to(from_pt);
    b1.cubic_bezier_to(wp_pt, ctrl_a1, ctrl_b1);
    if let Ok(p) = b1.build() { window.paint_path(p, edge_color); }

    // Segment 2: wp → to
    let dx2 = (to_pt.x - wp_pt.x).as_f32();
    let off2 = px(bezier_ctrl_offset(dx2));
    let ctrl_a2 = gpui::point(wp_pt.x + off2, wp_pt.y);
    let ctrl_b2 = gpui::point(to_pt.x - off2, to_pt.y);
    let mut b2 = gpui::PathBuilder::stroke(stroke_width);
    b2.move_to(wp_pt);
    b2.cubic_bezier_to(to_pt, ctrl_a2, ctrl_b2);
    if let Ok(p) = b2.build() { window.paint_path(p, edge_color); }

    paint_arrowhead_directed(layout, to_pt, (to_pt.x - ctrl_b2.x).as_f32(), (to_pt.y - ctrl_b2.y).as_f32(), window);
} else {
    // Existing single bezier (moved from before)
    // ... existing forward bezier code, unchanged
}
```

- [ ] Update both call sites in the render loop to look up the waypoint from `layout.edge_waypoints`:

```rust
let edge_key = (edge.from_node_id.clone(), edge.from_output_id.clone(), edge.to_node_id.clone(), edge.to_input_id.clone());
let waypoint = layout.edge_waypoints.get(&edge_key).copied();
paint_edge(&layout, *from_port, *to_port, waypoint, is_selected, origin, window);
```

### Step 4.9: Add a fourth render pass for edge waypoint handles

- [ ] After the backward-edge paint loop (after line 2002), add a handle-painting pass. This ensures handles appear on top of all nodes:

```rust
// Fourth pass: paint waypoint handle for selected forward edge
if let CanvasSelection::Edge(ref fn_id, ref fo_id, ref tn_id, ref ti_id) = selection {
    if let (Some(ref wf_inner), true) = (&workflow, is_editable) {
        if let Some(edge) = wf_inner.edges.iter().find(|e| {
            e.from_node_id == *fn_id && e.from_output_id == *fo_id
                && e.to_node_id == *tn_id && e.to_input_id == *ti_id
        }) {
            let from_opt = port_position_for_node(&layout, wf_inner, &node_types, fn_id, fo_id, false);
            let to_opt = port_position_for_node(&layout, wf_inner, &node_types, tn_id, ti_id, true);
            if let (Some(from), Some(to)) = (from_opt, to_opt) {
                if !is_backward_edge(from, to) {
                    let edge_key = (edge.from_node_id.clone(), edge.from_output_id.clone(), edge.to_node_id.clone(), edge.to_input_id.clone());
                    let handle_canvas = layout.edge_waypoints
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
                    });
                }
            }
        }
    }
}
```

Note: the render closure captures `is_editable` as a bool — check how the closure is set up and capture it alongside `workflow`, `layout`, and `selection`.

### Step 4.10: Build and verify

- [ ] Run: `./script/clippy`
- [ ] Expected: compiles cleanly. Selecting a forward edge should show a circular drag handle at its midpoint. Dragging the handle should bend the edge. Double-clicking removes the waypoint.

### Step 4.11: Commit

```bash
git add crates/workflow_ui/canvas.rs
git commit -m "workflow_ui: Add draggable waypoint handle on selected forward edges"
```

---

## Final Check

- [ ] Run `./script/clippy` one last time from the repo root.
- [ ] Manually verify all four behaviors:
  1. Canvas background click opens globals in inspector, background drag still pans.
  2. Backward edges arc above/below nodes; clicking a backward edge still selects it.
  3. Selecting any edge shows it white with a glow.
  4. Selecting a forward edge shows a handle; drag bends; double-click removes the bend.
