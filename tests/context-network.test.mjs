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

test("every loaded finding has one path through its workflow and type at scale", () => {
  const rows = Array.from({length:2400},(_,i) => {
    const row = card(String(i),"trace-"+i,"returns","policy","v1");
    row.evidence.organization = i % 4 ? "retail" : "wholesale";
    row.kind = i % 2 ? "conflict" : "duplicate";
    return row;
  });
  const hierarchy = vm.runInNewContext(source+"\nworkflowHierarchy",{plural:(n,word)=>n+" "+word});
  const {nodes,parents} = hierarchy([...rows,rows[0]],build(rows));
  const root = nodes.get("workflows");
  assert.equal(root.count,2400);
  assert.deepEqual(Array.from(root.children,id=>nodes.get(id).count),[1800,600]);
  const leaves = [];
  for (const row of rows) {
    const finding = "finding:"+row.id;
    const kind = nodes.get(parents.get(finding));
    const workflow = nodes.get(parents.get(kind.id));
    assert.equal(kind.name,row.kind);
    assert.equal(workflow.description,row.evidence.organization+" · otlp");
  }
  for (const workflow of root.children) {
    for (const kind of nodes.get(workflow).children) leaves.push(...nodes.get(kind).children);
  }
  assert.equal(new Set(leaves).size,2400);
  assert.equal(leaves.length,2400);
});
