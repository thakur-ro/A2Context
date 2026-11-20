function pendingToolScope(check, map, issue) {
  if (!check) return null;
  const section = element("section", {className:"panel catalog-panel", "aria-label":"Tool catalog scope"});
  section.append(element("h2", {text:issue?.action.title || "Tool catalog matches the workflow scope"}));
  const caption = check.captured == null ? "The assembled tools array is unavailable." :
    plural(check.captured, "tool definition") + " · " + check.outside.length + " outside scope · " + check.unclassified.length + " unclassified";
  section.append(element("p", {className:"catalog-caption", text:caption}));
  const trials = (map?.trials || []).flatMap(report => report.conditions.map(condition => ({
    ...condition, report:report.id, runner:report.runner, baseline:report.conditions[0]
  })));
  const selector = element("select", {"aria-label":"Proposal trial condition"});
  for (const [index, trial] of trials.entries()) {
    const name = trial.edits.length ? "Edit " + trial.edits.map(edit => edit.source).join(" + ") : "Original request";
    selector.append(element("option", {value:index, text:map.trials.length > 1 ? trial.report + " · " + name : name}));
  }
  if (trials.length) section.append(element("label", {className:"catalog-condition"}, "Proposal trials ", selector));
  const trialSummary = element("div", {className:"catalog-caption"});
  const original = [
    {id:"outside", label:"Outside scope", entries:check.outside},
    {id:"permitted", label:"Permitted", entries:map?.permitted || []},
    {id:"unknown", label:"Unclassified", entries:check.unclassified.map(index => ({index}))}
  ];
  const capturedNames = new Set(original.flatMap(group => group.entries.map(entry => entry.name)));
  const search = element("input", {type:"search", placeholder:"Find a function", "aria-label":"Find a function in the catalog"});
  const tree = element("div", {className:"catalog-tree", role:"group", "aria-label":"Catalog branches"});
  const listing = element("div", {className:"catalog-entries"});
  let selected = check.outside.length ? "outside" : check.unclassified.length ? "unknown" : "permitted";
  let page = 0;
  const pageSize = 8;

  function draw() {
    const trial = trials[Number(selector.value)];
    const counts = new Map(Object.entries(trial?.names || {}));
    const baseline = new Map(Object.entries(trial?.baseline.names || {}));
    trialSummary.replaceChildren(trial ? element("details",{},
      element("summary",{text:"Completed trials · " + trial.completed + "/" + trial.attempted}),
      ...trial.edits.map(edit => element("p",{},edit.source + " · ",element("code",{text:edit.pointer}),
        edit.text_range && " · UTF-8 bytes [" + edit.text_range.start_byte + ", " + edit.text_range.end_byte + ")")),
      element("p",{text:trial.runner + " · " + trial.report})) : "No verified proposal trials for this request.");
    const other = [...new Set([...counts.keys(), ...baseline.keys()])].filter(name => !capturedNames.has(name)).map(name => ({name}));
    const groups = [...original, {id:"other",label:"Proposed name without a catalog match",entries:other}].filter(g => g.entries.length);
    const term = search.value.trim().toLowerCase();
    for (const g of groups) {
      g.matches = term ? g.entries.filter(e => ((e.name || "") + " /tools/" + e.index).toLowerCase().includes(term)) : g.entries;
    }
    const group = groups.find(g => g.id === selected && g.matches.length) || groups.find(g => g.matches.length);
    if (group) selected = group.id;
    tree.replaceChildren();
    listing.replaceChildren();
    if (!groups.length) return;
    const root = element("div", {className:"catalog-root"}, element("strong",{text:"Original catalog"}),
      element("small",{text:check.captured == null ? "Unavailable" : plural(check.captured,"tool")}));
    const paths = networkSvg("svg",{viewBox:"0 0 100 100",preserveAspectRatio:"none","aria-hidden":"true",class:"catalog-paths"});
    groups.forEach((branch, i) => {
      const y = 100 * (i + .5) / groups.length;
      paths.append(networkSvg("path",{d:"M 0 50 C 49 50, 54 " + y + ", 100 " + y,
        class:branch === group ? "selected" : "", "vector-effect":"non-scaling-stroke"}));
    });
    const branches = element("div",{className:"catalog-branches"});
    tree.append(root, element("div",{className:"catalog-wires"}, paths), branches);
    for (const branch of groups) {
      const button = element("button",{type:"button",className:"catalog-branch", "aria-pressed":String(branch === group)},
        element("span",{className:"catalog-dot","aria-hidden":"true"}),element("span",{text:branch.label}),
        element("strong",{text:branch.matches.length}));
      button.disabled = !branch.matches.length;
      button.onclick = () => {
        selected = branch.id;
        page = 0;
        draw();
        tree.querySelector('[aria-pressed="true"]')?.focus();
      };
      branches.append(button);
    }
    if (!group) {
      listing.append(element("p",{role:"status",text:"No matching functions."}));
      return;
    }
    const entries = group.matches;
    const start = page * pageSize;
    const list = element("ul", {"aria-label":group.label});
    for (const entry of entries.slice(start, start + pageSize)) {
      const location = entry.index == null ? null : "/tools/" + entry.index;
      const rate = (names, completed) => completed ? (names.get(entry.name) || 0) + "/" + completed : "unmeasured";
      let observed = null;
      if (trial && entry.name) {
        const original = rate(baseline,trial.baseline.completed);
        if (trial.edits.length) observed = "Original " + original + " · Edited " + rate(counts,trial.completed);
        else observed = trial.completed ? "Proposed in " + original + " trials" : "No completed trials";
      }
      list.append(element("li",{},element("div",{},element("strong",{text:entry.name || "Unclassified definition"}),
        location && element("code",{text:location})),
        observed && element("span",{className:counts.has(entry.name) ? "catalog-proposed" : "catalog-count",
          text:observed})));
    }
    listing.append(list);
    if (entries.length > pageSize) {
      const pager = element("div",{className:"catalog-pages"});
      for (const [label, delta, disabled] of [["Previous",-1,page === 0],["Next",1,start + pageSize >= entries.length]]) {
        const button = element("button",{type:"button",text:label});
        button.disabled = disabled;
        button.onclick = () => { page += delta; draw();
          listing.querySelector(".catalog-pages button:not(:disabled)")?.focus(); };
        pager.append(button);
      }
      pager.prepend(element("span",{role:"status",text:(start+1)+"–"+Math.min(start+pageSize,entries.length)+" of "+entries.length}));
      listing.append(pager);
    }
  }
  search.oninput = () => { page = 0; draw(); };
  selector.onchange = () => { page = 0; draw(); };
  section.append(trialSummary);
  if (Math.max(check.captured || 0, ...trials.map(t => Object.keys(t.names).length)) > pageSize) section.append(search);
  section.append(tree, listing);
  if (issue) section.append(element("p",{className:"catalog-next",text:issue.action.next}));
  draw();
  return section;
}
