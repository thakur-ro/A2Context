import assert from "node:assert/strict";
import {readFile} from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";

const source = await readFile(new URL("../ui/sidecar-canvas.js", import.meta.url), "utf8");
const records = [
  {facts:{id:"first", title:"Compare dispatch", kind:"Proposal trials"},dispatch:true},
  {facts:{id:"second", title:"Compare declarations", kind:"Rule conflict"}},
];

function context(reply) {
  const env = vm.createContext({kindLabels:{conflict:"Rule conflict",duplicate:"Exact repetition"}, AbortSignal,
    fetch:async (_,request) => {
      const packet = JSON.parse(JSON.parse(request.body).messages[1].content);
      assert.deepEqual(packet.catalog.map(row => row.id),["R1","R2"]);
      assert.equal(packet.history[0].id, "R1");
      return {ok:true,json:async () => ({choices:[{message:{content:JSON.stringify(reply)}}]})};
    }});
  vm.runInContext(source, env);
  return env;
}

test("a model can name a finding in its own language and the canvas still shows the measured evidence", async () => {
  const env = context({id:"R2",view:"conflict",answer:"Check the approval values."});
  const result = await env.partnerAnswer("my-model","Which approval changed?",records,[{id:"first",question:"First?"}]);
  assert.equal(result.record,records[1]);
  assert.equal(result.view,"evidence");
});

test("a partner cannot cite a finding outside the selected scope", async () => {
  const env = context({id:"R3",view:"action",answer:"Inspect this result."});
  await assert.rejects(env.partnerAnswer("my-model","Another?",records,[{id:"first"}]),/invalid answer or evidence reference/);
});

test("no matching evidence, a follow-up about recurrence, and a new topic select different results", () => {
  const env = context();
  assert.equal(env.canvasAnswer("winter weather forecast",records).record,null);
  assert.equal(env.canvasAnswer("In which other sessions did that happen?",records,records[1]).record,records[1]);
  const next = env.canvasAnswer("What about the sandbox?",records,records[1]);
  assert.equal(next.record,records[0]);
  assert.equal(next.view,"evidence");
});

test("a measured dispatch answer uses the paired target states without a model paraphrase", async () => {
  const env = context();
  const receipt = {dispatch:true,facts:{id:"sample",observed:"Order A stayed delivered with the check."}};
  const answer = await env.partnerAnswer("my-model","What changed at dispatch?",[receipt],[]);
  assert.equal(answer.text,receipt.facts.observed);
  assert.equal(answer.author,"Sandbox replay");
});
