# Workflow Conditional Routing And Globals

**Date:** 2026-03-24
**Area:** `crates/workflow_ui`
**Depends on:** Existing workflow canvas, node inspector, and HTTP client

---

## Goal

Add a production-grade authoring model for workflow routing conditions and workflow-scoped globals without turning the canvas into a giant inline logic editor.

The canvas should keep conditionals compact and edge-directed:
- conditional nodes route to downstream nodes through named output ports
- the node inspector owns the condition authoring UI
- one workflow-level globals node owns reusable variables

The authoring UI must be explicit and language-agnostic:
- no freeform expression text
- left-hand operand is chosen from a dropdown of valid references
- comparison operator is chosen from a dropdown filtered by operand type
- nested `all` / `any` groups are stored as JSON-backed condition trees

---

## Product Decisions

### Conditional nodes

- A conditional remains a routing node on the canvas.
- It does not embed downstream actions.
- Its `then` targets are represented by named output ports and edges.
- The inspector edits a JSON-backed config that defines:
  - ordered branch outputs
  - branch labels
  - optional conditions per branch
  - nested boolean groups for each branch condition

### Globals node

- Exactly one workflow-level globals node may exist in a workflow.
- The globals node has no inputs and no outputs.
- It acts as a typed key/value store for workflow variables.
- Variables may declare:
  - key
  - value type
  - default value
  - whether runtime override is allowed
  - whether task/runtime mutation is allowed

### Runtime-facing model

- The inspector is not a text DSL.
- The workflow stores typed JSON under `WorkflowNode.configuration`.
- Frontend code owns serialization, deserialization, and sensible defaults.
- Runtime evaluation can later consume the same JSON model directly.

---

## Implementation Shape

### Data model

Extend `crates/workflow_ui/client.rs` with serde-backed types for:
- operand references sourced from connected input metadata or globals
- comparison operators
- predicates
- nested `all` / `any` groups
- conditional branches and their output ids
- globals definitions

### Canvas behavior

- Synthesize conditional output ports from conditional config instead of only from static node-type metadata.
- Add one synthetic globals node type client-side so the workflow editor can author globals without waiting on backend node-type catalog support.
- Keep the node visually compact.

### Inspector behavior

- Special-case conditional nodes and globals nodes instead of forcing them through the existing generic configure-time field loop.
- Use dropdown menus for operand and comparison selection.
- Keep branch editing readable and compact.
- Keep generic configure-time fields for other node types unchanged.

---

## Validation

- Unit tests for serde round-trips and defaults of conditional/globals config.
- Canvas tests proving synthesized conditional branch ports participate in port positioning and edge creation.
- Inspector tests proving conditional/globals edits update workflow JSON correctly.
