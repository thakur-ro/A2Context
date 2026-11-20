import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";

const app = readFileSync(new URL("../ui/app.js", import.meta.url), "utf-8");
const { replayScope } = runInNewContext(app + "\n({ replayScope });", { document: { addEventListener() {} } });

test("replay scopes use the normalized nanosecond timestamp and the full conversation identity", () => {
  const boundary = BigInt(Date.parse("2026-11-20T00:00:00Z")) * 1_000_000n;
  const occurrence = (id, time, organization_id = "journal-north") => ({
    id, organization_id, user_id: "support-17", trace_source: "fixture", conversation_id: "returns",
    started_at_unix_nanos: time?.toString(),
  });
  const groups = [{ index: 0, occurrences: [occurrence("first-in-b", boundary),
    occurrence("in-another-org", boundary - 1n, "journal-south"), occurrence("last-in-a", boundary - 1n),
    { ...occurrence("unknown-time"), started_at: "11/20/2026" }] }];
  const thread = JSON.stringify(["fixture", "journal-north", "support-17", "returns"]);
  const ids = (selection) => Array.from(replayScope(groups, thread, selection).get(0), (item) => item.id);
  assert.deepEqual(ids({ before: "2026-11-20T00:00:00Z" }), ["last-in-a"]);
  assert.deepEqual(ids({ after: "2026-11-20T00:00:00Z" }), ["first-in-b"]);
  assert.deepEqual(ids({}), ["last-in-a", "first-in-b", "unknown-time"]);
});
