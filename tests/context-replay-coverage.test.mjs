import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";

const scripts = ["decision-investigation.js", "context-replay-coverage.js"]
  .map((name) => readFileSync(new URL("../ui/" + name, import.meta.url), "utf8"));

test("counts each linked input once per decision and keeps measurement and readiness separate", () => {
  const positionKey = (link) => JSON.stringify([link.run_id, link.call_id]);
  const inputKey = (link) => JSON.stringify([link.run_id, link.call_id, link.decision_id, link.context_id]);
  const context = { id: "policy", availability: "included", text: "Use the current policy" };
  const callIndex = new Map();
  const captures = new Map();
  const reports = new Map();
  function link(call_id, options = {}) {
    const decision = { id: options.decision || "decision", kind: "model_output", outcome: 30 };
    const occurrence = { run_id: "run", call_id, decision_id: decision.id, context_id: context.id };
    const key = positionKey(occurrence);
    if (!callIndex.has(key)) callIndex.set(key, { call: { decisions: [] }, contextItems: new Map([[context.id, context]]) });
    callIndex.get(key).call.decisions.push(decision);
    if (options.fields !== undefined) captures.set(key, {
      request_evidence: { field: "body" },
      entries: new Map([[context.id, { matching_fields: options.fields }]]),
    });
    reports.set(inputKey(occurrence), { status: options.replay || "untested" });
    return occurrence;
  }
  const measured = link("archived", { replay: "measured" });
  const ready = link("captured", { fields: 1 });
  const secondDecision = link("captured", { decision: "other", fields: 1 });
  const failed = link("multiple-fields", { fields: 2, replay: "unavailable" });
  const missing = link("missing-request");
  const unmatched = link("unmatched", { fields: 0 });
  const { contextReplayCoverage, matchesContextReplayFilter } = runInNewContext(
    scripts.join("\n") + "\n({ contextReplayCoverage, matchesContextReplayFilter });",
    {
      contextCallIndex: callIndex,
      contextCallKey: positionKey,
      contextReplayKey: inputKey,
      contextReplay: (item) => reports.get(inputKey(item)),
      captureOnCall: (run_id, call_id) => captures.get(positionKey({ run_id, call_id })),
    },
  );
  const occurrences = [measured, ready, { ...ready, evidence: { index: 7 } }, secondDecision, failed, missing, unmatched];
  assert.deepEqual({ ...contextReplayCoverage(occurrences) }, { measured: 1, ready: 3, gaps: 2, unavailable: 1, total: 6 });
  assert.ok(matchesContextReplayFilter(ready, "ready"));
  assert.ok(matchesContextReplayFilter(failed, "ready"));
  assert.ok(matchesContextReplayFilter(failed, "unavailable"));
  assert.ok(!matchesContextReplayFilter(measured, "gaps"));
  assert.ok(matchesContextReplayFilter(missing, "gaps"));
  assert.ok(matchesContextReplayFilter(unmatched, "gaps"));
});
