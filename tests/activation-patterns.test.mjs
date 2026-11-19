import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";

const source = readFileSync(new URL("../ui/activation-patterns.js", import.meta.url), "utf-8");
const app = readFileSync(new URL("../ui/app.js", import.meta.url), "utf-8");

test("counts each decision once and requires agreement for a repeated measurement", () => {
  const report = (decision_id, after) => ({
    location: { run_id: "run", call_id: "call", decision_id },
    baseline_margin: 2,
    patched_margins: [after],
  });
  const occurrences = [report("repeat", -1), report("repeat", 3), report("once", -2)];
  const summary = runInNewContext(source + "\nsummarizeActivationEffects(occurrences, 0);", { occurrences });
  assert.deepEqual({ ...summary }, { minimum: -4, maximum: 1, lower: 1, flipped: 1, decisions: 2 });
});

test("compares UTC days and keeps unmatched rows visible in the other period", () => {
  const { activationPeriod, activationGroupsByPeriod } = runInNewContext(app + "\n" + source
    + "\n({ activationPeriod, activationGroupsByPeriod });", { document: { addEventListener() {} } });
  const occurrence = (context_id, started_at) => ({ context_id, started_at });
  const groups = [{
    source_id: "policy", checkpoint: { label: "Return window" },
    occurrences: [
      occurrence("last-in-a", "2026-10-31T23:59:59.999999999Z"),
      occurrence("first-in-b", "2026-11-01T01:00:00+01:00"),
      occurrence("untimed", null),
    ],
  }, {
    source_id: "policy", checkpoint: { label: "Refund window" },
    occurrences: [occurrence("only-b", "2026-11-01T00:30:00Z")],
  }];
  const periods = [activationPeriod("", "2026-10-31"), activationPeriod("2026-11-01", "2026-11-01")];
  const scope = { organization: null, user: null };
  const result = activationGroupsByPeriod(groups, periods.map((period) => ({ scope, period })), "policy");
  const ids = (occurrences) => Array.from(occurrences, (item) => item.context_id);
  assert.equal(result.length, 2);
  assert.deepEqual(ids(result[0].cohorts[0]), ["last-in-a"]);
  assert.deepEqual(ids(result[0].cohorts[1]), ["first-in-b"]);
  assert.deepEqual(ids(result[1].cohorts[0]), []);
  assert.deepEqual(ids(result[1].cohorts[1]), ["only-b"]);
  const all = activationGroupsByPeriod(groups, [{ scope, period: activationPeriod("", "") }], "untimed");
  assert.equal(all[0].cohorts[0].length, 3);
  assert.equal(activationPeriod("2026-11-01", "2026-10-31").valid, false);
});

test("aligns matching interventions across scopes while preserving one-sided groups", () => {
  const { activationPeriod, activationGroupsByPeriod } = runInNewContext(app + "\n" + source
    + "\n({ activationPeriod, activationGroupsByPeriod });", { document: { addEventListener() {} } });
  const reported = (organization_id, user_id) => ({ organization_id, user_id });
  const groups = [{ source_id: "policy", checkpoint: { label: "Return window" },
    occurrences: [reported("north", "sam"), reported("north", "lee"), reported("south", "sam")] },
    { source_id: "policy", checkpoint: { label: "Refund timing" }, occurrences: [reported("south", "pat")] }];
  const period = activationPeriod("", "");
  const rows = activationGroupsByPeriod(groups, [
    { scope: { organization: "north", user: "sam" }, period },
    { scope: { organization: "south", user: null }, period },
  ], "policy");
  assert.equal(rows.length, 2);
  assert.deepEqual(Array.from(rows[0].cohorts, side => side.length), [1, 1]);
  assert.deepEqual(Array.from(rows[1].cohorts, side => side.length), [0, 1]);
});
