import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";

const script = readFileSync(new URL("../ui/decision-investigation.js", import.meta.url), "utf8");
const collect = runInNewContext(script + "\ninvestigationInputs;");
const noReplay = { status: "untested", effects: [] };

function input(id, availability = "included") {
  return { id, availability, text: availability === "included" ? "Captured input " + id : null };
}

test("connects only the selected decision's links, findings, and replay evidence", () => {
  const call = {
    context: [input("tool"), input("policy"), input("preference"), input("other")],
    relationships: [
      { decision_id: "other-decision", context_id: "other", relation: "applies to", basis: "declared" },
      { decision_id: "selected", context_id: "policy", relation: "applies to", basis: "declared" },
    ],
  };
  const replay = { status: "measured", effects: [{}] };
  const rows = collect(call, { kind: "tool_call", id: "selected", outcome: {} }, undefined,
    [{ link: { kind: "tool_definition", context_id: "tool" }, index: 4 }],
    [{ kind: "conflict", context_ids: ["policy", "preference"], finding_index: 2 }],
    (id) => id === "policy" ? replay : noReplay);
  assert.equal(rows[0].signals[0].reference.evidence.index, 4);
  assert.equal(rows[1].signals[0].reference.evidence.kind, "annotation");
  assert.equal(rows[1].signals[0].reference.evidence.index, 1);
  assert.equal(rows[1].signals[1].reference.finding_index, 2);
  assert.equal(rows[1].replay, replay);
  assert.equal(rows[2].signals[0].reference.finding_index, 2);
  assert.equal(rows[3].signals.length, 0);
  assert.ok(rows.every((row) => row.request === "need_request"));
});

test("distinguishes capture gaps, multiple exact fields, and replay eligibility", () => {
  const call = {
    context: [input("exact"), input("multiple"), input("unmatched"), input("ref", "unknown"), input("redacted", "redacted")],
    relationships: [],
  };
  const capture = {
    request_evidence: { field: "request" },
    entries: new Map([
      ["exact", { matching_fields: 1, capture: "available" }],
      ["multiple", { matching_fields: 2, capture: "available" }],
      ["unmatched", { matching_fields: 0, capture: "available" }],
      ["ref", { capture: "reference_only" }],
      ["redacted", { capture: "redacted" }],
    ]),
  };
  const decision = { kind: "model_output", id: "selected", outcome: "30" };
  const rows = collect(call, decision, capture, [], [], () => noReplay);
  assert.deepEqual(Array.from(rows, ({ request }) => request), ["ready", "choose_field", "no_match", "uncaptured", "uncaptured"]);
  assert.equal(rows[3].captureStatus, "reference_only");
  assert.ok(collect(call, { ...decision, outcome: null }, capture, [], [], () => noReplay)
    .every((row) => row.request === "outcome_unavailable"));
  assert.ok(collect(call, { ...decision, kind: "tool_execution" }, capture, [], [], () => noReplay)
    .every((row) => row.request === "tool_execution"));
});
