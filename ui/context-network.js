// Only relationships present in the evidence become edges. A source label can recur across findings.
function makeContextNetwork(rows) {
  const nodes = new Map();
  const edges = [];
  const seen = new Set();
  function add(id, data) {
    if (!nodes.has(id)) nodes.set(id, {id, ...data});
    return id;
  }
  function connect(from, to, relation) {
    const key = JSON.stringify([from,to,relation]);
    if (!seen.has(key)) edges.push({from,to,relation});
    seen.add(key);
  }
  for (const card of rows) {
    const {facts} = canvasRecord(card);
    const detail = card.evidence.detail;
    const probe = ["prefill","candidate","token"].includes(card.kind) ? card : null;
    const coverage = card.kind === "source_gap" ? detail.coverage : null;
    const finding = add("finding:"+card.id,{type:"finding",name:facts.title,description:facts.observed,
      kind:facts.kind,probe,coverage});
    if (card.kind === "source_gap") {
      for (const name of facts.unconfirmed_sources) {
        if (coverage.text_mismatches?.includes(name)) continue;
        const id = add("unconfirmed:"+JSON.stringify([card.id,name]),{type:"unconfirmed",name,
          description:"No matching source anchor in this request"});
        connect(id,finding,"required anchor unconfirmed");
      }
    }
    for (const source of detail.sources) {
      const name = sourceName(source);
      const {producer, organization, workflow, trace_id, call_id} = card.evidence;
      const identity = [producer, organization, workflow, source.source ?? [trace_id,call_id,source.id],
        source.version ?? null,source.role ?? null];
      const id = add("source:"+JSON.stringify(identity),{type:"source",name,caption:[source.role,source.version].filter(Boolean).join(" · "),
        description:[workflow,source.role,source.version && "Version "+source.version].filter(Boolean).join(" · ") || "Version and role unavailable"});
      connect(id,finding,coverage?.text_mismatches?.includes(source.source) ? "text differs from expected digest" : "named in evidence");
    }
    for (const [i,arm] of (facts.map?.arms || []).entries()) {
      const id = add("arm:"+card.id+":"+i,{type:"outcome",name:arm.value,kind:arm.label,
        description:facts.map.root});
      const relation = {conflict:"reports a declared value",candidate:"compares candidate log odds",token:"compares token probabilities"};
      connect(finding,id,relation[card.kind] || "compares outcomes");
    }
  }
  return {nodes,edges};
}

function networkSvg(tag, attributes, text) {
  const node = document.createElementNS("http://www.w3.org/2000/svg",tag);
  for (const [key,value] of Object.entries(attributes)) node.setAttribute(key,value);
  if (text != null) node.textContent = text;
  return node;
}

// Each card contributes once, under the workflow of its representative evidence.
function workflowHierarchy(rows, evidence) {
  const root = {id:"workflows",type:"scope",name:"Loaded findings",count:0,children:[]};
  const nodes = new Map([[root.id,root]]);
  const parents = new Map();
  function connect(parent, node) {
    if (!nodes.has(node.id)) {
      nodes.set(node.id,node);
      parents.set(node.id,parent.id);
      parent.children.push(node.id);
    }
    return nodes.get(node.id);
  }
  for (const card of rows) {
    const finding = evidence.nodes.get("finding:"+card.id);
    if (parents.has(finding.id)) continue;
    const {producer,organization,workflow} = card.evidence;
    const scope = [producer,organization,workflow];
    const group = connect(root,{id:"workflow:"+JSON.stringify(scope),type:"workflow",
      name:workflow || "Workflow unavailable",description:[organization,producer].filter(Boolean).join(" · "),count:0,children:[]});
    const kind = connect(group,{id:"kind:"+JSON.stringify([...scope,card.kind]),type:"category",
      name:finding.kind || card.kind || "Unknown finding type",count:0,children:[]});
    connect(kind,finding);
    root.count++;
    group.count++;
    kind.count++;
  }
  for (const node of nodes.values()) {
    if (!node.children) continue;
    node.caption = plural(node.count,"finding");
    // Preserve the feed's ranking among the individual findings.
    if (node.type !== "category") node.children.sort((a,b) => nodes.get(b).count - nodes.get(a).count);
  }
  return {nodes,parents};
}

function renderWorkflowBranches(hierarchy,node,page,choose,flip) {
  const view = element("section",{className:"workflow-tree","aria-label":"Findings by workflow"});
  const trail = element("nav",{"aria-label":"Workflow path"});
  for (let id = hierarchy.parents.get(node.id); id; id = hierarchy.parents.get(id)) {
    const ancestor = hierarchy.nodes.get(id);
    const back = element("button",{type:"button",text:ancestor.name});
    back.onclick = () => choose(ancestor.id);
    trail.prepend(back);
  }
  const heading = element("h3",{tabindex:"-1",text:node.name});
  const branch = element("div",{className:"workflow-branch"},
    element("header",{},heading,element("small",{text:[node.description,node.caption].filter(Boolean).join(" · ")})));
  const items = element("ul",{});
  const size = 8;
  for (const id of node.children.slice(page*size,(page+1)*size)) {
    const child = hierarchy.nodes.get(id);
    const button = element("button",{type:"button"},element("span",{text:child.name}));
    if (child.children) button.append(element("small",{text:[child.description,child.caption].filter(Boolean).join(" · ")}));
    button.onclick = () => choose(id);
    items.append(element("li",{},button));
  }
  branch.append(items);
  view.append(trail,branch);
  if (node.children.length > size) {
    const pages = Math.ceil(node.children.length/size);
    const prev = element("button",{type:"button",text:"Previous"});
    const next = element("button",{type:"button",text:"Next"});
    prev.disabled = page === 0;
    next.disabled = page+1 === pages;
    prev.onclick = () => flip(page-1);
    next.onclick = () => flip(page+1);
    view.append(element("div",{className:"network-pager"},prev,element("span",{text:(page+1)+" / "+pages}),next));
  }
  return view;
}

function renderNeighborhood(graph, focus, page, choose, flip) {
  const adjacent = graph.edges.filter(e => e.from === focus.id || e.to === focus.id);
  // Each side is paged independently so a high-degree source never hides both measured arms.
  const incoming = adjacent.filter(e => e.to === focus.id);
  const outgoing = adjacent.filter(e => e.from === focus.id);
  const size = 5;
  const pages = Math.max(1,Math.ceil(Math.max(incoming.length,outgoing.length)/size));
  const windowOf = edges => edges.slice(page*size,(page+1)*size);
  const left = windowOf(incoming), right = windowOf(outgoing);
  const height = Math.max(225, 73*Math.max(left.length,right.length)+36);
  const chart = networkSvg("svg",{viewBox:"0 0 780 "+height,role:"group","aria-label":"Connected evidence"});
  const paths = networkSvg("g",{"aria-hidden":"true"});
  chart.append(paths);
  function drawNode(node,x,y,center=false) {
    const g = networkSvg("g",{class:"network-node "+node.type+(center?" selected":""),role:"button",
      tabindex:"0","aria-label":node.type+" · "+node.name});
    g.append(networkSvg("title",{},[node.kind,node.name,node.description].filter(Boolean).join(" · ")),
      networkSvg("circle",{cx:x,cy:y,r:center?22:14,class:"node-aura"}),
      networkSvg("circle",{cx:x,cy:y,r:center?10:6,class:"node-core"}));
    const displayName = node.coverage?.name || node.name;
    const label = displayName.length > 32 ? displayName.slice(0,31)+"…" : displayName;
    g.append(networkSvg("text",{x,y:y+31,"text-anchor":"middle",class:"network-label"},label));
    const detail = node.type === "unconfirmed" ? "Anchor unconfirmed" : node.caption || node.kind || node.description;
    if (detail) g.append(networkSvg("text",{x,y:y+45,"text-anchor":"middle",class:"network-caption"},detail.slice(0,32)));
    const activate = () => choose(node.id);
    g.addEventListener("click",activate);
    g.addEventListener("keydown",e => {if (e.key === "Enter" || e.key === " ") {e.preventDefault();activate();}});
    g.style.pointerEvents = "bounding-box";
    chart.append(g);
  }
  const middle = height/2-17;
  function branch(edges, leftSide) {
    const x = leftSide ? 125 : 655;
    const direction = leftSide ? -1 : 1;
    edges.forEach((edge,i) => {
      const y = middle + (i-(edges.length-1)/2)*73;
      const node = graph.nodes.get(leftSide ? edge.from : edge.to);
      const path = networkSvg("path",{d:`M390,${middle} C${390+130*direction},${middle} ${x-115*direction},${y} ${x},${y}`,
        class:"network-edge "+node.type});
      paths.append(path);
      drawNode(node,x,y);
    });
  }
  branch(left,true);
  branch(right,false);
  drawNode(focus,390,middle,true);
  const navigation = element("div",{className:"network-pager"});
  const button = (text,step) => {
    const b = element("button",{type:"button",text});
    b.disabled = page+step<0 || page+step>=pages;
    b.onclick = () => flip(page+step);
    return b;
  };
  if (pages>1) navigation.append(button("Previous connections",-1),element("span",{text:(page+1)+" / "+pages}),button("Next connections",1));
  return element("div",{},element("div",{className:"network-viewport",tabindex:"0","aria-label":"Context map, scroll horizontally on small screens"},chart),navigation);
}

// A bounded Fruchterman–Reingold layout for the overview. Rescale after cooling
// so that disconnected components can settle without piling up on a boundary.
function layoutNetwork(points, links) {
  const spacing = Math.sqrt(700 * 320 / points.length);
  for (let step = 0; step < 75; step++) {
    for (const p of points) { p.dx = 0; p.dy = 0; }
    function displace(a, b, scale) {
      const dx = (a.x-b.x)*scale, dy = (a.y-b.y)*scale;
      a.dx += dx; a.dy += dy;
      b.dx -= dx; b.dy -= dy;
    }
    for (const [i,a] of points.entries()) {
      for (let j = i+1; j < points.length; j++) {
        const b = points[j];
        const squared = (a.x-b.x)**2 + (a.y-b.y)**2;
        displace(a, b, spacing**2 / Math.max(1,squared));
      }
    }
    for (const [a,b] of links) {
      displace(a, b, -Math.hypot(a.x-b.x,a.y-b.y)/spacing);
    }
    const temperature = 12*(1-step/75);
    for (const p of points) {
      p.dx += (390-p.x)*0.35;
      p.dy += (195-p.y)*0.6;
      const scale = Math.min(1,temperature/Math.max(1,Math.hypot(p.dx,p.dy)));
      p.x += p.dx*scale;
      p.y += p.dy*scale;
    }
  }
  for (const [axis,lo,hi] of [["x",52,728],["y",25,350]]) {
    const values = points.map(p=>p[axis]);
    const min = Math.min(...values), span = Math.max(...values)-min;
    for (const p of points) p[axis] = lo + (hi-lo)*(p[axis]-min)/Math.max(1,span);
  }
}

const networkLayoutLimit = 120;

function networkOverview(graph, choose) {
  const nodes = [...graph.nodes.values()];
  const count = nodes.length;
  const points = nodes.map((node,i) => {
    const radius = Math.sqrt((i+.5)/count);
    return {node,x:390+Math.cos(i*2.39996)*300*radius,y:195+Math.sin(i*2.39996)*145*radius};
  });
  const lookup = new Map(points.map(p=>[p.node.id,p]));
  const links = graph.edges.map(e=>[lookup.get(e.from),lookup.get(e.to)]).filter(([a,b])=>a && b);
  if (count <= 12) {
    // Small cases read left to right, with enough space between full labels.
    for (const [column,type] of ["source","finding","outcome"].entries()) {
      const lane = points.filter(p => p.node.type === type || (type === "source" && p.node.type === "unconfirmed"));
      lane.forEach((p,i) => {
        p.x = 128 + column*260;
        p.y = (i+1)*365/(lane.length+1);
      });
    }
  }
  if (count > 12) layoutNetwork(points, links);
  const chart=networkSvg("svg",{viewBox:"0 0 780 390",class:count<=12?"small-network":"large-network",role:"group","aria-label":"Network overview"});
  for (const [a,b] of links) chart.append(networkSvg("line",{class:"network-edge "+b.node.type,x1:a.x,y1:a.y,x2:b.x,y2:b.y}));
  const degree = new Map();
  for (const [a,b] of links) {
    for (const p of [a,b]) degree.set(p,(degree.get(p) || 0)+1);
  }
  const hubs = [];
  for (const p of points.filter(p => p.node.type === "source").sort((a,b) => degree.get(b)-degree.get(a))) {
    if (hubs.length === 4) break;
    if (hubs.every(q => Math.abs(p.x-q.x)>140 || Math.abs(p.y-q.y)>38)) hubs.push(p);
  }
  // Paint named hubs last so surrounding nodes cannot obscure their labels.
  for (const point of [...points.filter(p=>!hubs.includes(p)), ...hubs]) {
    const {node,x,y} = point;
    const g=networkSvg("g",{class:"network-node "+node.type,role:"button",tabindex:"0","aria-label":node.type+" · "+node.name});
    const radius=node.type==="finding"?8:5;
    g.append(networkSvg("title",{},node.name+" · "+node.description),networkSvg("circle",{cx:x,cy:y,r:radius+5,class:"node-aura"}),
      networkSvg("circle",{cx:x,cy:y,r:radius,class:"node-core"}));
    const label=node.name.length>24 ? node.name.slice(0,23)+"…" : node.name;
    g.append(networkSvg("text",{x,y:y+radius+14,"text-anchor":"middle",class:count>12 && !hubs.includes(point)?"network-label hover-label":"network-label"},label));
    g.onclick=()=>choose(node.id);
    g.onkeydown=e=>{if(e.key==="Enter" || e.key===" ") {e.preventDefault();choose(node.id);}};
    if (count <= 12) g.style.pointerEvents = "bounding-box";
    chart.append(g);
  }
  return element("div",{},element("div",{className:"network-viewport overview-plot",tabindex:"0","aria-label":"Context map overview, scroll horizontally on small screens"},chart),
    element("small",{className:"network-key",text:count+" of "+plural(graph.nodes.size,"loaded node")+" · "+plural(links.length,"visible connection")+
      ". Select a node to inspect its neighbors. Distance is for layout only."}));
}

function contextNetwork(rows, {focusId, onNext, total, filters} = {}) {
  const root = element("section",{className:"context-network","aria-label":"Context map"});
  const graph = makeContextNetwork(rows);
  if (!graph.nodes.size) return element("p",{className:"empty",text:"No findings match this scope."});
  const hierarchy = workflowHierarchy(rows,graph);
  const large = graph.nodes.size > networkLayoutLimit;
  const search = element("select",{"aria-label":"Focus a node"});
  search.append(element("option",{value:"workflows",text:"Browse by workflow"}));
  const option = node => element("option",{value:node.id,text:node.type+" · "+node.name});
  if (!large) {
    search.prepend(element("option",{value:"",text:"Whole network"}));
    for (const node of graph.nodes.values()) search.append(option(node));
  }
  const body = element("div",{});
  const summary = element("span",{text:plural(rows.length,"finding")+(total != null ? " of "+total : "")+" loaded"});
  const labels = {source:"Source",unconfirmed:"Unconfirmed source",finding:"Finding",outcome:"Declared or measured value"};
  const types = new Set([...graph.nodes.values()].map(node => node.type));
  const legend = element("div",{className:"network-legend"},...Object.keys(labels).filter(type => types.has(type))
    .map(type => element("span",{className:type,text:labels[type]})));
  const controls = element("div",{className:"network-controls"},element("label",{}, "Focus", search),summary);
  let currentId;
  let currentOption;
  function selectFocus(value) {
    currentOption?.remove();
    currentOption = undefined;
    search.value = value;
    if (large && search.selectedIndex < 0 && graph.nodes.has(value)) {
      currentOption = option(graph.nodes.get(value));
      search.append(currentOption);
      search.value = value;
    }
  }
  if (large) {
    const query = element("input",{type:"search","aria-label":"Find a source or finding",placeholder:"Find a source or finding"});
    const matches = element("small",{"aria-live":"polite"});
    controls.prepend(element("div",{className:"network-search"},query,matches));
    query.oninput = () => {
      const term = query.value.trim().toLocaleLowerCase();
      search.replaceChildren(element("option",{value:"workflows",text:"Browse by workflow"}));
      let count = 0;
      for (const node of graph.nodes.values()) {
        if (!term || !node.name.toLocaleLowerCase().includes(term)) continue;
        if (++count <= 30) search.append(option(node));
      }
      selectFocus(graph.nodes.has(currentId) ? currentId : "workflows");
      matches.textContent = term ? count+(count === 1 ? " match" : " matches")+(count>30 ? " · first 30 listed" : "") : "";
    };
  }
  if (onNext) {
    const more = element("button",{type:"button",text:"Load more findings"});
    more.onclick = async () => {
      more.disabled = true;
      try { await onNext(currentId); }
      catch (err) { summary.textContent=err.message;more.disabled=false; }
    };
    controls.append(more);
  }
  function show(id,page=0,keyboard=false) {
    const grouped = hierarchy.nodes.get(id)?.children;
    selectFocus(grouped ? "workflows" : id);
    legend.hidden = Boolean(grouped);
    currentId = id;
    if (!id) {
      body.replaceChildren(networkOverview(graph,next=>show(next,0,true)));
      return;
    }
    if (grouped) {
      const node = hierarchy.nodes.get(id);
      body.replaceChildren(renderWorkflowBranches(hierarchy,node,page,next=>show(next,0,true),next=>show(id,next,true)),
        element("small",{className:"network-key",text:"Each card is counted once, under the workflow of its displayed evidence."}));
      if (keyboard) body.querySelector("h3").focus({preventScroll:true});
      return;
    }
    const node = graph.nodes.get(id);
    const neighbors = graph.edges.filter(e => e.from === id || e.to === id);
    const relations = [...new Set(neighbors.map(e=>e.relation))].join(" · ");
    const drawing = renderNeighborhood(graph,node,page, next => show(next,0,true), next => show(id,next,true));
    body.replaceChildren(drawing,element("div",{className:"network-readout"},
      element("strong",{text:node.name}),element("span",{text:node.description}),
      element("small",{text:plural(neighbors.length,"connection")+" · "+relations})));
    const parent = hierarchy.parents.get(id);
    if (parent) {
      const group = hierarchy.nodes.get(parent);
      const back = element("button",{type:"button",text:"See "+group.caption+" in "+group.name});
      back.onclick = () => show(parent,0,true);
      body.querySelector(".network-readout").append(back);
    }
    if (node.coverage) body.append(element("section",{className:"panel","aria-label":"Configured alternatives"},
      element("h3",{text:"One complete alternative is required"}),requiredSourceList(node.coverage)));
    if (node.probe?.kind === "prefill") {
      body.append(visual("prefill", node.probe.evidence.detail));
    } else if (node.probe) {
      const {kind, evidence:{detail}} = node.probe;
      const title = kind === "candidate" ? "Choice scores at this prefix" : "Measured output token " + (detail.landmark.token + 1);
      body.append(element("section", {className:"panel", "aria-label":title},
        element("h3", {text:title}), visual(kind, detail), recommendationContext(kind, detail)));
      if (kind === "token") {
        const path = element("details",{className:"network-answer-path"},
          element("summary",{text:"Follow source edits across this answer"}));
        path.addEventListener("toggle",() => {
          if (path.open && !path.querySelector(".answer-path")) path.append(tokenPath(node.probe,{sharePosition:false,inline:true,filters}));
        });
        body.append(path);
      }
    }
    if (keyboard) body.querySelector(".network-node.selected").focus({preventScroll:true});
  }
  search.onchange=()=>show(search.value);
  root.append(element("div",{className:"network-heading"},element("h2",{text:"Context map"}),legend),controls,body);
  show(graph.nodes.has(focusId) || hierarchy.nodes.has(focusId) ? focusId : large ? "workflows" : "");
  return root;
}

// Pages come from retained observations, including calls other than the example on a card.
async function sourceHistory() {
  const selection = {source_key:sourceHistoryKey};
  if (threadId) selection.thread = threadId;
  if (query.get("unthreaded_trace")) selection.unthreaded_trace = query.get("unthreaded_trace");
  const calls = Boolean(selection.thread || selection.unthreaded_trace);
  const {rows,next_offset} = await api("source-history",{...scope,...selection,offset:pageOffset});
  const [,org,workflow,name,revision,role] = JSON.parse(sourceHistoryKey);
  const sample = rows[0];
  byId("title").textContent = name;
  byId("summary").textContent = [workflow,org,revision && "Version "+revision,role].filter(Boolean).join(" · ");
  pagination(next_offset,selection);
  const back = calls ? link("Back to source conversations",url({source_key:sourceHistoryKey})) : link("Recommendations",url({}));
  if (!sample) return element("div",{className:"feed"},back,element("p",{className:"empty",text:"No matching source evidence on this page."}));
  const caption = calls ? "Ordered by recorded start time" : plural(sample.total_calls,"call")+" · "+plural(sample.total_threads,"identified conversation");
  const origin = calls ? sample.thread || "Trace without a conversation ID" : name;
  const branches = element("ul",{className:"source-destinations"});
  const paths = networkSvg("svg",{viewBox:"0 0 100 100",preserveAspectRatio:"none","aria-hidden":"true"});
  for (const [index,row] of rows.entries()) {
    const y = (index+.5)*100/rows.length;
    paths.append(networkSvg("path",{d:`M0 50 C45 50 55 ${y} 100 ${y}`,"vector-effect":"non-scaling-stroke"}));
    const label = calls ? row.turn || row.call : row.thread || "Trace without a conversation ID";
    const detail = calls ? [fullDate(row.started),row.kinds.map(kind => kindLabels[kind] || kind).join(" · ")] :
      [row.user,row.unthreaded_trace,plural(row.calls,"call")];
    const dest = calls ? (threadId ? {view:"threads",thread:threadId,focus_trace:row.trace,focus_call:row.call} :
      {view:"traces",trace:row.trace,call:row.call}) : {...selection,thread:row.thread_key,unthreaded_trace:row.unthreaded_trace};
    branches.append(element("li",{},rowLink(label,detail.filter(Boolean).join(" · "),url(dest))));
  }
  return element("section",{className:"source-history","aria-label":"Source evidence map"},back,
    element("h2",{text:calls ? "Calls citing this source" : "Conversations citing this source"}),
    element("p",{text:caption+(sample.unthreaded_calls ? " · "+plural(sample.unthreaded_calls,"call")+" without a conversation ID" : "")}),
    element("div",{className:"source-branches"},element("strong",{text:origin}),element("div",{className:"source-wires"},paths),branches),
    element("small",{className:"network-key",text:"Counts include calls with a retained finding that cites this source."}));
}
