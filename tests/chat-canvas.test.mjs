import assert from "node:assert/strict";
import {readFile} from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";

const source = (await Promise.all(["interview-conversations","interview-plan","sidecar-canvas"].map(name =>
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

test("user scope preserves exact identifiers, the remaining question and quoted search text", () => {
  const {interviewUserRequest:read,replyUserId} = context();
  const check = (text,value) => assert.deepEqual(JSON.parse(JSON.stringify(read(text))),value);
  check('Which issues keep coming up for user "Customer 042"?',{user:"Customer 042",question:"Which issues keep coming up"});
  check("Show recent conversations for user Customer 042 in Returns demo",{user:"Customer 042",question:"Show recent conversations in Returns demo"});
  check("Which patterns repeat across support-17’s conversations?",{user:"support-17",question:"Which patterns repeat across conversations"});
  check("Focus on user ID Case.Owner+2@example.org",{user:"Case.Owner+2@example.org",question:""});
  check("Which issues keep coming up for this user?",{current:true,question:"Which issues keep coming up"});
  check("Which issues recur for all users?",{user:null,question:"Which issues recur"});
  check("Show recorded activity for user",{needsId:true,question:"Show recorded activity"});
  check('Search for "timeout for user name"',null);
  check('Search for "problems for this user"',null);
  check("Find conversation “An issue in support-17’s conversations”",null);
  check('Search for "for user A" for user B',{user:"B",question:'Search for "for user A"'});
  check('Focus on user "Ops, A/B"',{user:"Ops, A/B",question:""});
  check('Show conversations for user A and user B',{needsId:true,question:"Show conversations"});
  assert.equal(replyUserId("Customer 042"),"Customer 042");
  assert.equal(replyUserId("Show recorded activity"),null);
});

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

test("the partner can select only a conversation offered in the answer",async () => {
  const ref = {thread:{id:"scoped-key",thread:"Hold the puzzle"},filters:{user:"support-17"},focus:{trace:"trace",call:"call"}};
  const options = {tools:[],conversations:[{title:"Hold the puzzle",conversation:ref}]};
  const choose = reply => context(reply,packet => {
    assert.match(packet,/C1 = "Open displayed conversation 1 · Hold the puzzle"/);
    assert.ok(!packet.includes("scoped-key"));
  }).planInterview("local","Take me back to the jigsaw order",[],options);
  assert.equal((await choose("C1")).reference,ref);
  await assert.rejects(choose("C2"),/could not select evidence/);
  const {listedConversationName:name} = context();
  assert.equal(name('Read “Hold the puzzle”',options.conversations),0);
  assert.equal(name('Read “Hold the puzzle”',[...options.conversations,...options.conversations]),-1);
  assert.equal(name('What did the customer say?',options.conversations),null);
});

test("a chart read carries the filters that produced it and leaves a tied count unresolved",async () => {
  const chart = {filters:{workflow:"orders",user:"a"},themes:[{kind:"conflict",conversations:2},{kind:"duplicate",conversations:2}]};
  const env = context("K2",packet => assert.match(packet,/K2 = "List the recorded conversations counted under Exact repetition/));
  const read = await env.planInterview("local","Find requests that got copied",[],{tools:[],chart});
  assert.equal(read.kind,"duplicate");
  assert.equal(read.filters,chart.filters);
  assert.equal(env.chartConversationRequest("Which two conversations?",chart).themes.length,2);
  assert.equal(env.chartConversationRequest("Which 3 conversations?",chart).themes.length,0);
  assert.equal(env.chartConversationRequest("Can you show the 2nd one?",{...chart,choosing:true}).themes[0].kind,"duplicate");
});

test("next-test requests distinguish future checks from recorded experiments",() => {
  const {asksForNextTest:asked,recommendationTopics:topics,rankNextTests:rank} = context();
  assert.ok(asked("Which experiment would you run before deployment?"));
  assert.ok(asked("What should we test before another order gets updated?"));
  assert.equal(asked("Did the experiment change the order?"),false);
  assert.equal(asked("What did the test before deployment show?"),false);
  assert.equal(topics("What should I test next?").length,0);
  const a = {facts:{title:"Inspect the search catalog",next:"Compare available tools."}};
  const b = {facts:{title:"Check purchase approval",next:"Replay an approved purchase."},receipt:{tool:"write_order"}};
  assert.equal(rank([a,b],topics("What should I check before a write_order call?"))[0].record,b);
  assert.ok(rank([a,b],topics("What should we test on Mars?")).every(r => r.score === 0));
});

test("a partner selects among topical next tests and receives no request when none match",async () => {
  const first = {facts:{title:"Order dispatch",next:"Replay the approved order."}};
  const second = {facts:{title:"Order conflict",next:"Isolate the disputed instruction."}};
  const candidates = [first,second];
  const env = context("A2",packet => assert.equal(packet.options.length,2));
  assert.equal(await env.selectNextTest("Which order test should we run?",candidates,null,[],"local"),second);
  const offline = context("A1",() => assert.fail("Unrelated findings reached the model"));
  assert.equal(await offline.selectNextTest("What should we test before sending the new satellite to Mars?",candidates,null,[],"local"),undefined);
  assert.equal(await offline.selectNextTest("What should we test next?",candidates,second,[],"local"),second);
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

test("a source edit and a state replay on the same record have independent evidence choices", async () => {
  const record = {facts:{id:"x",sources:["confirmation-cache"]},
    receipt:{guard:"check-current-approval",runner:"exchange-sqlite",target:"order-4",tool:"exchange_order"},
    proposal:{original:{kind:"tool_call",label:"exchange_order"},edited:{kind:"tool_call",label:"exchange_order"}}};
  const other = {facts:{id:"y",sources:["data-allowance"]},
    receipt:{guard:"check-refuel-at-dispatch",runner:"telecom",target:"data_refueling_gb",tool:"refuel_data"},
    proposal:{original:{kind:"tool_call",label:"refuel_data"},edited:{kind:"reply",label:"done"}}};
  const candidates = [other,record];
  const packet = p => {
    assert.match(p,/Compare both outcomes.*check-current-approval/);
    assert.match(p,/Compare the model's output when confirmation-cache was edited/);
  };
  const env = context("F4",packet);
  const read = q => env.recordedReplay(q,candidates);
  assert.equal(read("Compare the order database with and without the approval check.").record,record);
  assert.equal(read("Compare the refuel balance with and without the check.").record,other);
  assert.equal(read("How did editing data-allowance change the model output?").view,"proposal");
  assert.equal(read("Did the cache change the model’s answer?").record,record);
  assert.equal(read("Compare the invoice balance before and after."),null);
  assert.equal(env.recordedReplay("Compare the recorded proposals",candidates,other).record,other);
  const sameLabel = {...record,facts:{...record.facts,id:"another"}};
  assert.equal(env.recordedReplay("Compare the order database with and without approval.",[record,sameLabel]),null);
  const plan = await env.planInterview("model","What did the model do under the edit?",candidates,{tools:[],hasCalls:false});
  assert.equal(plan.view,"proposal");
  assert.equal(plan.record,record);
  const combined = read("What did the customer request and how did the approval check change the outcome?");
  assert.equal(combined.record,record);
  assert.deepEqual(Array.from(combined.parts),["message","receipt"]);
  const two = read("Compare the model proposal and the refuel balance.");
  assert.equal(two.record,other);
  assert.deepEqual(Array.from(two.parts),["proposal","receipt"]);
  const tied = env.recordedReplay("What did the user ask, and did the approval check change the outcome?",[record,sameLabel]);
  assert.equal(tied.view,"chooseReplay");
  assert.deepEqual(Array.from(tied.records),[record,sameLabel]);
  const select = reply => context(reply,packet).planInterview("model","Show the request, the attempt, and the consequence together.",candidates,{tools:[],hasCalls:false});
  const composed = await select("B8");
  assert.equal(composed.record,record);
  assert.deepEqual(Array.from(composed.parts),["message","proposal","receipt"]);
  for (const invalid of ["F1 + F4","B9","B4 + B8","MESSAGE + F99"]) {
    await assert.rejects(select(invalid),/could not select evidence/);
  }
  const partial = context("B1",p => {
    assert.match(p,/^B1 = .*own words and what took effect/m);
    assert.doesNotMatch(p,/^B\d+ = .*what the assistant proposed/m);
  });
  const twoParts = await partial.planInterview("model","The shopper’s own words next to both end states?",[record],{tools:[],hasCalls:false});
  assert.deepEqual(Array.from(twoParts.parts),["message","receipt"]);
});

test("a broad model rewrite preserves the original two-term witness and the partial-match label", async () => {
  const env = context();
  const pair = {trace:"pair",thread_id:"first"}, broad = {trace:"broad",thread_id:"second"};
  env.api = async (_,params) => params.q === "failing test suite" ?
    {rows:[pair],partial_match:true,minimum_matching_terms:2} : {rows:[broad],minimum_matching_terms:1};
  const result = await env.retrieveActivity({},"failing test suite",["developer"],new AbortController().signal);
  assert.deepEqual(Array.from(result.rows,r => r.trace),["pair","broad"]);
  assert.equal(result.partial_match,true);
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
