# Workflow Run Supervisor

**Date:** 2026-03-26
**Type:** Product planning and UX architecture
**Primary surfaces:** `crates/workflow_ui/runs.rs`, `crates/workflow_ui/canvas.rs`, `crates/workflow_ui/client.rs`, `crates/orchestration/src/orchestration.rs`
**Depends on:** `NEO-10`, `NEO-11`, `NEO-13`

---

## Summary

Workflow Run Supervisor should become the default run-detail experience layered on top of the existing workflow runs list and run canvas. Its purpose is to make long-running and multi-agent work observable, reviewable, and interruptible inside the editor without replacing the current `WorkflowRunsView` and `WorkflowCanvas` direction.

The supervisor should answer five questions at a glance: what is running, which node or agent owns the current step, what artifacts have been produced, whether human review is required, and what action the user should take next.

## Goals

* Keep workflow runs visible as a primary product surface rather than a hidden power-user tool
* Turn run monitoring into a first-class UX for active, failed, and completed workflows
* Expose artifacts, evidence, and approvals next to the run graph instead of burying them in terminal output
* Fit cleanly into the current sidebar list plus center-pane canvas architecture
* Leave room for future trust-tier and artifact-first work without blocking on them for the first read-only supervisor pass

## Non-Goals

* Do not replace the existing workflow run list, picker, or canvas with an entirely different navigation model
* Do not define the final trust-tier policy system here; that remains dependency work for `NEO-10`
* Do not define the canonical artifact schema here; that remains dependency work for `NEO-11`
* Do not change workflow execution semantics or add hosted-runtime-only behavior without separate review

## Primary User Flows

### 1. Start or open a run

The user starts from the workflow runs sidebar or opens an existing run row. Instead of seeing only a bare node graph, the editor opens a run supervisor shell around the existing run canvas. The shell shows a run summary header with title, workflow name, source repo, overall status, remote target or workspace when available, and a next-action chip.

### 2. Monitor active work

While a run is executing, the center pane remains the workflow graph so the mental model stays aligned with the definition. A secondary rail shows the active node, node owner or session, elapsed time, last artifact emitted, and whether the run is waiting on an approval, retry, or human interrupt.

### 3. Handle approvals and interruptions

When a node or run requires human input, the supervisor promotes that state into a top-level callout instead of leaving it implicit in node status. The user can inspect the relevant evidence bundle before approving, requesting changes, retrying, or escalating. This should feel like a queue of required decisions, not scattered toast notifications.

### 4. Inspect the artifact trail

Selecting a node or run-level event switches the detail area between conversation output, artifacts, evidence, and logs. The current `TaskNodeStatus.artifacts` map becomes visible as structured read-only cards rather than raw hidden payloads. The artifact trail should make it obvious what each node produced and what downstream steps consumed.

### 5. Complete, fail, or hand off

When a run reaches a terminal state, the supervisor collapses into a summary that shows outcome, retained artifacts, failure cause, and next recommended action such as reopen conversation, inspect evidence, retry, or archive. Completed runs should remain useful as an audit trail, not just historical rows.

## Core UI Surfaces

### Workflow runs sidebar

Extend the current `WorkflowRunsView` rather than creating a new navigation entry. The sidebar should continue to group runs by lifecycle status, but gain lightweight supervision affordances:

* a review-needed badge or chip when a run is blocked on human action
* artifact count or evidence count on hover or in muted metadata
* filter chips for Needs Review, Running, Failed, and Completed once the metadata exists
* stronger distinction between passive historical rows and runs that need attention now

The sidebar remains the inbox; the supervisor is the detail surface.

### Supervisor shell in the center pane

Build the supervisor as a shell around the existing run-mode canvas instead of a separate item type. Recommended layout:

* top header: run identity, status, next action, source repo, workspace or remote target
* center body: existing workflow graph canvas
* right rail: current node summary, node owner or session, review state, last artifact emitted
* lower detail tabs or drawer: Conversation, Artifacts, Evidence, Logs

This preserves the graph as the primary orientation tool while adding room for human supervision.

### Approval and interrupt affordances

Approval UX should appear at both the run level and node level. The user should be able to see:

* which node is waiting
* why it is waiting
* what evidence is attached
* whether the pause is a policy requirement, an explicit workflow review gate, or a failure requiring intervention

This should integrate with the review-queue and trust-tier direction in `NEO-10`, but the first plan pass should assume a read-only display if action endpoints are not ready yet.

### Artifact viewer

The artifact viewer should treat workflow outputs as first-class assets, especially:

* sprint contract
* plan
* assumptions
* diff summary
* failing test report
* evidence links
* handoff note
* review decision
* release note draft

Markdown, plain text, and JSON summaries should render in editor-native readers so artifacts can be inspected with the same trust model as code and conversations.

## Approval State Model

The current `TaskLifecycleStatus` enum in `crates/workflow_ui/client.rs` only models execution lifecycle: queued, running, completed, failed. That should remain true. The supervisor needs an orthogonal supervision state rather than overloading lifecycle status.

Recommended additions at task and node scope:

* `review_state`: `none`, `needs_review`, `approved`, `changes_requested`, `escalated`
* `next_required_action`: `observe`, `approve`, `resume`, `retry`, `open_artifact`, `resolve_failure`
* `blocking_reason`: policy gate, artifact missing, validation failure, manual interrupt, or runtime error

This separation avoids a confusing status explosion where lifecycle and review semantics are mixed into one field.

## Artifact and Data Contract Expectations

The current `TaskNodeStatus.artifacts: BTreeMap<String, serde_json::Value>` is a good compatibility foothold, but the supervisor should not depend entirely on anonymous JSON blobs. The runtime should augment `/tasks/{id}/status` with typed summaries that the editor can render predictably.

Recommended task-level additions:

* run summary metadata for current step and next required action
* a normalized list of pending reviews or interrupts
* artifact summaries with stable ids, titles, producer node ids, preview kind, and timestamps
* optional evidence metadata for tests, logs, diffs, or links

Recommended node-level additions:

* review state
* last updated timestamp
* artifact summary list derived from the raw artifact payloads
* actor metadata for the agent, session, or workspace that produced the step

Retain the raw artifact map for forward compatibility, but plan the UI around typed summaries.

## Architecture Fit With Existing Workflow Direction

### `crates/workflow_ui/client.rs`

This is the first dependency point. It needs typed response structs for supervisor metadata, review state, artifact summaries, and any task-level pending-action fields. The client should continue to tolerate missing fields so the read-only supervisor can ship progressively.

### `crates/workflow_ui/runs.rs`

This remains the workflow inbox. It should show supervision-oriented grouping and badges, but it should not absorb the full approval UI. Keep the list lightweight and route detail work into the center-pane supervisor.

### `crates/workflow_ui/canvas.rs`

This should evolve from a bare run graph into the main supervisor shell. The current run-mode canvas already knows about node activation, failure toasts, conversations, workspace attachment, and run polling. That makes it the right anchor point for the richer run summary, node detail rail, and artifact tabs.

### `crates/workflow_ui/inspector.rs`

Reuse the node inspector mental model for node detail rather than inventing a second node metadata surface. The supervisor rail and inspector should either share components or agree on a clear split: inspector for workflow-definition editing, supervisor rail for live run-state metadata.

### `crates/orchestration/src/orchestration.rs`

Keep the higher-level orchestration state distinct from per-run runtime state. `WorkflowRun`, `WorkflowStep`, and `WorkflowStatus` already model the editor's long-lived planning and execution timeline, so the supervisor should enrich the runtime detail view without collapsing orchestration-level step state and task-lifecycle transport state into one enum.

### `crates/sidebar/src/sidebar.rs`

Only minimal wiring should be required here. The sidebar owns navigation and badging, not full run supervision. Avoid introducing yet another workflow-specific panel.

## Dependencies

* `NEO-10` for the policy vocabulary behind trust tiers, escalations, and approval routing
* `NEO-11` for the canonical artifact schema and handoff expectations
* `NEO-13` for sprint-contract and milestone artifacts that the supervisor should surface
* Current implementation anchor points in `crates/workflow_ui/runs.rs`, `crates/workflow_ui/canvas.rs`, `crates/workflow_ui/client.rs`, and `crates/orchestration/src/orchestration.rs`; the earlier workflow-run and sidebar spec docs referenced in prior notes were not present in this checkout, so future planning should either land those documents or update issue references to the checked-in source of truth

## Recommended Rollout

### Phase 1: Read-only supervisor shell

Add the shell around the current run canvas using only data already available or easy to add without action endpoints:

* run summary header
* active-node rail
* artifact and conversation tabs
* review-needed badges when runtime metadata exists

### Phase 2: Review state and queue integration

Once the runtime exposes structured review metadata, add:

* pending review cards
* explicit next-action chips
* sidebar attention grouping
* richer failure and escalation messaging

### Phase 3: Actionable approvals and artifact-first flows

After the trust-tier and artifact planning work lands, add:

* approve / request changes / retry controls
* policy-aware escalation affordances
* richer artifact bundles and evidence drill-down
* milestone or contract checkpoints in the run timeline

## Risks

* The canvas can become visually overloaded if status, approvals, and artifacts compete with graph comprehension
* If lifecycle and review states are merged incorrectly, the resulting UX will be hard to reason about and hard to test
* Artifact-first UI will become noisy unless a small required schema exists for high-value artifact types
* Remote workspace and hosted-runtime metadata can blur the open-editor versus hosted-service boundary if surfaced without policy review
* The planning memo path cited in this issue and the earlier workflow-run/sidebar spec references were not present in this checkout, so future planning work should keep the checked-in source of truth synchronized with the issue references

## Verification Strategy

* Add serde coverage for new task and node supervisor payloads in `crates/workflow_ui/client.rs`
* Add GPUI tests for workflow-run grouping, review badges, and next-action display in `crates/workflow_ui/runs.rs`
* Add canvas or component tests for switching between conversation, artifacts, evidence, and logs in run mode
* Build runtime fixtures for the critical states: running with no review, running with pending approval, failed with evidence, completed with artifact bundle, and remote workspace attachment
* Run manual smoke tests against the editor with those fixtures to confirm the supervisor stays legible on both active and terminal runs

## Recommendation

Treat the Workflow Run Supervisor as the shell that turns NeoZed's current workflow system into an editor-native mission-control surface. Keep the list-plus-canvas architecture, add an orthogonal supervision model for approvals and next actions, and make artifacts visible before adding more autonomous behavior.
