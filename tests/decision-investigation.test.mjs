import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";

const script = readFileSync(new URL("../ui/decision-investigation.js", import.meta.url), "utf8");
const maskScript = readFileSync(new URL("../ui/source-masks.js", import.meta.url), "utf8");
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

test("keeps zero mask effects, pair interactions, and unmeasured inputs distinct", () => {
  const masks = runInNewContext(maskScript + "\nsourceMaskMeasurements;");
  const report = {
    id: "probe", location: { run_id: "run", call_id: "call", decision_id: "selected" },
    baseline_log_probs: [-1, -1, -1],
    sources: [
      { context_id: "policy", masked_log_probs: [-1, -1, -1] },
      { context_id: "copy", masked_log_probs: [-1, -2, -1] },
      { context_id: "missing", unavailable: "text_not_captured" },
    ],
    pairs: [{ context_ids: ["policy", "copy"], masked_log_probs: [-1, -2, -4] }],
  };
  const measured = masks(report);
  assert.equal(measured.length, 2);
  assert.deepEqual({ ...measured[0].mask }, { token: 0, effect: 0 });
  assert.deepEqual({ ...measured[1].mask }, { token: 1, effect: -1 });
  assert.deepEqual({ ...measured[0].interaction }, { token: 2, effect: -3, pair: 0 });
  assert.deepEqual({ ...measured[1].interaction }, { token: 2, effect: -3, pair: 0 });
  const sources = ["policy", "copy", "missing"].map((id) => input(id));
  const rows = collect({ context: sources, relationships: [] }, { id: "selected", kind: "model_output", outcome: "4" },
    undefined, [], [], () => noReplay, (id) => measured.filter(({ context_id }) => context_id === id));
  assert.equal(rows[0].masks.length, 1);
  assert.equal(rows[0].replay.status, "untested");
  assert.equal(rows[2].masks.length, 0);
});
