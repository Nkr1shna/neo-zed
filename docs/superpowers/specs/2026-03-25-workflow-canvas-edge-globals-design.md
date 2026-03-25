# Workflow Canvas: Edge & Globals Improvements

**Date:** 2026-03-25
**Scope:** `crates/workflow_ui/canvas.rs`, `crates/workflow_ui/inspector.rs`

---

## Summary

Four changes to the workflow canvas UI:

1. Remove the globals node from the canvas; surface globals in the inspector when the canvas background is clicked.
2. Backward edges auto-route above or below all nodes (whichever is shorter) instead of threading through them.
3. Selected edges render with a white glow to indicate selection state.
4. Edges support an optional waypoint: when selected, a drag handle appears at the midpoint allowing users to bend the edge path.

---

## 1. Globals → Canvas Background Inspector

### Behavior

- The globals node (`WORKFLOW_GLOBALS_NODE_TYPE_ID = "workflow_globals"`) is no longer rendered on the canvas. Its node card, ports, and labels are skipped in `paint_node()` in both the editable canvas path and the run-view canvas path.
- The globals node remains present in the workflow data and layout — it is simply invisible and non-interactive as a canvas node.
- `hit_test_node()` skips the globals node. Since `layout.node_positions` is keyed by node ID (not type), the implementation must first find the globals node ID from `self.workflow` (searching `nodes` for `node_type == WORKFLOW_GLOBALS_NODE_TYPE_ID`), cache it, and skip that ID in the hit-test loop.
- `hit_test_port()` likewise skips ports belonging to the globals node, using the same cached globals node ID.
- A background click (no node, no port hit) still sets `pan_mouse_start` so the user can pan by dragging. The globals inspector is opened on `mouse_up` only if no panning occurred (i.e., `pan_mouse_start` was set but `pan_viewport_start` was never used — the mouse did not move). On `mouse_up`, if the drag distance is below a threshold (e.g., 4px), treat it as a click: look up the globals node ID from `self.workflow` and emit `WorkflowCanvasEvent::NodeSelected(Some(globals_node_id))`. If no globals node exists, or if `self.workflow` is `None`, emit `WorkflowCanvasEvent::NodeSelected(None)`. If the drag distance meets the threshold, do nothing on `mouse_up` (panning already occurred).
- This reuses the existing event variant and subscriber infrastructure — no new event variant is added. `sync_node_inspector_panel()` requires no changes.
- The inspector panel opens automatically on background click (same code path as clicking a regular node).

### Files changed

- `canvas.rs`: skip rendering globals node in both canvas paths; skip globals node in `hit_test_node()` and `hit_test_port()`; on background click, emit `NodeSelected(Some(globals_id))` or `NodeSelected(None)`.

---

## 2. Backward Edge Routing: Auto Top/Bottom Arc

### Problem

The current `paint_smoothstep_edge()` routes backward edges (right-to-left) using a fixed elbow: source → right → down → left → target. This path passes behind or through other node cards.

### Solution

Replace the elbow routing with a bounding-box arc approach.

**Bounding box:** Compute `nodes_bbox` once per frame as the union of all node rects in canvas coordinates (width × height anchored at each node's position). Expand by `40px` padding on top and bottom to define the available rail space.

**Side selection:** For each backward edge, use the midpoint of source and destination port Y positions as the reference:

```
mid_y = (src.y + dst.y) / 2
cost_above = mid_y - nodes_bbox.top
cost_below = nodes_bbox.bottom - mid_y
```

Choose the side with the lower cost. On a tie, prefer above.

**Rail offset:** Collect all backward edges and group them by chosen side. Within each side, sort by horizontal span (longest span first — outermost rail). Each edge gets a `rail_index` (0-based). The rail Y coordinate is:

- Above: `rail_y = nodes_bbox.top - padding - (rail_index * 12.0)`
- Below: `rail_y = nodes_bbox.bottom + padding + (rail_index * 12.0)`

**Path shape** (for "above"; below is the vertical mirror):

1. From `src`, extend right by `H_GAP` (16px) to `(src.x + H_GAP, src.y)`.
2. Travel up to `(src.x + H_GAP, rail_y)`.
3. Travel horizontally to `(dst.x - H_GAP, rail_y)`.
4. Travel down to `(dst.x - H_GAP, dst.y)`.
5. Enter `dst` from left: `(dst.x - H_GAP, dst.y)` → `(dst.x, dst.y)`.

Corners are rounded using the existing `paint_smoothstep_polyline()` helper with `corner_radius = 12px`. Arrowhead at `dst` pointing rightward (left-to-right entry, same as forward edges).

**New helper:** `backward_edge_rail_y(mid_y: f32, nodes_bbox: &Bounds<Pixels>, rail_index: usize, above: bool) -> f32` — pure function computing the rail Y for a given index and side.

**Hit testing:** `edge_contains_screen_point()` must be updated to test the new arc polyline geometry for backward edges, instead of the old elbow geometry. Use the same polyline point-list that is passed to `paint_smoothstep_polyline()`, testing each segment with a tolerance of `8px`.

### Files changed

- `canvas.rs`: replace `paint_smoothstep_edge()` body; add `backward_edge_rail_y()`; compute `nodes_bbox` once before painting edges; update `edge_contains_screen_point()` for the new backward edge geometry.

---

## 3. Edge Selection: White Glow

### Visual spec

| Property | Unselected | Selected |
|---|---|---|
| Stroke color | `#9ca3afff` (gray) | `#ffffffff` (white) |
| Stroke width | `2.0px` | `2.5px` |
| Glow layer | none | `6.0px` white stroke at `25%` opacity |

The glow is a second path drawn immediately before the main stroke within the same paint pass (not a separate earlier pass — both glow and stroke are painted after all nodes, in the backward-edge pass or in the forward-edge pass as appropriate, so nodes are never painted on top of the glow).

Arrowhead color follows the same rule: `#9ca3afff` when unselected, `#ffffffff` when selected.

`paint_edge()` receives an additional `is_selected: bool` parameter. To make edge identity available at the call site, `edge_draw_list` (the list built before the paint loop) must be changed from `Vec<(PortPosition, PortPosition)>` to `Vec<(WorkflowEdge, PortPosition, PortPosition)>` so that `is_selected` can be evaluated as `selection == CanvasSelection::Edge(edge.from_node_id, edge.from_output_id, edge.to_node_id, edge.to_input_id)`.

### Files changed

- `canvas.rs`: add `is_selected: bool` to `paint_edge()`; branch on it for colors, stroke width, and glow pre-pass.

---

## 4. Edge Waypoints

### Data model

Waypoints are layout state, not workflow data. `CanvasLayout` gains:

```rust
#[serde(skip)]
edge_waypoints: HashMap<(String, String, String, String), Point<Pixels>>,
```

Waypoints are intentionally transient and must not be persisted. `#[serde(skip)]` expresses this intent and also avoids a serde_json compile error, since tuple keys cannot be serialized as JSON object keys.

The manual `Clone` impl for `CanvasLayout` (which lists each field explicitly) must be updated to include `edge_waypoints: self.edge_waypoints.clone()`.

The key is `(from_node_id, from_output_id, to_node_id, to_input_id)` — matching `CanvasSelection::Edge`.

### Drag state

The existing drag state in `WorkflowCanvas` is stored as a set of `Option<...>` fields (`drag_node`, `drag_node_start_pos`, `drag_mouse_start`, `pan_mouse_start`, `pan_viewport_start`). Follow the same pattern: add two new fields:

```rust
drag_edge_waypoint_key: Option<(String, String, String, String)>,
```

No offset field is needed — the waypoint is set directly to the canvas-space mouse position on each `mouse_move`. Do not introduce a new enum type; keep the existing `Option<...>` field convention.

### Interaction

**Show handle:** When an edge is selected (`CanvasSelection::Edge(...)`), paint a circular handle (`r=5px`, fill `#1e3a5f`, border `#ffffff`, width `1.5px`) at the waypoint position if one exists, or at the geometric midpoint of the edge path otherwise. For forward edges, use the existing `edge_canvas_midpoint()` helper (parametric midpoint of the cubic bezier at `t=0.5`) — this function is currently `#[cfg(test)]`-gated and uses a hardcoded `60.0px` control offset. Before removing the test gate, update it to compute control points using `bezier_ctrl_offset(dx)` where `dx = to.0 - from.0`, matching the logic in `paint_edge()`, so the handle lies exactly on the rendered curve. For backward arc edges, use the midpoint of the horizontal rail segment (the point halfway along the segment connecting step 2 and step 3 of the arc path at `rail_y`).

**Paint ordering:** The handle must appear on top of all node cards. Forward edges are currently painted before nodes, so handle painting cannot occur in the forward-edge pass. Instead, all edge handles are painted in a dedicated fourth pass appended after the existing three passes. The full render sequence becomes: forward edges → nodes → backward edges → **edge handles**. This short fourth pass iterates all edges, checks if any matches the current `CanvasSelection::Edge(...)`, and paints that edge's handle.

**Hit test priority:** `hit_test_edge_handle(screen_point)` checks if the point is within `8px` of the handle center. In `handle_mouse_down`, this check is inserted after the port hit-test guard (so port connections still take highest priority) but before `hit_test_node()`, ensuring a double-click near a node but on a selected edge handle triggers waypoint removal rather than node activation.

**Editable only:** Waypoint dragging is gated on `self.is_editable()`. In the run-view canvas, the handle is not shown and `hit_test_edge_handle()` returns `None`.

**Drag:** On `mouse_down` hitting the handle with `click_count == 1` (and `is_editable()`), set `drag_edge_waypoint_key = Some(edge_key)`. On `mouse_move` while `drag_edge_waypoint_key.is_some()`, update `layout.edge_waypoints[edge_key]` to the current canvas-space mouse position. Call `cx.notify()`. On `mouse_up`, clear `drag_edge_waypoint_key`.

**Edge path with waypoint:** Waypoints only apply to forward edges (cubic bezier paths). Backward arc edges use the polyline rail routing and do not support waypoints — the handle is not shown for a selected backward arc edge.

For forward edges: if `edge_waypoints` contains the edge key, split the edge into two cubic bezier segments: `src → waypoint` and `waypoint → dst`. The arrowhead is at `dst` pointing in the entry direction.

**Remove waypoint:** On `mouse_down` with `click_count == 2` hitting the handle, remove the entry from `edge_waypoints` and call `cx.notify()`. Do not emit any node event.

**Waypoints are cleared** when the workflow is reloaded (since they live in `CanvasLayout` which is rebuilt on load).

### Files changed

- `canvas.rs`: add `edge_waypoints` field to `CanvasLayout` with `#[serde(skip)]`; update manual `Clone` impl; add `drag_edge_waypoint_key: Option<...>` field to `WorkflowCanvas`; add `hit_test_edge_handle()`; update `paint_edge()` for waypoint path and handle rendering; extend `edge_draw_list` to carry `WorkflowEdge`; handle drag in `mouse_down`, `mouse_move`, `mouse_up`.

---

## Out of Scope

- Persisting waypoints to the server.
- Reconnecting edges by dragging endpoints.
- Animated edge flow indicators.
