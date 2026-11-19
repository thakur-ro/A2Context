# Conversation-first inspector

The [inference attribution plan](inference-attribution-plan.md) tracks the larger research goal of measuring how context influences a decision during inference.

The inspector should open on a conversation. A developer should be able to read what the user asked and what the workflow did, then open the evidence for a specific turn. Recurring patterns should lead back to that same turn and excerpt.

## What the interface shows

| Level | Main view | Drill-down |
| --- | --- | --- |
| **Turn** | The user message and recorded answer or action. Show compact indicators for conflicts, matching context, and replay results when present. | Select a decision within the turn. Open its sources, exact excerpts, tool and retrieval steps, and any replay outcomes. |
| **Conversation** | A readable transcript in order. Fold execution steps into their turn. Mark where a context version, rule, or outcome changed. | Select a turn to open its evidence in a side pane. On mobile, expand that evidence below the turn. |
| **User** | Recent conversations and patterns that recur across them, such as the same rule conflict or the same input linked to a decision. | Open the contributing conversations, then the affected turn. Show both the conversation count and the evidence coverage. |
| **Organization** | A table of patterns grouped by workflow, model, context source or rule version, with an optional time view. | Open a group to see the users and conversations contributing to it, subject to the workspace's access policy. |

Use a quiet Scandinavian design direction. Give the transcript a comfortable reading width and keep the filters, source identifiers, and trace paths in the surrounding inspector. Use warm neutral surfaces, hairline dividers, readable typography, one muted blue for selection, and a labeled amber treatment for conflicts. Avoid putting each message in a bordered card. Keep keyboard focus and the selected turn visible when the pane opens or closes.

Keep the execution tree, source–decision matrix, comparisons, and replay controls available from the conversation or pattern they explain. Preserve existing shared links and let new links open a precise turn, decision, or excerpt. Preserve the selected turn when switching from the transcript to execution order and back.

## What the evidence supports

Every turn summary and pattern result should link to its underlying evidence. Use four labels consistently.

- **Present** means the input appeared in the captured request or trace.
- **Linked** means a recorded relationship or exact match connects the input to an outcome. Show which kind of link it is.
- **Conflict** means annotated rules disagree within the decision's declared scope. Show the conflicting values and both sources.
- **Measured** means a specified replay edit changed or preserved an outcome under the recorded conditions. Show the edit, baseline, counts, and matching rule. Keep unavailable and untested results explicit.

Use declared conversation and turn identifiers when available. Within the same scoped conversation, group calls under the declared turn and order them by captured timestamps. Preserve recorded order where timestamps are absent. If an input has no reliable turn identity, keep the individual call visible with its assignment marked unknown. Link spans and tool results using their recorded IDs. Avoid making repeated message history look like new user turns. For a routing or batch workflow with no user messages, use the same view as an ordered list of recorded steps and outcomes.

## Configurable views and agent tools

Put the hierarchy and filters in a versioned `ViewSpec`. In the web UI, people edit it with a small **Group by** control. Through the API, an agent can discover the available fields and submit the same spec. The Rust query layer resolves registered dimensions, computes measures, and returns rows with counts, denominators, and references to contributing evidence. The renderer chooses a transcript, breakdown table, trend, or evidence pane from trusted components.

The people view follows organization → user → conversation → turn. Analytical views can group the same evidence by model, named source, source version, rule, and time. Add workflow when a producer supplies an explicitly mapped key. A plugin can register a typed field from allowlisted trace attributes and define how missing values are treated. Allow users or agents to arrange these dimensions in a view. Keep organization and user identity scoped and show unknown values and capture gaps explicitly.

The first query contract covers rule conflicts. For “Show return-policy conflicts by model and source version since November 1,” the agent can send this view.

```json
{
  "version": 1,
  "scope": {"kind": "organization", "organization_id": "Northland Retail"},
  "filters": {"after": "2026-11-01T00:00:00Z", "rule": "return_window_days"},
  "group_by": ["model", "source_version"]
}
```

Define every measure with its unit of counting and eligibility. For example, a repeated rule-conflict count deduplicates by decision and rule. A coverage measure reports how many decisions carried a rule scope. Two replays are comparable only when their model and runner, edit, outcome match rule, and other experimental settings support that comparison. Rows carry a query scope and a cursor for drill-down.

The conflict breakdown and stdio tools are available in the local pilot. The guide documents the current schema and client setup. [Connect an MCP client](guide.md#connect-an-mcp-client)

### Integration surface

- **Rust / JSON.** Add a versioned, paginated query interface that serves the web UI and adapters. Keep the evidence schema as the ingestion contract and reuse the existing OTLP, OpenInference, and LangSmith importers. Return evidence IDs and links in compact summaries. Fetch excerpts only when requested and allowed.
- **MCP.** Start with read-only tools such as `describe_dimensions`, `query_patterns`, and `get_turn_evidence`. The host chooses the authorized workspace. Calling a tool can query a view and return evidence references. Saving a view and running a replay remain explicit user actions. [MCP tools specification](https://modelcontextprotocol.io/specification/2025-11-25/server/tools)
- **MCP Apps.** Offer one embedded result view that opens a pattern and drills into its contributing turns. Hosts that implement the extension can render it inline. Structured results and a local inspector link cover other hosts. [MCP Apps extension](https://apps.extensions.modelcontextprotocol.io/api/)
- **A2UI.** Add an optional renderer adapter after the query contract and web components are proven. A host that supports the catalog can render the transcript, evidence, and group results. Keep styling in the host and let agents specify component data through the catalog. [A2UI catalogs](https://a2ui.org/concepts/components/)

Apply scope and field access before aggregating counts, returning excerpts, or resolving a link. Imported prompt and tool text is trace data. It cannot choose dimensions or invoke actions. Keep the pilot local and expose only read-only queries to an agent. A shared deployment would require authenticated access and verified tenant isolation.

## Build order and acceptance

1. **Ship a conversation slice.** Build the transcript and turn evidence pane using the existing Rust inspector and browser assets. Use a synthetic support conversation with two turns and a linked tool step. Add a fixture with missing turn IDs. Verify that a developer can open either turn, find the source for an observed decision, inspect a conflict, and return to the same scroll position on desktop and mobile. Check that old decision and timeline links still resolve.
2. **Make patterns link to the conversation.** Add user and organization summaries with one shared group-by control. A row must open the contributing conversations and then the exact turn. Verify counts and conflict coverage against synthetic fixtures, including the same user ID in two organizations. Verify what an imported trace with missing identities can display.
3. **Expose the view contract and MCP tools.** Move the proven groupings to the shared Rust query layer. Reproduce the same group counts and evidence IDs from the browser and MCP for a single spec. Test custom field registration, unknown dimensions, bounded result sizes, and scope enforcement.
4. **Prototype embedded components.** Render a read-only pattern drill-down in one MCP Apps host. Then assess an A2UI adapter against the same reference task and retain a structured response for hosts without renderers. Bring multi-call replay planning back into the pattern view once developers can reliably discover and review the supporting evidence.

The first usability test has three tasks. Find the instruction linked to the second answer, explain a rule conflict from its two sources, and find another conversation where it occurred. Record the time and number of steps, where participants get stuck, and whether they distinguish a recorded link from a measured replay effect. Compare the same tasks against the current inspector.
