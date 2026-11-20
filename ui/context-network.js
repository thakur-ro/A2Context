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
    const record = canvasRecord(card);
    const {facts} = record;
    const detail = card.evidence.detail;
    const finding = add("finding:"+card.id,{type:"finding",name:facts.title,description:facts.observed,href:record.href,kind:facts.kind});
    for (const source of detail.sources) {
      const name = sourceName(source);
      const {producer, organization, workflow, trace_id, call_id} = card.evidence;
      const identity = [producer, organization, workflow, source.source ?? [trace_id,call_id,source.id],
        source.version ?? null,source.role ?? null];
      const id = add("source:"+JSON.stringify(identity),{type:"source",name,
        description:[workflow,source.role,source.version && "Version "+source.version].filter(Boolean).join(" · ") || "Version and role unavailable"});
      connect(id,finding,"named in evidence");
    }
    for (const [i,arm] of (facts.map?.arms || []).entries()) {
      const id = add("arm:"+card.id+":"+i,{type:"outcome",name:arm.value,kind:arm.label,
        description:facts.map.root,href:record.href});
      connect(finding,id,card.kind === "conflict" ? "reports a declared value" : "compares outcomes");
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
    const label = node.name.length > 32 ? node.name.slice(0,31)+"…" : node.name;
    g.append(networkSvg("text",{x,y:y+31,"text-anchor":"middle",class:"network-label"},label));
    const detail = node.type === "outcome" ? node.kind : node.type === "finding" ? node.kind : node.description;
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

// The overview spends a fixed layout budget. The node picker and neighborhoods cover every loaded row.
function networkOverview(graph, choose) {
  const limit = 120;
  const nodes = [...graph.nodes.values()].slice(0,limit);
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
      const lane = points.filter(p => p.node.type === type);
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

function contextNetwork(rows, {focusId, onNext, total} = {}) {
  const root = element("section",{className:"context-network","aria-label":"Context map"});
  const graph = makeContextNetwork(rows);
  if (!graph.nodes.size) return element("p",{className:"empty",text:"No findings match this scope."});
  const search = element("select",{"aria-label":"Focus a node"});
  for (const node of graph.nodes.values()) search.append(element("option",{value:node.id,text:node.type+" · "+node.name}));
  search.prepend(element("option",{value:"",text:"Whole network"}));
  const body = element("div",{});
  const summary = element("span",{text:plural(rows.length,"finding")+(total != null ? " of "+total : "")+" loaded"});
  const legend = element("div",{className:"network-legend"},...["source","finding","outcome"].map(type =>
    element("span",{className:type,text:type === "outcome" ? "Declared or measured value" : type === "source" ? "Source" : "Finding"})));
  const controls = element("div",{className:"network-controls"},element("label",{}, "Focus", search),summary);
  if (onNext) {
    const more = element("button",{type:"button",text:"Load more findings"});
    more.onclick = async () => {
      more.disabled = true;
      try { await onNext(search.value); }
      catch (err) { summary.textContent=err.message;more.disabled=false; }
    };
    controls.append(more);
  }
  function show(id,page=0,keyboard=false) {
    search.value=id;
    if (!id) {
      body.replaceChildren(networkOverview(graph,next=>show(next,0,true)));
      return;
    }
    const node = graph.nodes.get(id);
    const neighbors = graph.edges.filter(e => e.from === id || e.to === id);
    const relations = [...new Set(neighbors.map(e=>e.relation))].join(" · ");
    const drawing = renderNeighborhood(graph,node,page, next => show(next,0,true), next => show(id,next,true));
    body.replaceChildren(drawing,element("div",{className:"network-readout"},
      element("strong",{text:node.name}),element("span",{text:node.description}),
      element("small",{text:plural(neighbors.length,"connection")+" · "+relations}),
      node.href && link("Inspect this evidence →",node.href)));
    if (keyboard) body.querySelector(".network-node.selected").focus({preventScroll:true});
  }
  search.onchange=()=>show(search.value);
  root.append(element("div",{className:"network-heading"},element("h2",{text:"Context map"}),legend),controls,body);
  show(graph.nodes.has(focusId) ? focusId : "");
  return root;
}
