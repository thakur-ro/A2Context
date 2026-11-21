import assert from "node:assert/strict";
import {readFile} from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";

const source = (await Promise.all(["interview-plan","sidecar-canvas"].map(name =>
  readFile(new URL(`../ui/${name}.js`,import.meta.url),"utf8")))).join("\n");
const records = [
  {facts:{id:"first", title:"Compare dispatch", kind:"Proposal trials"},dispatch:true},
  {facts:{id:"second", title:"Compare declarations", kind:"Rule conflict"}},
];

function context(reply, checkPacket = packet => {
  assert.deepEqual(packet.catalog.map(row => row.id),["R1","R2"]);
  assert.equal(packet.history[0].id, "R1");
}) {
  const env = vm.createContext({kindLabels:{conflict:"Rule conflict",duplicate:"Exact repetition"}, AbortSignal,
    fetch:async (_,request) => {
      const content = JSON.parse(request.body).messages[1].content;
      const packet = content.startsWith("{") ? JSON.parse(content) : content;
      checkPacket(packet);
      return {ok:true,json:async () => ({choices:[{message:{content:typeof reply === "string" ? reply : JSON.stringify(reply)}}]})};
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

test("the interview can choose a measured view only for a supplied measurement", async () => {
  const measured = {facts:{id:"real",title:"Policy edit",kind:"Token measurements",sources:["policy"]},tokenCard:{}};
  const packet = p => {
    assert.match(p,/T1 = "Read the proposal or result for tool verify customer"/);
    assert.match(p,/F1 = "Measure how editing policy/);
    assert.ok(!p.includes('"real"'));
  };
  const check = reply => context(reply,packet).planInterview("model","Which words moved?",[records[1]],
    {subject:measured,hasCalls:false,tools:["verify_customer"]});
  assert.equal((await check("F1")).record,measured);
  assert.equal((await check("F1")).view,"tokens");
  assert.equal((await check("F2")).view,"evidence");
  assert.equal((await check("T1")).tool,"verify_customer");
  for (const invalid of ["F99","T2","SHOWN","T1 | extra","https://unrelated.invalid"]) {
    await assert.rejects(check(invalid),/could not select evidence/);
  }
});

test("planned searches only supply keywords and preserve the host's scope, result cap and partial fallback", async () => {
  const env = context("SEARCH | verify_customer | https://unrelated.invalid",() => {});
  const plan = await env.planInterview("model","Was identity confirmed?",[],{hasCalls:false,tools:[]});
  assert.deepEqual([...plan.queries],["verify_customer"]);
  const reads = [];
  env.api = async (path,p) => {
    assert.equal(path,"activity-search");
    assert.equal(p.workflow,"mine");
    reads.push(p.q);
    if (p.q === "shopper identity") return {partial_match:true,index_ready:true,rows:[{trace:"old",thread_id:"other"}]};
    assert.equal(p.match,"all");
    const threads = p.q === "verify_customer" ? ["a","b","c","d"] : ["a","e","f","g"];
    return {rows:threads.map(thread_id => ({thread_id,trace:"sample"}))};
  };
  const result = await env.retrieveActivity({workflow:"mine"},"shopper identity",["verify_customer","identification"],new AbortController().signal);
  assert.deepEqual(reads,["shopper identity","verify_customer","identification"]);
  assert.deepEqual(Array.from(result.rows,r => r.thread_id),["a","b","e","c"]);
  assert.equal(result.partial_match,false);
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

test("conversation interpretation can cite only supplied calls and cannot change their evidence links", async () => {
  const sections = [{title:"Call 1",lines:["Tool result · read_order · delivered"],href:"?view=traces&call=first"},
    {title:"Call 2",lines:["Assistant · Order delivered."],href:"?view=traces&call=second"}];
  const check = packet => {
    assert.deepEqual(packet.calls.flatMap(call => call.lines.map(line => line.id)),["C1.1","C2.1"]);
    assert.equal(packet.calls.some(call => "href" in call),false);
  };
  const env = context({answer:"An unsupported claim",sources:["C1.1"],href:"https://unrelated.invalid"},check);
  const result = await env.interpretActivity("model","What did it return?",sections);
  assert.equal(result.evidence[0].href,sections[0].href);
  assert.equal(result.evidence[0].lines[0],sections[0].lines[0]);
  assert.ok(!result.answer.includes("unsupported claim"));
  const indirect = await context({sources:["C2.1"]},check).interpretActivity("model","Show requests to change an order.",sections);
  assert.match(indirect.answer,/no direct user message/);
  const invalid = context({sources:["C1.2"]},check);
  await assert.rejects(invalid.interpretActivity("model","What did it return?",sections),/Invalid recorded line reference/);
  const unknown = context({answer:"Something happened",sources:[]},check);
  assert.equal(await unknown.interpretActivity("model","Why did it do that?",sections),null);
});
