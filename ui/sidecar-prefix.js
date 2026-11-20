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
    element("small", {text:(run.origin === "sidecar.prefix_trials" ? "Attached trial · " : "Runner · ") + run.runner + " · " + run.report})));
  return element("section", {className:"panel target-check prefix-review", "aria-label":"Source edits at a supplied prefix"},
    element("p", {className:"label",text:"Scores at a supplied prefix"}), element("h2", {text:headline}),
    question && element("blockquote", {text:question}),
    element("p", {}, "With the answer beginning ", element("code", {text:JSON.stringify(prefix)}),
      ", how likely was each continuation?"),
    comparisonTable(["Request", ...alternatives.map(s => JSON.stringify(s)), "All other starts"], rows),
    element("p", {className:"row-extra",text:runs.length ? "The proposal trials below allowed the runner to choose its own opening." :
      selected ? "Test the complete answer after editing " + selected.label + " to see whether the generated text follows this prefix." :
      "Compare the generated answers under each edit. The scores identify no single edit to test first."}),
    review.reference?.set && element("p",{},"Expected beginning in " + review.reference.set + " ",
      element("code",{text:JSON.stringify(prefix + review.reference.continuation)}),
      " · " + prefixOutcomes[review.reference.outcome]),
    review.reference?.issue && element("p",{text:review.reference.issue}),
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

const prefixOutcomes = {
  restored:"Restored the expected continuation", lost:"Lost the expected continuation",
  kept:"Kept the expected continuation", missed:"Missed the expected continuation twice",
  unselected:"No single edit selected", untested:"Awaiting a trial", unresolved:"Trials were unresolved",
  mixed:"Reports gave different reference results"
};

function referenceNavigation(group, selection) {
  const options = [["restored","Restored",group.restored],["lost","Lost",group.lost]];
  return element("nav",{className:"coverage-options","aria-label":"Reference outcomes"},
    ...options.filter(([, ,n]) => n).map(([outcome,label,count]) =>
      link(label + " · " + count,url({...selection,ref_outcome:outcome}),null,query.get("ref_outcome") === outcome)),
    link("All reference results",url({...selection,reference:1}),null,query.get("reference") === "1"));
}

function prefixReferenceDetail(review, question, inspectedIndex) {
  const ref = review.reference;
  const index = inspectedIndex || (review.reversals.length === 1 ? review.reversals[0] : null);
  const edited = index && review.conditions[index];
  const checked = index ? review.runs.filter(r => r.rows[index]) : [];
  const others = review.conditions.slice(1).flatMap((c,i) => i+1 === index ? [] : [[c,ref.edits[i]]]);
  return element("section",{className:"panel prefix-forecast","aria-label":"Reference continuation check"},
    element("p",{className:"label",text:"Reference continuation check"}),element("h2",{text:prefixOutcomes[index ? ref.edits[index - 1] : ref.outcome]}),
    question && element("blockquote",{text:question}),
    element("p",{},"Expected beginning ",element("code",{text:JSON.stringify(review.prefix + ref.continuation)})),
    element("small",{text:"Reference set · " + ref.set}),
    edited && element("p",{},element("b",{text:"Source tested "}),edited.label,
      edited.original && element("span",{text:" · " + edited.original})),
    edited && element("p",{},element("b",{text:"Edit "}),edited.edit.operation === "remove_json" ? "Remove the field." : edited.edit.replacement),
    ...checked.map(run => element("div",{},element("div",{className:"comparison forecast-values"},
      ...[[0,"Original answer"],[index,"After the edit"]].map(([index,label]) => {
        const row = run.rows[index];
        return element("div",{},element("small",{text:label}),row.example != null && element("strong",{text:row.example}),
          prefixTrialResult(row,review.alternatives));
      })),element("small",{text:"Runner · " + run.runner}))),
    others.length > 0 && element("details",{},element("summary",{text:index ? "Compare the other source edits" : "Compare individual source edits"}),
      comparisonTable(["Source","Reference result"],others.map(([c,outcome]) => element("tr",{},
        element("th",{scope:"row",text:c.label}),element("td",{text:prefixOutcomes[outcome]}))))));
}

function prefixInvestigation(group, comparison) {
  const pending = !group.exceptions && group.needs_trial > 0;
  let filter = {}, headline, detail;
  if (comparison) {
    filter = {compare_refs:1};
    headline = "Compare two questions where the same edit helped one answer and hurt the other";
    detail = "Editing " + comparison.source + " restored the expected continuation for one question and lost it for another. The earlier messages and trial settings matched.";
  } else if (group.restored || group.lost) {
    const restored = group.restored > 0;
    filter = {ref_outcome:restored ? "restored" : "lost"};
    headline = "Review " + plural(restored ? group.restored : group.lost,"conversation") + " where an edit " +
      (restored ? "restored" : "lost") + " the expected continuation";
    detail = restored && group.lost ? "Edits also lost the expected continuation in " + plural(group.lost,"conversation") + ". Compare the answers and sources for each result." :
      "Compare the original answer and the edited answer against the reference supplied for each request.";
  } else if (pending && group.forecast_restore) {
    filter = {needs_trial:1,verdict:"untested",ref_prediction:"restored"};
    headline = "Test " + plural(group.forecast_restore,"conversation") + " where scores shifted to the expected continuation";
    detail = "The reference favors the edited prediction. A free-generation trial can check both answers.";
  } else {
    filter = pending ? {verdict:"untested",needs_trial:1} : {};
    headline = group.exceptions ? "Review " + plural(group.exceptions,"conversation") + " where scores and trials need a closer look" :
      pending ? "Test source suggestions from " + plural(group.needs_trial,"conversation") :
      "Inspect " + plural(group.confirmed,"conversation") + " with verified source edits";
    detail = pending ? plural(group.experiments,"distinct edit test") + " across " + plural(group.conversations,"scored conversation") + ". Check whether the score change predicts a change in the generated answer." :
      "Compare the suggested edit with what the runner generated after " + JSON.stringify(group.prefix) + ".";
  }
  return element("section",{className:"investigation"},element("h2",{text:"Recommended investigation"}),
    element("a",{className:"featured",href:url({prefixes:1,design:group.design,...filter})},element("div",{},
      element("h3",{text:headline}),element("p",{text:detail}),
      comparison && element("small",{text:plural(group.referenced,"labeled conversation") + " · Suggested edits restored " +
        "matches in " + plural(group.restored,"conversation") + " and lost them in " + group.lost + "."}))));
}

function prefixExperiment(params) {
  const status = element("p",{"aria-live":"polite"});
  const download = element("button",{type:"button",text:"Download scored input"});
  download.addEventListener("click", async () => {
    download.disabled = true;
    try {
      const input = await api("prefix-input",params,true);
      if (!input) throw new Error("The captured input is unavailable in this scope.");
      downloadJson("scored-input.json",input);
      status.textContent = "Saved the captured request and its source scores.";
    } catch (error) {
      status.textContent = error.message;
    } finally {
      download.disabled = false;
    }
  });
  const upload = element("input",{type:"file",accept:".json,application/json"});
  upload.addEventListener("change",async () => {
    const file = upload.files[0];
    if (!file) return;
    upload.disabled = true;
    try {
      const checked = JSON.parse(await file.text());
      if (!checked.shadow) throw new Error("Choose the checked input written by test-prefix.");
      const response = await fetch("/api/prefix-results",{method:"POST",headers:{"Content-Type":"application/json"},
        body:JSON.stringify({scope:params,shadow:checked.shadow})});
      if (!response.ok) throw new Error((await response.json()).message);
      location.assign(url({prefixes:1,design:params.design,trace:params.trace,call:params.call,report:params.report}));
    } catch (error) {
      status.textContent = error.message;
      upload.disabled = false;
      upload.value = "";
    }
  });
  return element("details",{className:"prefix-experiment"},element("summary",{text:"Prepare a two-request test"}),
    element("p",{text:"Give the original request and the suggested edit to your proposal runner. Let it generate both answers from the beginning."}),
    download,status,element("pre",{},element("code",{text:"nexusweave test-prefix scored-input.json RUNNER_ID checked-input.json -- RUNNER"})),
    element("p",{text:"Replace RUNNER_ID and RUNNER with your runner’s identity and command. Attach checked-input.json here to update this recommendation."}),
    element("label",{},"Attach trial results ",upload));
}

function prefixForecast(review, question, params, captureEnabled, matchingTrial) {
  const edited = review.conditions[review.reversals[0]];
  const predicted = row => JSON.stringify(review.alternatives[row.preferred]);
  return element("section",{className:"panel prefix-forecast","aria-label":"Suggested source test"},
    element("p",{className:"label",text:"Forecast from model scores"}),
    element("h2",{text:"Test the edit to " + edited.label}),
    question && element("blockquote",{text:question}),
    element("p",{},"After ",element("code",{text:JSON.stringify(review.prefix)}),", the model favored these continuations."),
    element("div",{className:"comparison forecast-values"},
      ...[[review.conditions[0],"Original request"],[edited,"After the source edit"]].map(([row,label]) =>
        element("div",{},element("small",{text:label}),element("strong",{text:predicted(row)})))),
    edited.original && element("p",{},element("b",{text:"Captured source "}),edited.original),
    element("p",{},element("b",{text:"Test edit "}),edited.edit.operation === "remove_json" ? "Remove the field." : edited.edit.replacement),
    review.reference?.set && element("p",{},element("b",{text:"Expected continuation "}),
      element("code",{text:JSON.stringify(review.reference.continuation)})," · " + review.reference.set),
    matchingTrial ? link("See the matching trial in " + matchingTrial.thread,url({prefixes:1,design:params.design,
      trace:matchingTrial.trace,call:matchingTrial.call,report:matchingTrial.report}),"action") :
    captureEnabled ? prefixExperiment(params) : link("Check a scored input in Before inference",url({view:"preview"})),
    element("details",{},element("summary",{text:"Inspect the model scores"}),prefixReview(review,null)));
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

async function referenceComparisons(page, selection, offset) {
  if (!page.rows.length) return panel("No matching pair in this scope",link("Review source results",url(selection)));
  const params = {...selection,compare_refs:1,...(offset && {offset})};
  const id = pair => JSON.stringify([pair.setup,pair.edit,pair.reference_set]);
  const active = Math.max(0,page.rows.findIndex(pair => id(pair) === query.get("matched")));
  const pair = page.rows[active];
  const locator = row => ({design:selection.design,ref_set:pair.reference_set,pair_setup:pair.setup,
    ref_edit:pair.edit,edit_outcome:row.outcome,edit_index:row.index,trace:row.trace,call:row.call,report:row.report});
  const first = pair.examples[0];
  const witness = await api("prefix-check",{...scope,...locator(first)},true);
  if (!witness) return panel("The comparison changed. Reopen it from the recommendations.",link("Back to recommendations",url({})));
  const condition = witness.review.conditions[first.index];
  const pick = element("select",{"aria-label":"Matched source edits"},...page.rows.map((p,i) => element("option",
    {value:i,text:p.source + " · " + p.examples.map(r => r.thread).join(" / ")})));
  pick.value = String(active);
  pick.addEventListener("change",() => location.assign(url({...params,matched:id(page.rows[Number(pick.value)])})));
  return element("section",{className:"panel prefix-contrast","aria-label":"Matched source edit"},
    element("h2",{text:"Editing " + pair.source}),page.rows.length > 1 && element("label",{},"Compare ",pick),
    condition.original != null && element("p",{},element("b",{text:"Captured source "}),condition.original),
    element("p",{},element("b",{text:"Tested edit "}),condition.edit.operation === "remove_json" ? "Remove the field." : condition.edit.replacement),
    element("div",{className:"comparison question-contrast"},...pair.examples.map(row =>
      element("article",{className:"reference-" + row.outcome},element("small",{text:row.thread}),
        element("h3",{text:row.outcome === "restored" ? "Restored the expected value" : "Lost the expected value"}),
        element("blockquote",{text:row.question}),element("p",{},"Generated continuation ",
          element("strong",{text:JSON.stringify(row.original) + " → " + JSON.stringify(row.edited)})),
        element("p",{text:"Expected " + JSON.stringify(row.expected)}),
        link("Inspect this edit",url({...selection,compare_refs:1,matched:id(pair),...locator(row),from_offset:offset || null}))))),
    element("p",{className:"row-extra",text:"The final user question changed. The earlier request, source edits, runner, sampling settings and trial counts matched. Reference set · " + pair.reference_set}));
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
  const filter = Object.fromEntries(["source","verdict","unnamed","needs_trial","reference","ref_outcome","ref_set","ref_prediction"].filter(k => query.has(k)).map(k => [k,query.get(k)]));
  const params = {...scope,design,...filter};
  if (traceId) {
    const edit = query.has("ref_edit") ? Object.fromEntries(["ref_edit","pair_setup","edit_outcome","edit_index"].map(k => [k,query.get(k)])) : {};
    const locator = {...params,...edit,trace:traceId,call:query.get("call"),report:query.get("report")};
    const data = await api("prefix-check",locator,true);
    const comparing = query.has("compare_refs");
    const previous = link(comparing ? "Back to matched questions" : "Back to source results",url({...selection,
      ...(comparing ? {compare_refs:1} : filter),matched:query.get("matched"),offset:query.get("from_offset")}));
    if (!data) return panel("Comparison unavailable in this scope",previous);
    byId("title").textContent = data.example.thread;
    byId("summary").textContent = edit.ref_edit ? "Observed result for the source edit in the matched comparison." : data.matching_trial ? "An identical input and edit have trial results in this scope." : prefixVerdicts[data.review.verdict];
    const referenceDetail = data.review.reference?.set && data.review.verdict !== "untested";
    return element("div",{className:"feed"},previous,
      referenceDetail && prefixReferenceDetail(data.review,data.example.question,Number(edit.edit_index)),
      data.review.verdict === "untested" ? prefixForecast(data.review,data.example.question,locator,data.capture_enabled,data.matching_trial) :
        referenceDetail ? element("details",{className:"panel"},element("summary",{text:"Inspect scores and all trial conditions"}),prefixReview(data.review,null)) :
          prefixReview(data.review,data.example.question),
      link("Read this conversation",url({view:"threads",thread:data.example.thread_key,focus_trace:traceId,focus_call:data.example.call})));
  }
  if (query.has("compare_refs")) {
    const pairs = await api("prefix-reference-pairs",{...scope,design,offset:pageOffset});
    byId("title").textContent = "When the same edit helps and hurts";
    byId("summary").textContent = "Compare the measured effect on two questions before applying the edit more widely.";
    pagination(pairs.next_offset,{...selection,compare_refs:1});
    return element("div",{className:"feed"},back,await referenceComparisons(pairs,selection,pageOffset),
      link("See all reference results",url({...selection,reference:1})));
  }
  const references = Boolean(filter.reference || filter.ref_outcome || filter.ref_prediction);
  const [overview, breakdown, contrasts, examples] = await Promise.all([api("prefix-groups",{...scope,design}),
    api(references ? "prefix-references" : "prefix-sources",{...params,offset:sourceOffset}),
    filter.verdict || references ? {rows:[]} : api("prefix-contrasts",{...scope,design,offset:pairOffset}),
    api("prefix-cases",{...params,offset:pageOffset})]);
  const group = overview.rows[0];
  if (!group) return panel("No prefix comparisons in this scope",back);
  byId("title").textContent = filter.ref_outcome ? prefixOutcomes[filter.ref_outcome] : filter.reference ? "How did the answers compare with the reference?" : filter.verdict === "untested" ? "Which source edit should I test?" : "Which source suggestions held up?";
  byId("summary").textContent = plural(group.conversations,"conversation") + " · " + plural(group.inputs,"distinct input") +
    " · Scores after " + JSON.stringify(group.prefix);
  pagination(examples.next_offset,{...selection,...filter});
  const pairs = group.matched_questions > 1 && !filter.source && !filter.ref_outcome && !filter.verdict ?
    await api("prefix-reference-pairs",{...scope,design}) : null;
  const rows = breakdown.rows.map(row => {
    const status = references ? prefixOutcomes[row.outcome] : prefixVerdicts[row.verdict];
    const target = {...selection,...filter,...(references ? {ref_outcome:row.outcome,ref_set:row.reference_set} : {verdict:row.verdict}),
      source:row.source,unnamed:row.source ? null : 1};
    const noSelection = references ? row.outcome === "unselected" : ["no_selection","unselected_change"].includes(row.verdict);
    const active = (filter.source || null) === row.source && (references ?
      filter.ref_outcome === row.outcome && filter.ref_set === row.reference_set : filter.verdict === row.verdict);
    return element("tr",{},element("th",{scope:"row",text:row.source || (noSelection ? "No single edit" : "Unnamed source")}),
      references && element("td",{text:row.reference_set}),element("td",{},link(status,url(target),null,active)),
      element("td",{text:row.conversations}));
  });
  return element("div",{className:"feed"},back,group.referenced > 0 && referenceNavigation(group,selection),
    pairs?.rows.length > 0 && link("Compare the same edit across two questions",url({...selection,compare_refs:1})),
    element("section",{className:"panel target-check"},element("h2",{text:"Results by suggested source"}),
    comparisonTable(["Source",...(references ? ["Reference set"] : []),"What the trials showed","Conversations"],rows),
    sourceOffset > 0 && link("Previous sources",url({...selection,...filter,source_offset:Math.max(0,sourceOffset-15)})),
    breakdown.next_offset != null && link("More sources",url({...selection,...filter,source_offset:breakdown.next_offset})),
    element("p",{className:"label",text:references ? "Each row counts conversations once. A match requires the generated answer to begin with the supplied prefix and its declared continuation." : "Each row counts conversations once. A confirmed switch measures how the answer changed. Whether the edit improved it requires a reference."})),
    questionContrasts(contrasts,selection,pairOffset),
    panel(filter.verdict ? prefixVerdicts[filter.verdict] : "Conversations",
      (filter.verdict || references) && link("Show all results",url(selection)),listing(examples.rows,row => rowLink(row.question || row.thread,
        [row.thread,row.source,prefixVerdicts[row.verdict]].filter(Boolean).join(" · "),
        url({...selection,...filter,trace:row.trace,call:row.call,report:row.report,from_offset:pageOffset || null})))),
    link("All scored prefixes",url({prefixes:1})));
}
