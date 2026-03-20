# Agent Workflow Canvas Design

Date: 2026-03-20
Status: Draft approved for spec write-up

## Goal

Design a host-native orchestration system in Zed with two equally important parts:

- a visual workflow canvas for authoring AI-agent workflows
- an event-driven runtime that can execute those workflows durably in the foreground or background

The system should complement text-thread workflows and also support background automation. The initial target is AI-agent workflows rather than general automation graphs.

## Product Shape

The first version is an orchestration platform for AI-agent workflows with:

- reusable workflow templates
- separate workflow runs instantiated from templates
- event-driven execution
- full replayable run history
- checkpoints, resume, and fork
- human approval gates
- background terminals owned by runs
- integration with existing Zed review and terminal surfaces

Deferred from v1:

- Linear integration
- Slack or other outbound notification integrations
- true multi-project runs
- general automation/integration nodes

## Core Architectural Decision

Use a split model from day one:

- define a canonical workflow model and execution ledger shared by both UI and runtime
- make the canvas editor a client of that model
- make the execution engine a separate client of that model

The canvas must not become the runtime model, and the runtime must not invent a hidden model the UI only partially understands.

## High-Level Architecture

The system has four major layers:

1. Template model
   Reusable workflow definitions including graph structure, node configuration, triggers, policies, and project scope.
2. Run model
   Concrete executions bound to one project in v1, with inputs, current state, active resources, and links to checkpoints and artifacts.
3. Execution ledger
   Append-only durable history for every run event and state transition.
4. UI surfaces
   Sidebar navigation plus center items for authoring templates and inspecting runs.

## Object Model

### WorkflowTemplate

Represents a reusable workflow definition.

Contains:

- graph nodes and edges
- node configuration
- trigger subscriptions
- approval and safety policy hooks
- declared artifact inputs and outputs
- declared memory access rules
- allowed project scope

Project scope may be:

- global
- one specific project
- multiple allowed projects

### WorkflowRun

Represents one instantiated execution of a template.

Rules for v1:

- every run is bound to exactly one project
- the same template may be run in multiple projects across different runs
- runs are the unit of project context selection for the rest of the workspace

Contains:

- template reference
- bound project
- bound inputs
- status
- active node state
- active approvals
- active terminals and other owned resources
- checkpoint references

### ExecutionLedger

Append-only event history for a run.

Events include:

- trigger received
- node scheduled
- node started
- node completed
- node failed
- retry requested
- approval requested
- approval granted or denied
- artifact emitted
- working memory updated
- terminal opened
- terminal cancelled
- checkpoint created
- run resumed
- run forked
- run cancelled

### Checkpoint

Durable recovery boundary derived from ledger state.

Used for:

- resume
- replay
- fork
- inspection

The system should preserve enough state to restart or inspect a run without relying on transient UI state.

## Runtime Semantics

### Execution Style

The runtime is event-driven.

It advances by:

- consuming events
- applying them to the run state and ledger
- determining which nodes are eligible
- scheduling bounded execution work

### Node Granularity

The first version targets fine-grained primitives rather than only coarse workflow stages.

This allows the graph to express:

- LLM calls
- tool use
- memory read and write
- artifact read and write
- branching
- event waits
- handoffs
- approvals

Agent nodes may still contain bounded internal autonomy, but the graph remains the durable control structure.

### Autonomy Model

Use a hybrid model:

- graph edges define major control flow and durability boundaries
- some agent-capable nodes may perform bounded internal reasoning and tool use

The runtime should not allow unconstrained node behavior to bypass explicit graph or policy boundaries.

## Handoff Model

The canonical handoff between nodes in v1 is:

- typed artifacts
- a run-scoped working memory object
- run-owned background terminals when continuity matters

Threads are not special execution state. They are artifact refs like any other output.

### Typed Artifacts

Example artifact categories:

- spec
- implementation plan
- handoff bundle
- review report
- patch metadata
- thread reference
- commit reference

Artifacts are durable and typed so later nodes can consume them predictably.

### Working Memory

The run owns a shared working memory/context object.

This is for run-scoped context that does not belong in immutable artifacts, such as:

- derived context
- intermediate decisions
- current assumptions
- run-local variables

### Background Terminals

Runs may own background terminals.

Support both:

- ephemeral terminals for one node
- persistent named terminals for reuse across a run

These terminals must be first-class workspace/editor terminals with orchestration metadata, not hidden runtime internals.

Users must be able to:

- inspect them
- focus them
- cancel them

from normal terminal/editor surfaces as well as from the run view.

## Navigation And Surface Model

### Sidebar

The orchestration tree replaces the current primary sidebar experience.

The sidebar becomes the orchestration control plane and the workspace context selector.

Hierarchy:

- Workspace
- Workflows
- Runs

Selecting a sidebar node changes what the rest of the workspace is centered on.

### Center Items

There are two center item types:

- WorkflowTemplate item
  The editable graph canvas used to create and modify workflow templates.
- WorkflowRun item
  The execution inspector used to inspect state, events, resources, approvals, and checkpoints for a concrete run.

### Context Switching

Selecting a run should update the active workspace context used by surrounding surfaces.

For v1, selecting a run should determine:

- active project context
- terminal cwd defaults
- git context
- related file navigation context
- related chat/history context where applicable

Selecting a workflow template should open the template item. Since templates may be global or project-scoped, template selection must preserve the template’s binding model without forcing true multi-project execution semantics into v1.

## Relationship To Existing Zed Surfaces

This system should reuse existing detailed work surfaces instead of replacing them.

### Review

Detailed code review should hand off into existing Zed review surfaces, especially agent diff.

The run view should provide:

- orchestration summary
- provenance
- artifact links
- explanation of why a change exists

The deep diff experience should stay in existing review surfaces.

### Terminals

Run-owned terminals should behave like normal terminals in the editor and workspace.

### Sidebar And Workspace Shell

The current orchestration UI placeholder should be evolved toward:

- sidebar orchestration tree
- template center item
- run center item

rather than remaining a single placeholder center item.

## Triggers And Events

The runtime must be designed for a general event bus in v1, even if early triggers are limited.

Trigger categories:

- internal events
- repo and workspace events
- external webhook-compatible events

Examples:

- manual run start
- node completion
- artifact emitted
- validation failure
- file changes
- branch state changes
- diagnostics changes
- test result changes
- future external callbacks

## Approval And Safety Model

Use both explicit graph-level approval nodes and runtime-enforced policy gates.

### Explicit Approval Nodes

Workflow authors can place approval steps directly in the graph.

### Policy Gates

The runtime may inject or enforce pauses for sensitive actions such as:

- destructive edits
- spec divergence
- review-sensitive transitions
- push

### Failure Handling

Use hybrid failure handling:

- graph-defined recovery and retries are allowed within bounded limits
- unresolved failures pause the run for human intervention

### Spec Divergence

If implementation diverges from the spec, the runtime must block by default.

It should not automatically rewrite the spec.

The user decides whether to:

- revise implementation
- create a spec change set

### Cancellation

Cancellation of terminals or other long-lived resources must emit durable ledger events so the runtime never silently loses track of resource state.

## Resume And Fork Semantics

When a run is resumed after human intervention, the user must be able to choose the resume unit.

Supported choices for v1:

- resume from the paused node
- resume from the latest durable checkpoint
- resume or fork from an earlier checkpoint

This choice should be surfaced in the `WorkflowRun` item and recorded in the ledger.

## Completion Semantics

For v1, a run is considered complete when the graph reaches its terminal success state.

Repo outcomes such as commit or push may still be required by a specific workflow, but they should be represented as:

- explicit graph nodes
- policy requirements attached to that template

They should not be treated as global completion rules for all workflows.

## Initial Workflow Example

The motivating workflow shape for v1 includes a loop like:

1. Acquire or generate spec
2. Generate implementation plan with memory injection
3. Implement with memory injection and emit handoff artifacts
4. Validate against the spec and, if necessary, pause for human decision when implementation diverges
5. Review code and security
6. Push

This flow is event-driven and may loop through implementation and validation until the spec is satisfied.

The system must support this pattern without hardcoding these stages as the only workflow form.

## Testing Strategy

### Model Tests

Test:

- template serialization and versioning
- run instantiation
- event application
- checkpoint creation
- resume and fork semantics

### Runtime Tests

Test:

- event scheduling
- node readiness
- approval pauses
- retry limits
- failure escalation
- spec divergence blocking
- working memory and artifact handoff rules
- persistent and ephemeral terminal lifecycle

### UI Tests

Test:

- sidebar tree selection
- opening template items
- opening run items
- context switching from run selection
- checkpoint actions
- terminal visibility and cancellation

### Integration Tests

Test representative end-to-end orchestration flows such as:

- brainstorm to spec
- spec to plan
- plan to implement
- implement to validate
- review and push
- pause, resume, and fork
- handoff into agent diff for detailed review

## Out Of Scope For V1

- Linear integration
- Slack notifications
- true multi-project run execution
- generic automation nodes beyond AI-agent workflow needs
- a brand-new diff or terminal UI replacing existing Zed surfaces

## Migration Direction From Current State

Today the codebase already has:

- a persisted orchestration state model
- a placeholder orchestration item

The v1 design should evolve that into:

- a richer canonical workflow model
- a durable execution ledger
- a left-sidebar orchestration tree
- distinct template and run center items
- runtime-managed resources and checkpoints

## Open Follow-Up Questions For Planning

These do not block the design but should be resolved in implementation planning:

- exact workflow IR shape for nodes, ports, and edges
- whether template selection should set a default project context when not opening a run
- how much of the current sidebar shell can be reused versus replaced
- how run context is exposed to existing file, terminal, git, and chat surfaces
- which concrete trigger sources land in the first milestone
