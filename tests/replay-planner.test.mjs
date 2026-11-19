import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";

const script = readFileSync(new URL("../ui/replay-planner.js", import.meta.url), "utf8");

test("a queued close and an old request cannot reset a newly opened replay planner", async () => {
  const elements = new Map();
  const pending = [];
  const element = (id) => {
    if (!elements.has(id)) elements.set(id, {
      open: false,
      textContent: "",
      reset() {},
      replaceChildren() {},
      showModal() { this.open = true; },
      close() { this.open = false; },
    });
    return elements.get(id);
  };
  const open = runInNewContext(script + "\nopenReplayPlanner;", {
    element,
    hasRecordedRequest: () => true,
    crypto: { randomUUID: () => "case" },
    Option: class {},
    fetch: () => new Promise((resolve) => pending.push(resolve)),
  });
  const dialog = element("replay-planner");
  const loadCapturedRequest = () => element("use-recorded-request").onclick();
  const call = { id: "call", context: [{ id: "input", text: "context", availability: "included" }] };
  const decision = { id: "decision", label: "Observed decision" };
  const response = (id) => ({
    ok: true,
    json: async () => ({
      fields: [],
      evidence_json: JSON.stringify([{
        id,
        calls: [{ id: call.id, recorded_request: { body: "{}", evidence: { field: id + "-body" } } }],
      }]),
    }),
  });

  open({ id: "old" }, call, decision);
  const oldRequest = loadCapturedRequest();
  dialog.close();
  open({ id: "new" }, call, decision);
  const newRequest = loadCapturedRequest();
  dialog.onclose();
  pending[1](response("new"));
  await newRequest;
  const currentSource = element("replay-request-source");
  assert.equal(currentSource.textContent, "Captured on this call · new-body");
  assert.equal(element("download-replay-evidence").hidden, false);

  pending[0](response("old"));
  await oldRequest;
  assert.equal(currentSource.textContent, "Captured on this call · new-body");
});
