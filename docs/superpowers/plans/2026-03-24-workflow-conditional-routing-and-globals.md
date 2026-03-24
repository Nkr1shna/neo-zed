# Workflow Conditional Routing And Globals Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add JSON-backed conditional routing and workflow globals authoring to the workflow canvas and node inspector.

**Architecture:** Keep conditionals as compact routing nodes on canvas, synthesize their output ports from typed JSON config, and special-case conditional/globals editing in the inspector instead of stretching the generic text-field editor path. Add a single synthetic globals node type client-side so globals can be authored immediately.

**Tech Stack:** Rust, serde, GPUI, workflow canvas, node inspector, dropdown/context menus

**Spec:** `docs/superpowers/specs/2026-03-24-workflow-conditional-routing-and-globals.md`

---

### Task 1: Add typed config models and tests

**Files:**
- Modify: `crates/workflow_ui/client.rs`

- [ ] Add serde-backed conditional and globals config structs plus defaults.
- [ ] Add tests for round-trip serialization and fallback defaults.
- [ ] Run: `cargo test -p workflow_ui client::tests --lib`

### Task 2: Synthesize ports and globals node type on canvas

**Files:**
- Modify: `crates/workflow_ui/canvas.rs`
- Modify: `crates/workflow_ui/client.rs`

- [ ] Add helpers that derive effective input/output ports from node config when the node is conditional or globals.
- [ ] Add a synthetic globals node type to editor-visible node types when absent.
- [ ] Seed new conditional and globals nodes with sensible default config.
- [ ] Add canvas tests for synthesized conditional ports.
- [ ] Run: `cargo test -p workflow_ui canvas::tests --lib`

### Task 3: Add conditional and globals inspector editors

**Files:**
- Modify: `crates/workflow_ui/inspector.rs`

- [ ] Add inspector state for conditional branches, predicates, and globals variables.
- [ ] Render dropdown-driven operand/operator controls for conditional rows.
- [ ] Render globals variable rows with typed values and mutation flags.
- [ ] Persist edits back into `WorkflowNode.configuration`.
- [ ] Add inspector tests for conditional/globals edit persistence.
- [ ] Run: `cargo test -p workflow_ui inspector::tests --lib`

### Task 4: Run focused verification

**Files:**
- Modify: `crates/workflow_ui/client.rs`
- Modify: `crates/workflow_ui/canvas.rs`
- Modify: `crates/workflow_ui/inspector.rs`

- [ ] Run: `cargo test -p workflow_ui --lib`
- [ ] Run: `cargo check -p workflow_ui -p sidebar`
