# Workflow Runtime Port Catalog Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `neo-zed` orchestration consume a runtime-provided node type catalog with explicit input/output ports, use port-aware edges, and replace explicit select/connect/pan modes with direct manipulation.

**Architecture:** The runtime becomes the source of truth for node types, port definitions, and workflow edge endpoints. `neo-zed` fetches the catalog, renders per-port handles, and updates canvas interactions so empty-space drag pans and output-to-input drag creates connections.

**Tech Stack:** Rust runtime API/types/tests, Rust `workflow_ui` client/canvas tests, GPUI interaction tests, OpenAPI-backed JSON models.

---

### Task 1: Add runtime contract coverage for node type catalog and port-aware workflows

**Files:**
- Modify: `/Users/nest/Developer/neo-zed-runtime/tests/api_runtime.rs`
- Modify: `/Users/nest/Developer/neo-zed-runtime/tests/storage_reactivity.rs`
- Reference: `/Users/nest/Developer/neo-zed-runtime/src/api/types.rs`

- [ ] **Step 1: Write failing runtime API and storage tests**

Add tests that expect:
- `GET /workflow-node-types` returns runtime-defined node types with `inputs` and `outputs`
- workflow create/update payloads accept `node_type` on nodes and port-qualified edges
- stored workflows round-trip the new schema without losing port ids

- [ ] **Step 2: Run the targeted runtime tests to verify they fail for missing catalog/schema**

Run: `cargo test --manifest-path /Users/nest/Developer/neo-zed-runtime/Cargo.toml api_runtime storage_reactivity -- --nocapture`

Expected: FAIL because the endpoint and new fields do not exist yet.

- [ ] **Step 3: Implement the minimal runtime type changes**

Update runtime API/storage models so:
- node types are catalog entries with stable ids, labels, and explicit input/output ports
- workflow nodes reference a `node_type` instead of a hardcoded enum kind
- workflow edges include source and target port ids

- [ ] **Step 4: Wire runtime state/routes/OpenAPI to serve the catalog and persist the new workflow shape**

Add the catalog endpoint, route registration, schema exposure, and default in-memory catalog data.

- [ ] **Step 5: Re-run targeted runtime tests**

Run: `cargo test --manifest-path /Users/nest/Developer/neo-zed-runtime/Cargo.toml api_runtime storage_reactivity -- --nocapture`

Expected: PASS.

### Task 2: Update `neo-zed` workflow client models to consume the runtime catalog

**Files:**
- Modify: `/Users/nest/Developer/neo-zed/crates/workflow_ui/client.rs`
- Modify: `/Users/nest/Developer/neo-zed/crates/workflow_ui/workflow_ui.rs`
- Reference: `/Users/nest/Developer/neo-zed-runtime/src/api/types.rs`

- [ ] **Step 1: Write failing `workflow_ui` model/client tests**

Add tests that expect:
- catalog JSON deserializes into `WorkflowNodeType` plus port definitions
- workflow definitions deserialize nodes with `node_type`
- edges deserialize source/target port ids

- [ ] **Step 2: Run the targeted `workflow_ui` tests to verify they fail**

Run: `cargo test -p workflow_ui client::tests -- --nocapture`

Expected: FAIL because the client structs and fetch methods do not support the new schema.

- [ ] **Step 3: Implement the minimal client/model changes**

Add runtime-backed node type and port structs, update workflow node/edge structs, and add a client call for the node type catalog endpoint.

- [ ] **Step 4: Re-run targeted `workflow_ui` client tests**

Run: `cargo test -p workflow_ui client::tests -- --nocapture`

Expected: PASS.

### Task 3: Replace mode buttons with direct pan and port drag behavior in the `neo-zed` canvas

**Files:**
- Modify: `/Users/nest/Developer/neo-zed/crates/workflow_ui/canvas.rs`
- Modify: `/Users/nest/Developer/neo-zed/crates/workflow_ui/inspector.rs` if palette data or inspector seeding needs catalog access
- Test: `/Users/nest/Developer/neo-zed/crates/workflow_ui/canvas.rs`

- [ ] **Step 1: Write failing canvas behavior tests**

Add tests that expect:
- dragging empty canvas space pans without a dedicated pan mode
- clicking a node still selects it and opens the inspector
- dragging from an output handle to an input handle creates a port-qualified edge
- toolbar no longer renders select/connect/pan controls

- [ ] **Step 2: Run the targeted canvas tests to verify they fail**

Run: `cargo test -p workflow_ui canvas::tests -- --nocapture`

Expected: FAIL because the canvas still uses `CanvasMode` and node-wide connection semantics.

- [ ] **Step 3: Implement minimal canvas interaction changes**

Update hit testing and paint logic so:
- node ports are visible on the left/right edges
- output-handle drag begins a pending connection
- input-handle release finalizes an edge with port ids
- empty-space drag pans immediately
- mode buttons and mode state are removed

- [ ] **Step 4: Re-run targeted canvas tests**

Run: `cargo test -p workflow_ui canvas::tests -- --nocapture`

Expected: PASS.

### Task 4: Hook catalog-driven node creation into `neo-zed`

**Files:**
- Modify: `/Users/nest/Developer/neo-zed/crates/workflow_ui/canvas.rs`
- Modify: `/Users/nest/Developer/neo-zed/crates/workflow_ui/picker.rs` if catalog preload belongs there
- Modify: `/Users/nest/Developer/neo-zed/crates/workflow_ui/inspector.rs` if draft creation or workflow open paths need catalog seeding

- [ ] **Step 1: Write failing tests for catalog-backed node creation**

Add tests that expect added nodes use runtime node type ids and default ports from the fetched catalog rather than the old hardcoded enum palette.

- [ ] **Step 2: Run the targeted tests to verify they fail**

Run: `cargo test -p workflow_ui canvas::tests client::tests -- --nocapture`

Expected: FAIL because the add-node palette still hardcodes `WorkflowNodeKind`.

- [ ] **Step 3: Implement minimal catalog-backed creation**

Use fetched node types to drive add-node actions and default node instance creation in `neo-zed`.

- [ ] **Step 4: Re-run the focused `workflow_ui` suite**

Run: `cargo test -p workflow_ui -- --nocapture`

Expected: PASS.

### Task 5: Final verification

**Files:**
- Verify only

- [ ] **Step 1: Format both codebases**

Run:
- `cargo fmt -p workflow_ui`
- `cargo fmt --manifest-path /Users/nest/Developer/neo-zed-runtime/Cargo.toml`

- [ ] **Step 2: Run final targeted verification**

Run:
- `cargo test --manifest-path /Users/nest/Developer/neo-zed-runtime/Cargo.toml api_runtime storage_reactivity -- --nocapture`
- `cargo test -p workflow_ui -- --nocapture`

Expected: PASS.
