import assert from "node:assert/strict";
import {readFile} from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";

const source = await readFile(new URL("../ui/context-network.js",import.meta.url),"utf8");
const build = vm.runInNewContext(source + "\nmakeContextNetwork", {
  canvasRecord: card => ({facts:{title:card.id},href:"?trace="+card.evidence.trace_id}),
  sourceName: item => item.source || item.id,
});

function card(id, trace, workflow, source, version) {
  return {id, evidence:{trace_id:trace,call_id:"call",producer:"otlp",organization:"retail",workflow,
    detail:{sources:[{id:"slot-1",source,version,role:"system"}]}}};
}

test("shares a declared source only within its namespace and revision", () => {
  const graph = build([
    card("a","trace-1","returns","approval","v1"), card("b","trace-2","returns","approval","v1"),
    card("c","trace-3","support","approval","v1"), card("d","trace-4","returns","approval","v2"),
    card("e","trace-5","returns",null,null), card("f","trace-6","returns",null,null),
  ]);
  const sources = [...graph.nodes.values()].filter(n=>n.type === "source");
  assert.equal(sources.length,5);
  assert.deepEqual(sources.map(n=>graph.edges.filter(e=>e.from===n.id).length).sort(),[1,1,1,1,2]);
});
