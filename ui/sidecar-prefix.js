"use strict";

function prefixReview(review, question, action) {
  const {prefix, alternatives, conditions, reversals, runs} = review;
  const selected = reversals.length === 1 ? conditions[reversals[0]] : null;
  const headline = selected ? "Editing " + selected.label + " switched the preferred continuation" :
    reversals.length ? "Several edits switched the preferred continuation" : "No edit switched the preferred continuation";
  const percent = new Intl.NumberFormat(undefined, {style:"percent", maximumSignificantDigits:3});
  const rows = conditions.map(row => {
    const source = row.edit ? element("details", {}, element("summary", {text:row.label}),
      element("small", {text:[row.version && "Version " + row.version, row.edit.pointer].filter(Boolean).join(" · ")}),
      row.original != null && element("small", {text:"Captured: " + JSON.stringify(row.original)}),
      element("small", {text:row.edit.operation === "remove_json" ? "Removed" : "Replacement: " + JSON.stringify(row.edit.replacement)})) : "Original";
    return element("tr", {}, element("th", {scope:"row"}, source),
      ...[...row.probabilities, row.outside].map((p,i) => element("td", {},
        element("span", {text:percent.format(p)}),
        element("meter", {value:p,min:0,max:1,"aria-label":(alternatives[i] || "Other starts") + " probability"}),
        i === row.preferred && element("small", {text:review.labels[i] + " favored"}))));
  });
  const trials = runs.map(run => panel("What did the runner generate?",
    comparisonTable(["Request","Generated answer"], run.rows.map((row,i) =>
      element("tr", {}, element("th", {scope:"row",text:conditions[i].label}),
        element("td", {}, row?.example != null && element("code", {text:row.example}), prefixTrialResult(row, alternatives))))),
    element("small", {text:"Runner · " + run.runner + " · " + run.report})));
  return element("section", {className:"panel target-check prefix-review", "aria-label":"Source edits at a supplied prefix"},
    element("p", {className:"label",text:"Scores at a supplied prefix"}), element("h2", {text:headline}),
    question && element("blockquote", {text:question}),
    element("p", {}, "With the answer beginning ", element("code", {text:JSON.stringify(prefix)}),
      ", how likely was each continuation?"),
    comparisonTable(["Request", ...alternatives.map(s => JSON.stringify(s)), "All other starts"], rows),
    element("p", {className:"row-extra",text:runs.length ? "The proposal trials below allowed the runner to choose its own opening." :
      selected ? "Test the complete answer after editing " + selected.label + " to see whether the generated text follows this prefix." :
      "Compare the generated answers under each edit. The scores identify no single edit to test first."}),
    action, ...trials,
    element("details", {}, element("summary", {text:"Scoring conditions"}),
      ...conditions.slice(1).map(c => element("p", {text:c.label + " · " + signed(c.change_log_odds) + " change in log odds"})),
      element("p", {text:[review.model, review.revision, review.runtime, review.report].join(" · ")}),
      element("p", {text:"Each probability covers a token sequence beginning immediately after the supplied prefix. Other continuations retain the remaining probability."})));
}

function prefixTrialResult(row, alternatives) {
  if (!row) return "Edit untested in this report";
  const counts = row.starts.map((n,i) => n && n + " began with the prefix and " + JSON.stringify(alternatives[i]));
  const other = row.followed - row.starts[0] - row.starts[1];
  const missed = row.text - row.followed;
  counts.push(other && other + " followed the prefix without a unique continuation match",
    missed && missed + " used a different opening",
    row.other_outputs && row.other_outputs + " returned a non-text proposal",
    row.failed && row.failed + " incomplete");
  return element("small", {text:plural(row.attempted, "trial") + " · " + counts.filter(Boolean).join(" · ")});
}

const prefixVerdicts = {
  confirmed:"Generated answers followed the predicted switch",
  disagreed:"Generated answers contradicted the predicted switch",
  mixed:"Reports gave different results",
  unresolved:"Trials did not resolve both continuations",
  untested:"Suggested edit still needs a trial",
  unselected_change:"Answers switched without a single suggested edit",
  no_selection:"Scores identified no single edit to test"
};

function prefixGroupRow(group) {
  return rowLink(JSON.stringify(group.prefix) + " · " + group.alternatives.map(s => JSON.stringify(s)).join(" or "),
    plural(group.conversations,"conversation") + " · " + plural(group.inputs,"distinct input") + " · " + group.model + " · " + group.scorer,
    url({prefixes:1,design:group.design}));
}

function prefixInvestigation(group) {
  const headline = group.exceptions ? "Review " + plural(group.exceptions,"conversation") + " where scores and trials need a closer look" :
    "Inspect " + plural(group.confirmed,"conversation") + " with verified source edits";
  return element("section",{className:"investigation"},element("h2",{text:"Recommended investigation"}),
    element("a",{className:"featured",href:url({prefixes:1,design:group.design})},element("div",{},
      element("h3",{text:headline}),element("p",{text:"Compare the suggested edit with what the runner generated after " +
        JSON.stringify(group.prefix) + "."}))));
}

function questionContrasts(page, selection, offset) {
  if (!page.rows.length) return null;
  const pairs = page.rows.map(pair => element("div",{className:"comparison question-contrast"},
    ...pair.examples.map(row => element("article",{},element("small",{text:row.thread}),
      element("blockquote",{text:row.question}),element("strong",{text:row.source}),
      element("p",{text:JSON.stringify(row.original) + " → " + JSON.stringify(row.edited)}),
      link("Inspect this edit",url({...selection,matched:pair.setup,trace:row.trace,call:row.call,report:row.report}))))));
  const active = Math.max(0,page.rows.findIndex(pair => pair.setup === query.get("matched")));
  const container = element("div",{},pairs[active]);
  const pick = element("select",{"aria-label":"Matched questions"},...pairs.map((_,i) => element("option",
    {value:i,text:page.rows[i].examples.map(r => r.thread).join(" / ")})));
  pick.value = String(active);
  pick.addEventListener("change", () => {
    const index = Number(pick.value);
    container.replaceChildren(pairs[index]);
    history.replaceState(null,"",url({...selection,matched:page.rows[index].setup}));
  });
  return element("section",{className:"panel target-check prefix-contrast"},
    element("h2",{text:"Two questions pointed to different verified edits"}),
    element("p",{text:"Earlier messages, model settings, source edits and trial design matched. Each selected edit switched the generated continuation in the predicted direction."}),
    pairs.length > 1 && element("label",{},"Compare questions ",pick),container,
    offset > 0 && link("Previous comparisons",url({...selection,pair_offset:Math.max(0,offset-15)})),
    page.next_offset != null && link("More comparisons",url({...selection,pair_offset:page.next_offset})));
}

async function prefixCohort() {
  const design = query.get("design");
  const sourceOffset = Number(query.get("source_offset")) || 0;
  const pairOffset = Number(query.get("pair_offset")) || 0;
  const selection = {prefixes:1,design,...(pairOffset && {pair_offset:pairOffset}),...(sourceOffset && {source_offset:sourceOffset})};
  const back = link("Back to recommendations",url({}));
  if (!design) {
    const groups = await api("prefix-groups",{...scope,offset:pageOffset});
    byId("title").textContent = "Answer prefixes";
    byId("summary").textContent = "Choose a scoring setup to inspect its source suggestions.";
    pagination(groups.next_offset,selection);
    return panel("Scored across conversations",back,listing(groups.rows,prefixGroupRow));
  }
  const filter = Object.fromEntries(["source","verdict","unnamed"].filter(k => query.has(k)).map(k => [k,query.get(k)]));
  const params = {...scope,design,...filter};
  if (traceId) {
    const data = await api("prefix-check",{...params,trace:traceId,call:query.get("call"),report:query.get("report")},true);
    const previous = link("Back to source results",url({...selection,...filter,matched:query.get("matched"),offset:query.get("from_offset")}));
    if (!data) return panel("Comparison unavailable in this scope",previous);
    byId("title").textContent = data.example.thread;
    byId("summary").textContent = prefixVerdicts[data.review.verdict];
    return element("div",{className:"feed"},previous,prefixReview(data.review,data.example.question),
      link("Read this conversation",url({view:"threads",thread:data.example.thread_key,focus_trace:traceId,focus_call:data.example.call})));
  }
  const [overview, breakdown, contrasts, examples] = await Promise.all([api("prefix-groups",{...scope,design}),
    api("prefix-sources",{...scope,design,offset:sourceOffset}),
    filter.verdict ? {rows:[]} : api("prefix-contrasts",{...scope,design,offset:pairOffset}),
    api("prefix-cases",{...params,offset:pageOffset})]);
  const group = overview.rows[0];
  if (!group) return panel("No prefix comparisons in this scope",back);
  byId("title").textContent = "Which source suggestions held up?";
  byId("summary").textContent = plural(group.conversations,"conversation") + " · " + plural(group.inputs,"distinct input") +
    " · Scores after " + JSON.stringify(group.prefix);
  pagination(examples.next_offset,{...selection,...filter});
  const rows = breakdown.rows.map(row => {
    const target = {...selection,verdict:row.verdict,...(row.source ? {source:row.source} : {unnamed:1})};
    return element("tr",{},element("th",{scope:"row",text:row.source || (["no_selection","unselected_change"].includes(row.verdict) ? "No single edit" : "Unnamed source")}),
      element("td",{},link(prefixVerdicts[row.verdict],url(target),null,filter.verdict === row.verdict && (filter.source || null) === row.source)),
      element("td",{text:row.conversations}));
  });
  return element("div",{className:"feed"},back,element("section",{className:"panel target-check"},element("h2",{text:"Results by suggested source"}),
    comparisonTable(["Source","What the trials showed","Conversations"],rows),
    sourceOffset > 0 && link("Previous sources",url({...selection,...filter,source_offset:Math.max(0,sourceOffset-15)})),
    breakdown.next_offset != null && link("More sources",url({...selection,...filter,source_offset:breakdown.next_offset})),
    element("p",{className:"label",text:"Each row counts conversations once. A confirmed switch measures how the answer changed. Whether the edit improved it requires a reference."})),
    questionContrasts(contrasts,selection,pairOffset),
    panel(filter.verdict ? prefixVerdicts[filter.verdict] : "Conversations",
      filter.verdict && link("Show all results",url(selection)),listing(examples.rows,row => rowLink(row.question || row.thread,
        [row.thread,row.source,prefixVerdicts[row.verdict]].filter(Boolean).join(" · "),
        url({...selection,...filter,trace:row.trace,call:row.call,report:row.report,from_offset:pageOffset || null})))),
    link("All scored prefixes",url({prefixes:1})));
}
