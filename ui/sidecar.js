"use strict";

const byId = (id) => document.getElementById(id);
const query = new URLSearchParams(location.search);
const view = ["traces", "threads", "preview"].includes(query.get("view")) ? query.get("view") : "findings";
const filterNames = { after: "From", before: "Before", org: "Organization", user: "User",
  workflow: "Workflow", meta_key: "Metadata key", meta_value: "Metadata value" };
const scope = Object.fromEntries(Object.keys(filterNames).filter((key) => query.has(key)).map((key) => [key, query.get(key)]));
const kindLabels = { baseline_check:"Replay baseline", source_gap:"Source capture gap", prefill: "First-token test", replacement: "Replacement comparison", conflict: "Rule conflict", duplicate: "Exact repetition", replay: "Verified replay",
  candidate: "Candidate scores", shadow: "Proposal trials", token: "Token measurements", source_mask: "Source mask trials" };
const recommendationId = query.get("recommendation");
const sourceHistoryKey = query.get("source_key");
const factorId = query.get("factor");
const dispatchOverview = query.get("dispatches") === "1";
const dispatchSelection = query.has("dispatch_design") && query.has("dispatch_outcome") ? {
  dispatch_design:query.get("dispatch_design"), dispatch_outcome:query.get("dispatch_outcome"),
  ...(dispatchOverview && {dispatches:1}),
} : null;
const openingStudy = query.get("opening") === "1";
const openingSlot = query.get("slot");
const conflictDesign = query.get("conflict_design");
const conflictCoverage = query.get("conflict_coverage");
const comparisonResult = query.get("proposal_result");
const comparisonScope = query.get("coverage_factor");
const prefixCohorts = query.get("prefixes") === "1";
const targetPatterns = query.get("targets") === "1";
const ranking = query.get("ranking");
const quality = query.get("quality");
const review = query.get("review");
const targetDesign = query.get("design");
const missSignal = query.get("miss_signal");
const targetSelection = targetPatterns ? {targets:1, design:targetDesign, miss_signal:missSignal} : {};
const screenedPairs = query.get("screens") === "1";
const screenedPattern = query.get("pattern");
const screenedRelation = query.get("relation");
const screenSelection = screenedPairs ? {screens:1, pattern:screenedPattern, relation:screenedRelation} : {};
const withoutFindings = query.get("coverage") === "without-findings";
const traceId = query.get("trace");
const measuredExample = Object.fromEntries(["trace", "call", "decision", "replay", "context", "routing_design", "routing_input", "report"]
  .filter((key) => recommendationId && query.has(key)).map((key) => [key, query.get(key)]));
const jointSelection = query.has("joint_pattern") ? {joint_pattern:query.get("joint_pattern"), baseline_trials:query.get("baseline_trials")} : null;
const positionPair = query.get("pair");
const positionDesign = positionPair ? Object.fromEntries(["group", "baseline_trials", "edit_trials"]
  .filter((key) => query.has(key)).map((key) => [key, query.get(key)])) : {};
const threadId = query.get("thread");
const pageOffset = Number(query.get("offset")) || 0;
const dateFormat = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short", timeZone: "UTC" });

function element(tag, { text, className, ...attributes } = {}, ...children) {
  const node = document.createElement(tag);
  if (text !== undefined) node.textContent = text;
  if (className) node.className = className;
  Object.entries(attributes).forEach(([name, value]) => node.setAttribute(name, value));
  node.append(...children.filter(Boolean));
  return node;
}

function link(label, href, className, current = false) {
  return element("a", { text: label, href, className, ...(current && { "aria-current": "page" }) });
}

function downloadJson(filename, value) {
  const href = URL.createObjectURL(new Blob([JSON.stringify(value, null, 2)], {type:"application/json"}));
  element("a", {href, download:filename}).click();
  setTimeout(() => URL.revokeObjectURL(href), 1000);
}

function url(params, filters = scope) {
  const search = new URLSearchParams(filters);
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined || value === null || value === "") search.delete(key);
    else search.set(key, value);
  }
  return search.size ? "?" + search : "/";
}

// The sidecar serves dashboard queries from one SQLite reader. Issue one read at
// a time per tab so a detail page does not compete for idle loopback connections.
let lastRead = Promise.resolve();
function api(path, params = {}, allowMissing = false) {
  const result = lastRead.then(() => readEvidence(path, params, allowMissing));
  lastRead = result.then(() => {}, () => {});
  return result;
}

async function readEvidence(path, params, allowMissing) {
  const response = await fetch("/api/sidecar/" + path + "?" + new URLSearchParams(params));
  if (allowMissing && response.status === 404) return null;
  if (!response.ok) throw new Error((await response.json().catch(() => ({}))).message || "Evidence could not be loaded.");
  return response.json();
}

const plural = (count, name) => count + " " + name + (count === 1 ? "" : "s");
const fullDate = (seconds) => seconds == null ? "Time unavailable" : dateFormat.format(new Date(seconds * 1000)) + " UTC";
const sourceName = (item) => item.source || item.source_id || item.id || "Source ID unavailable";
const version = (value) => value ? " · " + value : "";

function sourceCells(sources, values = []) {
  return element("div", { className: "comparison" }, ...sources.map((source, i) => element("div", {},
    element("strong", { text: sourceName(source) }), element("span", { className: "label", text: source.role || "Role unavailable" }),
    source.version ? element("span", { className: "label", text: " · " + source.version }) : null,
    values[i] === undefined ? null : element("div", {}, element("code", { text: values[i] })))));
}

function trend(rows) {
  if (!rows.length) return element("p", { text: "Capture times unavailable" });
  const max = Math.max(...rows.map((row) => row.count));
  return element("ul", { className: "trend" }, ...rows.slice(0, 7).reverse().map((row) => element("li", {},
    element("time", { text: row.day.slice(5), datetime: row.day }),
    element("meter", { min: 0, max, value: row.count, "aria-label": row.day + " · " + plural(row.count, "observation") }),
    element("span", { text: row.count }))));
}

function replayTrials(detail) {
  const m = detail.measurement;
  const original = detail.response_pair?.original;
  const proposals = [detail.response_pair?.edited, ...(detail.single_edits || []).map(arm => arm.proposal)].filter(Boolean);
  const sameAnswerLabel = original?.kind === "model_output" && proposals.every(p => p.kind === original.kind && p.label === original.label);
  const proposalText = p => p && (sameAnswerLabel ? p.outcome_preview : [p.kind, p.label, p.outcome_preview].join(" · "));
  const singles = (detail.single_edits || []).map(arm => ["Edit " + arm.id, arm.summary, proposalText(arm.proposal), arm.id]);
  const pair = detail.edit === "joint" && detail.response_pair;
  const arms = [["Baseline", m.baseline, pair ? proposalText(pair.original) : detail.baseline_answer], ...singles,
    [detail.edit === "joint" ? "Joint edit" : "Source edit", m.variant, pair ? proposalText(pair.edited) : detail.edited_answer], ["Control edit", m.control]];
  return element("div", { className: "comparison" }, ...arms.filter(([, data]) => data).map(([label, data, answer, id]) =>
    element("div", id ? {"data-replay-arm":id} : label === "Joint edit" ? {"data-joint-edit":""} : {}, element("strong", { text: label }),
      element("span", { text: data.matched + " / " + data.attempted + " matched" }),
      answer == null ? null : element("code", { className: "arm-answer", text: JSON.stringify(answer) }),
      element("meter", { min: 0, max: Math.max(1, data.attempted), value: data.matched, "aria-label": label + " matched trials out of attempted trials" }),
      data.unscored || data.completed < data.attempted ? element("div", { className: "label", text: plural(data.unscored || 0, "unscored trial") + " · " + data.completed + " completed" }) : null)));
}

function signed(number) {
  const value = number !== 0 && Math.abs(number) < 0.001 ? number.toExponential(2) : Number(number.toFixed(3));
  return (number > 0 ? "+" : "") + value;
}

function candidateVisual(detail) {
  const alternatives = detail.alternatives.map((entry) => entry.id);
  const percent = new Intl.NumberFormat(undefined, { style: "percent", maximumFractionDigits: 1 });
  const readout = (label, margin, outside) => element("div", {},
    element("strong", { text: label }),
    element("span", { text: margin === 0 ? "Tied" : alternatives[margin > 0 ? 0 : 1] + " favored" }),
    element("p", { className: "label", text: signed(margin) + " log odds" }),
    outside == null ? null : element("p", { text: percent.format(outside) + " probability outside this pair" }));
  return element("div", { className: "comparison" }, readout("Original request", detail.baseline, detail.baseline_outside_pair),
    readout("One source edited", detail.edited, detail.edited_outside_pair));
}

function tokenVisual(landmark) {
  const max = Math.max(...landmark.positions.map(({ baseline, edited }) => Math.abs(edited - baseline)));
  return element("ol", { className: "trend tokens" }, ...landmark.positions.map(({ token, fragment, baseline, edited }) =>
    element("li", { ...(token === landmark.token && { className: "selected-token" }) },
      element("code", { text: JSON.stringify(fragment), title: "Output token " + (token + 1) }),
      element("meter", { min: 0, max, value: Math.abs(edited - baseline),
        "aria-label": "Output token " + (token + 1) + " · change " + signed(edited - baseline) }),
      element("span", { text: signed(edited - baseline) }))));
}

const tokenPercent = new Intl.NumberFormat(undefined, {style:"percent", maximumSignificantDigits:4});
const tokenProbability = logProbability => tokenPercent.format(Math.exp(logProbability));

function trialTable(headers, rows) {
  const headings = headers.map(text => element("th", { text, scope: "col" }));
  function countCell(arm) {
    return element("td", {},
      element("span", { text: arm.matched + " / " + arm.attempted }),
      arm.answer == null ? null : element("code", { className: "arm-answer", text: JSON.stringify(arm.answer) }));
  }
  const body = rows.map(([label, a, b]) => element("tr", { className: a.matched !== b.matched ? "different" : "" },
    element("th", { text: label, scope: "row" }), countCell(a), countCell(b)));
  return element("table", {}, element("thead", {}, element("tr", {}, ...headings)), element("tbody", {}, ...body));
}

function replacementVisual(detail) {
  const [left, right] = detail.conditions;
  function wording(condition, label) {
    const edits = condition.edits;
    const same = edits.every((edit) => edit.replacement === edits[0].replacement);
    return element("div", {}, element("strong", { text: label }),
      ...((same ? edits.slice(0, 1) : edits).map((edit) =>
        element("p", { text: (same ? "" : edit.id + " · ") + (edit.replacement || "(Removed text)") }))));
  }
  return element("div", { className: "replacement" },
    element("div", { className: "comparison" }, wording(left, "Wording A"), wording(right, "Wording B")),
    trialTable(["Edit", "Wording A", "Wording B"], [["Original", left.baseline, right.baseline],
      ...left.edits.map((edit, i) => [edit.id, edit, right.edits[i]]),
      ...(left.joint ? [["Both sources", left.joint, right.joint]] : [])]));
}

function orderComparison({ forward, reverse, differs }) {
  const [a, b] = forward.edits;
  const first = [a.source + " first", b.source + " first"];
  const rows = [["Original", forward.baseline, reverse.baseline],
    ...forward.edits.map((arm, i) => ["Edit " + arm.source, arm, reverse.edits[i]])];
  function relativeToBaseline(row, index) {
    const arm = row.edits[index];
    return Math.sign(arm.matched * row.baseline.attempted - row.baseline.matched * arm.attempted);
  }
  const preservedEarlier = relativeToBaseline(forward, 0) === 0 && relativeToBaseline(reverse, 1) === 0;
  const lowerLater = relativeToBaseline(forward, 1) < 0 && relativeToBaseline(reverse, 0) < 0;
  const wording = a.replacement === b.replacement ? [a.replacement] :
    forward.edits.map(arm => arm.source + " · " + arm.replacement);
  const trials = [forward, reverse].map((row, i) => {
    const measurement = { location: { decision_id: row.decision }, replay_id: row.report, context_id: row.edits[0].id };
    const href = advancedHref({ id: row.trace }, { id: row.call }, { detail: { measurement } });
    return element("p", {}, link(row.thread || first[i], href));
  });
  return element("section", { className: "panel replacement" },
    element("h2", { text: differs ? "Match counts changed with order" : "Match counts held across orders" }),
    element("p", { text: forward.model + " · " + wording.map(text => JSON.stringify(text)).join(" / ") }),
    preservedEarlier && lowerLater ? element("p", { text: "In both orders, editing the earlier source preserved the match rate. Editing the later source lowered it." }) : null,
    trialTable(["Edited input", ...first], rows),
    element("details", {}, element("summary", { text: "Original trials" }), ...trials));
}

function requiredSourceList(coverage) {
  return element("ul",{},...coverage.alternatives.map(group => element("li",{
    text:sourceAlternativeText(coverage, group)
  })));
}

function sourceMismatchEvidence(detail) {
  const fragments = detail.sources.filter(source => source.request_fragment && source.captured_text_sha256);
  if (!fragments.length) return null;
  return element("details", {}, element("summary", {text:"Inspect differing fragments"}), ...fragments.map(source => {
    const {pointer, text_range} = source.request_fragment;
    return element("div", {}, element("strong", {text:sourceName(source)}),
      element("p", {}, element("code", {text:pointer}), " · UTF-8 bytes ["+text_range.start_byte+", "+text_range.end_byte+")"),
      element("p", {}, "Expected SHA-256 ", element("code", {text:detail.coverage.expected_text_sha256[source.source]})),
      element("p", {}, "Captured SHA-256 ", element("code", {text:source.captured_text_sha256})));
  }));
}

function overlappingAnchors(report) {
  if (!report?.shared_bytes) return null;
  function drawRange({pointer,text_range:shared,sources,source_count:count}) {
    const left = Math.min(...sources.map(s => s.text_range.start_byte));
    const right = Math.max(...sources.map(s => s.text_range.end_byte));
    function band(range, className) {
      const node = element("span",{className});
      node.style.left = 100*(range.start_byte-left)/(right-left)+"%";
      node.style.width = 100*(range.end_byte-range.start_byte)/(right-left)+"%";
      return node;
    }
    const label = r => "["+r.start_byte+", "+r.end_byte+")";
    return element("figure",{className:"anchor-ranges"},
      element("figcaption",{},element("code",{text:pointer})," · "+count+" source IDs share bytes "+label(shared)),
      ...sources.map(s => element("div",{},element("span",{text:s.source_id+"  "+label(s.text_range)}),
        element("div",{className:"byte-track","aria-hidden":"true"},band(s.text_range,"named-range"),band(shared,"shared-range")))));
  }
  return element("section",{className:"panel","aria-label":"Overlapping source anchors"},
    element("h2",{text:"Check the boundaries of your source edit"}),
    element("p",{text:report.shared_bytes+" of "+report.named_bytes+" named UTF-8 bytes belong to more than one source ID. Changing those bytes edits both sources."}),
    ...report.examples.map(drawRange),
    element("small",{text:"First shared range per field, showing "+report.examples.length+" of "+plural(report.shared_fields,"field")+". Select disjoint ranges to test the sources separately."}));
}

function visual(kind, detail, rows = []) {
  if (kind === "baseline_check") {
    const {attempted,completed,unscored=0,matched} = detail.measurement.baseline;
    return comparisonTable(["Original request","Trials"],[
      ["Matched",matched],["Scored and differed",completed-unscored-matched],
      ["Could not score",unscored],["Runner failed",attempted-completed]
    ].map(([label,count]) => element("tr",{},element("th",{scope:"row",text:label}),element("td",{text:count}))));
  }
  if (kind === "source_gap") return element("div",{},requiredSourceList(detail.coverage),sourceMismatchEvidence(detail));
  if (kind === "prefill") return firstTokenPreview(detail, true);
  if (kind === "source_mask") return maskComparison(detail.mask);
  const [label, body] = kind === "replacement" ? ["Trials matching the recorded outcome", replacementVisual(detail)] :
    kind === "conflict" ? ["Captured declarations", sourceCells(detail.sources, detail.values)] :
    kind === "replay" || kind === "shadow" ? ["Matches by trial arm", replayTrials(detail)] :
    kind === "candidate" ? [detail.alternatives.map((entry) => entry.id).join(" vs ") + " at a fixed prefix", candidateVisual(detail)] :
    kind === "token" ? ["Change in output-token log probability", tokenVisual(detail.landmark)] :
    rows.length ? ["Occurrences by day (UTC)", trend(rows)] : ["Repeated sources", sourceCells(detail.sources)];
  return element("div", { className: "visual" }, element("span", { className: "label", text: label }), body);
}

const jointPatterns = {
  joint_only: "Only the joint edit changed the proposal",
  single_and_joint: "A single edit and the joint edit each changed the proposal",
  single_only: "A single edit changed the proposal and the joint edit restored it",
  unchanged: "All four conditions produced the same proposal",
  unresolved: "At least one condition has no consistent proposal",
};

function findingTitle(kind, detail) {
  if (kind === "baseline_check") return {
    no_completion:"The replay runner returned no completed baseline",
    no_score:"The scoring rule could not grade a baseline reply",
    no_match:"The unchanged request missed the recorded outcome"
  }[detail.reason];
  if (kind === "source_gap") return "Check sources for " + detail.coverage.name;
  if (kind === "source_mask") return "Mask " + detail.sources.map(sourceName).join(" and ") + " when the recorded tool was " + detail.mask.original.label;
  if (kind === "prefill") {
    if (detail.leader.status === "zero") return "The tested edits left the first-token distribution unchanged";
    if (detail.leader.status === "tie") return "Several edits tied for the largest first-token shift";
    return detail.leader.source + " produced the largest first-token shift";
  }
  if (kind === "replacement") {
    const [left, right] = detail.conditions;
    const changed = left.edits.filter((edit, i) => edit.matched !== right.edits[i].matched);
    if (changed.length === 1 && left.joint?.matched === right.joint?.matched) {
      const source = detail.sources.find((s) => s.id === changed[0].id);
      return "Match counts for " + (source ? sourceName(source) : changed[0].id) + " differed by wording";
    }
    return "Match counts differed across replacement texts";
  }
  if (kind === "conflict") return "Conflicting values for " + detail.rule.replaceAll("_", " ");
  if (kind === "duplicate" && detail.dialogue_only) return "The same dialogue appears at " + plural(detail.copies, "message location");
  if (kind === "duplicate") return "The same input appears " + detail.copies + " times";
  if (kind === "candidate") {
    const id = sourceName(detail.sources[0]);
    if (detail.baseline * detail.edited < 0) return "Editing " + id + " switched the preferred alternative";
    return "Editing " + id + " shifted candidate scores";
  }
  if (kind === "token") return "Editing " + sourceName(detail.sources[0]) + " changed a token score";
  if (kind === "replay" && detail.baseline_answer != null && detail.edited_answer != null &&
      detail.baseline_answer !== detail.edited_answer) {
    return "Editing “" + sourceName(detail.sources[0]) + "” changed the replayed answer";
  }
  if (detail.joint_pattern && detail.joint_pattern !== "unresolved") return jointPatterns[detail.joint_pattern];
  if (detail.effect_pp === 0 && detail.response_pair?.changed) return "The proposal changed with an equal match rate";
  if (kind === "shadow" && detail.response_pair?.changed === false) return "The proposal stayed the same after editing " + sourceName(detail.sources[0]);
  if (detail.joint_only) return "Only the joint edit changed the match rate";
  const amount = Math.abs(detail.effect_pp);
  const points = amount < 0.01 ? "less than 0.01 percentage points" :
    plural(Number(amount.toFixed(2)), "percentage point");
  if (detail.effect_pp === 0) return "Measured match rates were equal";
  if (detail.edit === "control") return "Target and control differed by " + points;
  return "Match rate " + (detail.effect_pp > 0 ? "rose by " : "fell by ") + points;
}

function findingDetail(kind, detail) {
  if (kind === "baseline_check") {
    const {attempted,completed,unscored=0} = detail.measurement.baseline;
    return [detail.runner, plural(attempted,"baseline attempt"), plural(completed-unscored,"scored reply"),
      "Recorded "+detail.recorded.kind.replaceAll("_"," ")+" · "+detail.recorded.label].join(" · ");
  }
  if (kind === "source_gap") return detail.coverage.text_mismatches?.length
    ? "Captured text differs from the expected value for " + detail.coverage.text_mismatches.join(", ") + "."
    : "No configured alternative had all its source fragments anchored in the request.";
  if (kind === "source_mask") return maskOutcomes[detail.mask.outcome] + " · " + detail.mask.model;
  if (kind === "prefill") return plural(detail.ranked.length, "source edit") + " · " + detail.model_id;
  if (kind === "replacement") return detail.outcome + " · " + detail.model + " · " + plural(detail.wordings, "tested wording");
  if (kind === "conflict") return detail.rule + " · " + detail.scope + " scope";
  if (kind === "duplicate") return "Captured input text matches exactly.";
  const sources = detail.sources.map(sourceName).join(" and ");
  if (kind === "candidate") return signed(detail.change) + " change in log odds · " + detail.model;
  if (kind === "token") return "Output token " + (detail.landmark.token + 1) + " of " + detail.landmark.total_tokens + " · " + detail.landmark.model;
  if (kind === "shadow") return detail.outcome.label + " · Proposal-only test of " + sources;
  return detail.outcome + " · " + (detail.edit === "joint" ? "Joint edit of " : "Tested edit of ") + sources;
}

function tag(kind) {
  return element("span", { className: "kind " + kind, text: kindLabels[kind] });
}

function factorName(sources) {
  return sources.map((source) => {
    const revision = /^[a-f0-9]{64}$/i.test(source.version) ? source.version.slice(0, 12) : source.version;
    return sourceName(source) + (revision ? " (" + revision + ")" : "");
  }).join(" + ");
}

function replayCoverage(impact) {
  const counts = impact.replay_outcomes;
  const tested = impact.conversations - counts.unmeasured;
  return element("details", { className: "replay-coverage" },
    element("summary", { text: "Scored replays in " + tested + " of " + plural(impact.conversations, "conversation") }),
    element("p", { text: "Each conversation counts once. Tests compare how often the answer or tool call matched the recorded outcome." }),
    element("dl", {}, ...[
      ["Every scored comparison changed the match rate", counts.changed_only],
      ["Every scored comparison had equal match rates", counts.equal_only],
      ["Some comparisons changed the rate and others held steady", counts.mixed],
      ["No scored replay in these findings", counts.unmeasured],
    ].map(([label, count]) => element("div", {}, element("dt", { text: label }), element("dd", { text: count })))));
}

const comparisonLabels = {
  both: "Changed and unchanged proposals",
  changed_only: "Changed proposals only",
  held_only: "Unchanged proposals only",
  no_pair: "No comparable proposal pair",
};

function proposalCoverage(impact, factor) {
  const counts = impact.proposal_outcomes;
  const tested = impact.conversations - counts.no_pair;
  return element("details", { className:"proposal-coverage" },
    element("summary", {text:"Answers or tool calls compared in " + tested + " of " + plural(impact.conversations, "conversation")}),
    element("p", {text:"Each conversation counts once. Its comparisons can use different inputs, edits or runner settings."}),
    element("dl", {}, ...Object.entries(comparisonLabels).map(([result, label]) => element("div", {},
      element("dt", {}, counts[result] ? link(label, url({proposal_result:result, coverage_factor:factor})) : label),
      element("dd", {text:counts[result]})))),
    !factor && impact.ranked_calls ? link("Compare ranked edits across " + plural(impact.ranked_calls,"call"),url({ranking:"all"})) : null,
    impact.opening_gap && (factor || impact.ranked_reference_losses) ? link("Compare answers that changed after the same opening",
      url({opening:1, recommendation:impact.opening_gap.group, factor:impact.opening_gap.factor})) : null,
    replayCoverage(impact));
}

const outcomeLevels = [
  {key:"proposal", title:"Generated proposal changed", description:"Every listed conversation has a source edit that changed a complete replayed answer or tool proposal."},
  {key:"rate", title:"Replay match rate changed", description:"A change in the fraction of proposals meeting a scoring rule is the strongest recorded result here."},
  {key:"score", title:"Model score shifted", description:"A first-token, candidate or output-token score moved. These conversations have no verified proposal change or match-rate shift in this scope."},
  {key:"tested", title:"No change in measured model results", description:"A verified comparison had equal scores or match rates. A matched rate alone can conceal differences in the generated text."},
  {key:"inputs", title:"Captured inputs only", description:"Declared rules conflict or input text repeats. These conversations have no measured comparison among their findings in this scope."},
];
const outcomeLevel = Number(query.get("level"));
const outcomeCategory = outcomeLevels[outcomeLevel - 1];
const cohortFactor = query.get("cohort_factor");
const editedSources = outcomeLevel === 1 && query.get("by") === "edit";
const outcomeGrouping = editedSources ? "edit" : query.get("by") === "source" ? "source" : "conversation";
const cohortMode = editedSources ? {by:"edit"} : {};

function impactPanel(impact) {
  return element("section", { className: "impact", "aria-label": "Conversation impact" },
    element("div", {}, element("h2", { text: "Conversations with findings" }),
      element("p", {className:"impact-total"}, element("strong", { className: "impact-number", text: impact.conversations }),
        " ", element("span", {text:"of " + plural(impact.captured_conversations, "captured conversation")})),
      impact.without_findings ? link(plural(impact.without_findings, "conversation") + " without a finding" +
        (impact.without_findings_with_screen_suggestion ? " · " + impact.without_findings_with_screen_suggestion + " with a flagged source pair" : ""),
        url({coverage:"without-findings"}), "coverage-link") : null,
      element("p", { text: plural(impact.factors, "source factor") + (impact.unlinked_requests ? " · " + plural(impact.unlinked_requests, "additional request") + " without a conversation ID" : "") })),
    element("div", { className: "impact-breakdown" },
      element("p", {text:"Strongest model or input evidence per conversation"}),
      ...outcomeLevels.flatMap(({key, title}, index) => impact.evidence_counts[key] ? [
        element("a", {className:"impact-outcome", href:url({level:index + 1, by:index === 0 ? "edit" : "source"})},
          element("strong", {text:impact.evidence_counts[key]}), element("span", {text:title}),
          element("span", {"aria-hidden":"true",text:"→"}))] : []),
      proposalCoverage(impact)),
    impact.sandbox_tested_conversations > 0 && element("a", {className:"impact-outcome dispatch-impact", href:url({dispatches:1})},
      element("strong", {text:impact.sandbox_tested_conversations}),
      element("span", {text:"Conversations with a sandbox dispatch comparison"}),
      element("span", {"aria-hidden":"true",text:"→"})));
}

function positionLink(factor, test) {
  return url({ factor, variation: 1, pair: test.pair, group: test.group,
    baseline_trials: test.baseline_trials, edit_trials: test.edit_trials });
}

function screenedPairHref(row, sources, decision) {
  const sample = new URLSearchParams({v:1, view:"conversation", run:row.trace, call:row.call,
    screen_pair:JSON.stringify(sources.flatMap(s => [s.id,s.digest]))});
  if (decision) sample.set("decision", decision);
  return "/details?" + new URLSearchParams({run:row.trace}) + "#" + sample;
}

function pairName(sources) {
  return sources.map(s => s.source || (s.role || "Input") + " · " + s.text.slice(0, 45)).join(" and ");
}

function screenedRow(group) {
  const conflict = group.relation === "possible_conflict";
  const detail = plural(group.conversations, "flagged conversation") + " · " + group.tested + " with a completed single-source comparison";
  return rowLink(pairName(group.sources), (conflict ? "Possible conflict" : "Possible overlap") + " · " + detail,
    url({screens:1, pattern:group.pattern, relation:group.relation}));
}

function sourceOutcomes(group) {
  const rows = group.sources.map(source => {
    const counts = group.source_outcomes[source.digest];
    return element("tr", {}, element("th", {scope:"row", text:sourceName(source)}),
      element("td", {text:counts?.changed ?? "—"}), element("td", {text:counts?.held ?? "—"}));
  });
  return panel(group.relation === "possible_conflict" ? "Possible conflict" : "Possible overlap", pairExcerpts(group.sources),
    element("h3", {text:"What changed when one source was edited?"}),
    element("p", {text:group.tested + " of " + plural(group.conversations,"flagged conversation") +
      " had a completed single-source comparison. Each count includes conversations with at least one such result."}),
    element("table", {className:"screen-outcomes"}, element("thead", {}, element("tr", {},
      ...["Edited source","Proposal changed","Proposal held"].map(text => element("th",{scope:"col",text})))),
      element("tbody", {}, ...rows)),
    group.sources.some(source => !group.source_outcomes[source.digest]) ? element("p", {className:"label", text:"A dash means this source has no completed comparison."}) : null,
    element("details", {}, element("summary", {text:"Classifier"}), element("p", {text:[group.scorer.model_id, group.scorer.revision, group.scorer.runtime].join(" · ")})));
}

function pairExample(row) {
  const comparisons = row.comparisons.map(test => {
    const source = row.sources.find(s => s.id === test.context);
    const changed = test.pair.changed;
    const report = url({...screenSelection, recommendation:test.group, trace:row.trace, call:row.call, decision:test.decision,
      replay:test.replay, context:test.context}) + "#proposal-pair";
    return element("li", {}, link(sourceName(source) + (changed ? " edit changed the proposal" : " edit preserved the proposal"), report),
      element("p", {text:proposalLabel(test.pair.original, test.pair) + " → " + proposalLabel(test.pair.edited, test.pair)}),
      element("details", {}, element("summary", {text:"Edit and runner"}),
        test.replacement && element("p", {text:test.replacement.text + (test.replacement.truncated ? "…" : "")}),
        element("p", {className:"row-extra", text:[test.pair.baseline_trials + " original / " + test.pair.edited_trials + " edited", test.runner].join(" · ")})));
  });
  return panel(row.thread, element("p", {className:"label", text:[row.org,row.user,fullDate(row.started)].filter(Boolean).join(" · ")}),
    row.last_user_text ? element("p", {text:"Last user text · " + row.last_user_text.text + (row.last_user_text.truncated ? "…" : "")}) : null,
    comparisons.length ? element("ul", {className:"pair-trials"}, ...comparisons) :
      element("p", {text:"No completed single-source replay comparison for these inputs on this call."}),
    link(comparisons.length ? "Inspect the screened inputs" : "Review the pair for a replay",
      screenedPairHref(row, row.sources, row.decision)));
}

function pairExcerpts(sources) {
  return element("div", {className:"comparison"}, ...sources.map(s => element("div", {},
    element("strong", {text:sourceName(s) + version(s.version)}), element("p", {text:s.text + (s.truncated ? "…" : "")}))));
}

function captureReview(row) {
  const screen = row.screen;
  const sample = new URLSearchParams({v:1, view:"conversation", run:row.trace, call:row.call});
  const captured = "Input text captured on " + row.with_text + " of " + plural(row.calls, "call") + ".";
  const metadata = [row.org, row.user, row.latest && "Last call " + fullDate(row.latest)].filter(Boolean);
  const path = "/details?" + new URLSearchParams({run:row.trace}) + "#";
  if (!screen) return panel(row.thread, element("p", {className:"label", text:metadata.join(" · ")}), element("p", {text:captured}),
    row.with_text ? link("Review a captured exchange", path + sample) :
      link("Inspect a call with missing input text", url({view:"traces", trace:row.trace, call:row.call})));
  return screenReview(screen, element("p", {className:"label", text:[row.thread, ...metadata].join(" · ")}),
    link(screen.decision ? "Inspect the pair and its replay options" : "Inspect the source pair", screenedPairHref(row, screen.sources, screen.decision), "action"), captured);
}

function screenReview(screen, subtitle = null, action = null, captured = "") {
  const conflict = screen.suggestion === "possible_conflict";
  const score = conflict ? "contradiction" : "entailment";
  return panel(conflict ? "Possible conflict between two sources" : "Possible overlap between two sources", subtitle,
    pairExcerpts(screen.sources),
    element("p", {text:"A text classifier flagged this pair. Test each source edit against the answer or tool call to measure its effect."}),
    action,
    element("details", {}, element("summary", {text:"Classifier and coverage"}),
      element("p", {text:[screen.scorer.model_id,screen.scorer.revision,screen.scorer.runtime].join(" · ")}),
      element("p", {text:score + " score · first to second " + screen.left_to_right[score].toFixed(3) +
        " · second to first " + screen.right_to_left[score].toFixed(3)}),
      element("p", {text:[plural(screen.flagged_pairs,"flagged pair") + " among " + screen.examined_pairs + " examined of " + screen.total_pairs + " possible.", captured].filter(Boolean).join(" ")})));
}

function positionReadout(result) {
  const { present, replaced } = result;
  if (present.compared < 2 || replaced.compared < 2) return "Position test has incomplete trials";
  return "Changed proposals · source present " + present.changed + "/2 · source replaced " + replaced.changed + "/2";
}

function featuredLink(href, { title, label, description, detail, action = "Open evidence →" }, className = "featured") {
  return element("a", {className, href},
    NexusWeaveA2UI.recommendation("recommended-investigation", {title, heading:"h2", label, description, detail}),
    element("span", {className:"investigation-link", text:action}));
}

function investigation(href, title, description, detail) {
  return element("section", {"aria-label":"Recommended investigation"},
    featuredLink(href, {title, description, detail}));
}

function factorRow(factor, featured = false) {
  const signals = { baseline_check:"Replay baseline", source_gap:"Source capture gap", routing_repeats:"Repeats disagreed", routing: factor.routing_inputs > 1 ? "Routing tests on " + plural(factor.routing_inputs, "input") : "Instruction and tool test", prefill: "First-token sensitivity", variation: "Edit effects varied across inputs", order: "Results varied with source order", replacement: "Results varied across wordings", joint_proposal: "Only the joint edit changed the proposal", joint: "Only the joint edit changed the match rate", flip: "Choice flipped", replay: "Replay outcome shifted", shadow: "Proposal comparison",
    token: "Token score shifted", source_mask: "Source masks tested", sandbox_dispatch: "Sandbox state preserved",
    sandbox_regression: "Expected state missed with check", score: "Preference shifted", conflict: "Conflicting rules",
    duplicate: "Repeated input", dialogue: "Repeated dialogue", equal: "Equal match rates" };
  const step = { baseline_check:"Inspect the unchanged-request trials", source_gap:"Inspect unconfirmed source IDs", routing_repeats:"Check why the same edit gave different results", routing: factor.routing_inputs > 1 ? "See where the source edit mattered" : "Compare the four routing setups", prefill: "Review which edits shifted the first token", variation: "Compare where this edit changed the outcome", order: "Compare the two source orders", replacement: "Compare the two replacement texts", joint_proposal: "Check whether both sources mattered", joint: "Inspect the joint edit",
    flip: "Check which edit flipped the choice", replay: "Compare the original and edited outcomes",
    shadow: "Check the proposed answers or tool calls", token: "Locate the shift along the answer", source_mask: "Compare tool proposals after masking sources",
    sandbox_dispatch: "See what changed at dispatch",
    sandbox_regression: "Check where dispatch missed the expected state",
    score: "Check which alternative gained ground", conflict: "Compare the conflicting declarations",
    duplicate: "Check whether one copy can be removed", dialogue: "Check the repeated message positions",
    equal: "Review the equal match rates" };
  const test = factor.signal === "variation" && factor.position_test;
  const isolated = test && test.result.present.changed === 2 && test.result.replaced.compared === 2 && test.result.replaced.changed === 0;
  const headline = isolated ? "Moving this source changed both proposals" : "Review the source-position test";
  const name = factorName(factor.sources) || (factor.signal === "baseline_check" ? "Unchanged request" : "Configured sources");
  const counts = [factor.org, plural(factor.conversations, "identified conversation"), plural(factor.requests, "request"),
    plural(factor.recommendations, "recommendation")].filter(Boolean).join(" · ");
  const disputed = factor.disputed_routing;
  const href = disputed ? url({factor:factor.id, recommendation:disputed.group, routing_design:disputed.design, routing_input:disputed.input}) :
    test ? positionLink(factor.id, test) : url({ factor: factor.id, ...(factor.signal === "order" && { order: 1 }), ...(factor.signal === "variation" && { variation: 1 }) });
  const signal = test ? positionReadout(test.result) : signals[factor.signal];
  if (featured) {
    const completed = disputed && disputed.changed + disputed.held;
    const title = disputed ? (disputed.metric === "tool" ? "The proposed tool" : "The complete proposal") +
      " changed in " + disputed.changed + " of " + completed + " completed runs" :
      factor.order_counts ? "Swapping source order changed the match count in " + factor.order_counts.changed + " of " +
        plural(factor.order_counts.total, "comparison") : test ? headline : step[factor.signal];
    const description = disputed ? "The runs repeated " + (disputed.source ? "the edit to " + disputed.source : "one source edit") +
      " on the same input with the same instruction and tools." +
      (disputed.missing ? " " + plural(disputed.missing, "additional run") + " incomplete." : "") : null;
    return featuredLink(href, {title, label:signal, description,
      detail:[name, factor.org, plural(factor.conversations, "identified conversation")].filter(Boolean).join(" · "),
      action:disputed ? "Compare runs →" : undefined}, "factor-row featured");
  }
  return element("a", {className:"factor-row", href}, element("div", {},
    element("strong", {text:name}), element("span", {className:"row-extra", text:counts})),
    element("span", {className:["conflict", "source_gap", "duplicate", "dialogue"].includes(factor.signal) ? "basis" : "basis measured", text:signal}),
    element("span", {className:"arrow", "aria-hidden":"true", text:"→"}));
}

function cohortNavigation() {
  return element("nav", {className:"cohort-navigation", "aria-label":"Group evidence"},
    outcomeLevel === 1 ? link("By edited source", url({level:1, by:"edit"}), null, editedSources && !cohortFactor) : null,
    outcomeLevel !== 1 || outcomeGrouping === "source" ? link("By source", url({level:outcomeLevel, by:"source"}), null, outcomeGrouping === "source" && !cohortFactor) : null,
    link("By conversation", url({level:outcomeLevel}), null, outcomeGrouping === "conversation" && !cohortFactor),
    link("All source factors", url({explore:1}), "cohort-home"));
}

function cohortRow(row) {
  const subtitle = [editedSources && (row.sources.length > 1 ? "Edited together" : "Edited individually"), row.org, plural(row.requests, "request")].filter(Boolean).join(" · ");
  return element("a", {className:"cohort-row", href:url({level:outcomeLevel, ...cohortMode, cohort_factor:row.factor})},
    element("div", {}, element("strong", {text:factorName(row.sources)}), element("span", {className:"row-extra", text:subtitle})),
    element("div", {className:"cohort-reach"}, element("span", {text:row.conversations + " of " + plural(row.cohort_size, "conversation")}),
      element("meter", {min:0, max:row.cohort_size, value:row.conversations, "aria-hidden":"true"})),
    element("span", {className:"arrow", "aria-hidden":"true", text:"→"}));
}

function witnessAnchor(witness) {
  if (witness?.pair) return "#proposal-pair";
  if (witness?.arm) return "#" + new URLSearchParams({arm:witness.arm});
  if (witness?.generation) return "#" + new URLSearchParams({generation:witness.generation});
  if (witness?.setup != null) return "#routing-setup-" + witness.setup;
  if (witness?.check) return "#proposal-check=" + new URLSearchParams({report:witness.check, context:witness.context || witness.sources[0].id});
  return "";
}

function positionTrialTable(result, selection) {
  const table = element("div", {});
  const section = element("details", {}, element("summary", { text: "Compare all eight proposals" }), table);
  let loaded = false;
  section.addEventListener("toggle", async () => {
    if (!section.open || loaded) return;
    loaded = true;
    try {
      const receipt = await api("position-results", { ...selection, id: result.latest_position_result.id });
      const rows = [];
      for (const example of [result.changed_example, result.equal_example]) {
        const m = example.measurement;
        const arms = receipt.trials.filter((t) => t.location.run_id === m.location.run_id &&
          t.location.call_id === m.location.call_id && t.location.decision_id === m.location.decision_id &&
          t.replay_id === m.replay_id && t.context_id === m.context_id);
        for (const slot of [...new Set(arms.map((t) => t.position))].sort((a,b) => a-b)) {
          const outcomes = [false,true].map((replaced) => {
            const trial = arms.find((t) => t.position === slot && t.source_replaced === replaced).trial;
            if (trial.status === "failed") return "No completed proposal";
            const choice = trial.choice;
            const value = typeof choice.outcome === "string" ? choice.outcome : JSON.stringify(choice.outcome);
            return choice.kind === "model_output" ? value : choice.label + " · " + value;
          });
          rows.push(element("tr", {}, element("th", { scope: "row", text: "Originally " + example.fragment_order.position }),
            element("td", { text: slot }), ...outcomes.map((text) => element("td", {}, element("code", { text })))));
        }
      }
      table.replaceChildren(element("table", {}, element("thead", {}, element("tr", {},
        ...["Background", "Test slot", "Source present", "Source replaced"].map((text) => element("th", { scope: "col", text })))),
        element("tbody", {}, ...rows)));
    } catch (error) {
      loaded = false;
      table.textContent = error.message;
    }
  });
  return section;
}

function positionExperiment(result) {
  const m = result.changed_example.measurement;
  const selection = { ...scope, factor: result.factor, group: result.group, pair: result.position_pair,
    baseline_trials: m.baseline.attempted, edit_trials: m.variant.attempted };
  const button = element("button", { type: "button", text: "Download position test" });
  const status = element("span", { role: "status", className: "label" });
  button.addEventListener("click", async () => {
    button.disabled = true;
    status.textContent = "Preparing the requests…";
    try {
      const plan = await api("position-plan", selection);
      downloadJson("position-test-" + plan.pair.slice(0, 12) + ".json", plan);
      status.textContent = "Prepared eight conditions for your runner.";
    } catch (error) {
      status.textContent = error.message;
    } finally {
      button.disabled = false;
    }
  });
  const upload = element("input", { type: "file", accept: ".json,application/json" });
  const message = element("p", { role: "status" });
  upload.addEventListener("change", async () => {
    const file = upload.files[0];
    if (!file) return;
    upload.disabled = true;
    try {
      if (file.size > 2 * 1024 * 1024) throw new Error("Choose a result file under 2 MiB.");
      const response = await fetch("/api/position-results", { method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ selection, result: JSON.parse(await file.text()) }) });
      if (!response.ok) throw new Error((await response.json()).message || "Position results could not be saved.");
      await render();
    } catch (error) {
      message.textContent = error.message;
    } finally {
      upload.disabled = false;
      upload.value = "";
    }
  });
  const latest = result.latest_position_result;
  const resultUrl = "/api/sidecar/position-results?" + new URLSearchParams(selection);
  return element("div", {},
    latest ? element("section", { className: "position-result" }, element("h3", { text: "Measured position test" }),
      element("p", { text: "Moving the intact source changed the proposal in " + latest.present.changed +
        " of " + plural(latest.present.compared, "scored background") + ". With the source replaced, it changed in " +
        latest.replaced.changed + " of " + latest.replaced.compared + "." }),
      element("p", { className: "label", text: "Latest submitted runner · " + latest.runner_id }),
      positionTrialTable(result, selection),
      element("p", { text: latest.present.compared < 2 || latest.replaced.compared < 2 ?
        "Repeat the incomplete conditions before comparing the two backgrounds." :
        latest.replaced.changed ? "Try a second replacement to check whether its wording affected the result." :
        "Repeat the position test on another request before changing the workflow." }),
      result.position_reports > 1 ? element("p", {}, link("Browse submitted reports (" + result.position_reports + ")", resultUrl)) : null) : null,
    element("div", { className: "actions" }, button, status),
    element("details", {}, element("summary", { text: "Attach completed trials" }),
      element("label", { text: "Results from run-position-plan" }, upload), message));

}

function variationComparison(result) {
  const first = result.changed_example.fragment_order, second = result.equal_example.fragment_order;
  const permutation = result.matched_fragment_order;
  const source = result.changed_example.sources[0];
  function example(label, detail) {
    const m = detail.measurement;
    const target = { factor: result.factor, recommendation: result.group, trace: m.location.run_id,
      call: m.location.call_id, replay: m.replay_id, context: m.context_id };
    return element("section", { className: "variation-example" },
      element("h3", { text: label }),
      m.conversation_id ? element("p", { className: "label", text: m.conversation_id }) : null,
      detail.fragment_order ? element("p", { text: "Position " + detail.fragment_order.position + " of " +
        detail.fragment_order.total + " named sources in this field", title: detail.fragment_order.pointer }) : null,
      replayTrials(detail), link("Inspect this replay", url(target), "action"));
  }
  return element("section", { className: "panel variation" },
    element("h2", { text: sourceName(source) }),
    source.version ? element("p", { className: "label", title: source.version, text: "Source version " +
      (/^[a-f0-9]{64}$/i.test(source.version) ? source.version.slice(0, 12) : source.version) }) : null,
    element("p", { text: "The match rate changed on " + plural(result.changed_inputs, "distinct input") +
      " and held steady on " + plural(result.equal_inputs, "other input") + "." }),
    result.mixed_inputs ? element("p", { text: "Repeated tests gave both results on " +
      plural(result.mixed_inputs, "additional input") + "." }) : null,
    permutation ? element("p", { className: "order-evidence", text: "The two captured requests differ only in the order of named fragments. The edited source moved from position " +
      first.position + " to " + second.position + "." }) : null,
    element("div", { className: "variation-examples" },
      example("Match rate changed", result.changed_example), example("Match rate held steady", result.equal_example)),
    !result.latest_position_result ? element("h3", { text: "What to test next" }) : null,
    !result.latest_position_result ? element("p", { text: permutation ? "Move this source alone and keep the remaining sources in their relative order. Replay both conditions." : "Keep the edit fixed and vary one surrounding input at a time. Compare the answers under the same decoding settings." }) : null,
    result.position_pair ? positionExperiment(result) : null);
}

function prefillSummary(counts) {
  const inputs = counts.reduce((total, row) => total + row.inputs, 0);
  const descriptions = { zero: "All tested shifts were zero", tie: "Several edits tied for first", mixed: "Repeat rankings disagreed" };
  return counts.map(({leader, inputs: n}) =>
    (leader.status === "unique" ? leader.source + " ranked first" : descriptions[leader.status]) +
    " on " + n + " of " + plural(inputs, "distinct input")).join(". ") + ".";
}

function routingCoverage(card) {
  const tested = (card.routing_profiles || []).reduce((n, row) => n + row.inputs, 0);
  const eligible = card.prefill_counts.reduce((n, row) => n + row.inputs, 0);
  return tested + " of " + plural(eligible, "distinct input") + " tested with this design · " +
    plural(card.routing_conversations, "identified conversation") + (card.routing_design_count > 1 ? " · " + card.routing_design_count + " designs in scope" : "");
}

function routingPatterns(card) {
  const detail = card.evidence.detail, test = detail.routing_test;
  const edit = detail.ranked.find(row => row.context_id === test.context_id);
  const toolSets = test.inventories.map(names => names.length ? names.join(" + ") : "no tools");
  const inventory = i => toolSets[0] === toolSets[1] ? (i ? "alternate" : "original") + " tool definitions" : toolSets[i];
  const setups = ["Original instruction with " + inventory(0), "Original instruction with " + inventory(1),
    "Alternate instruction with " + inventory(0), "Alternate instruction with " + inventory(1)];
  function description(signature) {
    const common = {"0000":"No setup", "1111":"Every setup", "0101":"With " + inventory(1), "1010":"With " + inventory(0),
      "0011":"With the alternate instruction", "1100":"With the original instruction", mixed:"Repeat trials disagreed"};
    if (common[signature]) return common[signature];
    return Array.from(signature, (ch, i) => ch === "1" ? setups[i] : ch === "?" ? setups[i] + " (missing)" : null)
      .filter(Boolean).join(" · ");
  }
  function evidence(profile) {
    const location = example => url({factor:factorId || detail.factor, recommendation:card.id, ...example});
    if (!profile.repeat_evidence) return link("Inspect example", location(profile.example));
    return element("div", {}, ...Object.entries(profile.repeat_evidence).sort().map(([metric, pair]) =>
      element("details", {}, element("summary", {text:metric === "tool" ? "Compare tool selection" : "Compare full proposals"}),
        element("p", {text:setups[pair.setup]}),
        element("p", {text:pair.changed_runs + " changed · " + pair.held_runs + " unchanged · " + pair.missing_runs + " incomplete"}),
        element("div", {className:"actions"}, link("Unchanged in this run", location(pair.held) + "#routing-setup-" + pair.setup),
          link("Changed in this run", location(pair.changed) + "#routing-setup-" + pair.setup)))));
  }
  function runCounts(profile, expanded = false) {
    if (!profile.repeat_counts) return null;
    const total = profile.example_runs;
    const rows = profile.repeat_counts.map((counts, i) => {
      const completed = total - counts.missing;
      return element("tr", {}, element("th", {scope:"row", text:setups[i]}),
        ...[counts.proposal, counts.tool].map(changed => element("td", {},
          element("span", {text:completed ? changed + " / " + completed : "No pairs"}),
          completed ? element("meter", {min:0, max:completed, value:changed, "aria-hidden":"true"}) : null)),
        element("td", {text:counts.missing}));
    });
    const table = element("div", {className:"routing-table repeat-counts", role:"region", "aria-label":"Repeat counts by setup", tabindex:"0"},
        element("table", {}, element("thead", {}, element("tr", {}, ...["Setup", "Proposal changed", "Tool changed", "Missing"]
          .map(text => element("th", {scope:"col", text})))), element("tbody", {}, ...rows)));
    return expanded ? table : element("details", {}, element("summary", {text:"Compare " + plural(total, "run") + " on this input"}), table);
  }
  const sourceEdit = element("p", {text:"Tested source · " + (edit.source || edit.context_id) + " · " + edit.before + " → " +
    (edit.operation === "remove_json" ? "(removed)" : edit.replacement)});
  const first = card.routing_profiles[0];
  if (card.routing_profiles.length === 1 && first.inputs === 1 && first.repeat_counts) {
    return element("section", {className:"panel routing-comparison"}, element("h2", {text:"How often did the edit change the result?"}),
      sourceEdit, element("p", {text:plural(first.example_runs, "captured run") + " on the same input. Counts show how many completed pairs changed under each setup."}),
      runCounts(first, true), evidence(first), routingInstructions(test));
  }
  const rows = card.routing_profiles.map(profile => element("tr", {},
    element("th", {scope:"row", text:profile.inputs}), element("td", {text:description(profile.signature)}),
    element("td", {text:description(profile.tool_signature)}), element("td", {}, evidence(profile), runCounts(profile))));
  return element("section", {className:"panel routing-comparison"}, element("h2", {text:"Where the source edit mattered"}),
    sourceEdit,
    element("p", {text:"Each row counts inputs with the same result. A change in wording or arguments can leave tool selection unchanged."}),
    element("div", {className:"routing-table", tabindex:"0", role:"region", "aria-label":"Routing results across inputs"},
      element("table", {}, element("thead", {}, element("tr", {}, ...["Inputs", "Complete proposal changed", "Proposed tool changed", "Evidence"]
        .map(text => element("th", {scope:"col", text})))), element("tbody", {}, ...rows))), routingInstructions(test));
}

function repeatedRouting(card) {
  const profiles = card.routing_profiles;
  if (!profiles?.length) return false;
  return profiles.length > 1 || profiles[0].example_runs > 1 || profiles[0].inputs > 1 || profiles[0].signature === "mixed" ||
    profiles[0].tool_signature === "mixed" || card.routing_design_count > 1 ||
    card.prefill_counts.reduce((n, row) => n + row.inputs, 0) > profiles[0].inputs;
}

function prefillFollowup(card) {
  const eligible = card.prefill_counts.filter((row) => row.leader.status === "unique").reduce((n, row) => n + row.inputs, 0);
  if (!eligible) return element("p", { text: "A consistent, uniquely ranked edit is needed to compare follow-up proposals." });
  if (!card.prefill_followup.length) return element("p", { text: "No paired proposals for the leading edit in this scope." });
  return element("div", {}, ...card.prefill_followup.map((check) => {
    const scored = check.changed + check.equal + check.mixed;
    return element("p", { title: check.runner, text: (card.evidence.detail.routing_test ? "Leading edit in the original setup · " : "Leading edit · ") + check.changed + " changed, " + check.equal + " unchanged" +
      (check.mixed ? ", " + check.mixed + " mixed" : "") + " across " + plural(scored, "replayed input") +
      (eligible > scored ? " · " + (eligible - scored) + " without a paired result on this runner" : "") +
      (card.prefill_followup.length > 1 ? " · " + check.runner : "") });
  }));
}

function findingCard(card) {
  const { kind, evidence: { detail } } = card;
  return element("article", { className: "card" },
    element("div", {}, tag(kind), element("h2", { text: kind === "prefill" ? prefillSummary(card.prefill_counts) : findingTitle(kind, detail) }),
      detail.sandbox_dispatch?.outcome === "matched_without_check" ?
        element("p", {text:"The expected state was reached only when the proposal ran without the dispatch check."}) :
      detail.sandbox_dispatch?.protected ? element("p", {text:"The same proposal changed state without the dispatch check. The check preserved the expected state."}) :
        repeatedRouting(card) ? element("p", {text:routingCoverage(card)}) : kind === "prefill" ? prefillFollowup(card) :
        element("p", { text:kind === "source_mask" ? maskOutcomeSummary(card.mask_outcomes) : findingDetail(kind, detail) }),
      element("p", { className: "label", text: plural(card.occurrences, "observation") + " · " + plural(card.traces, "request") }),
      link("Review recommendation", url({ factor: factorId, recommendation: card.id }), "action"),
      card.kind === "conflict" && card.threads > 1 && factorId ? link("Review test coverage across conversations",url({recommendation:card.id,factor:factorId,conflict_coverage:"all"})) : null),
    element("div", { className: "suggestion" }, element("span", { className: "label", text: "Next experiment" }),
      element("p", { text: card.action.title })));
}

function panel(title, ...children) {
  return element("section", { className: "panel" }, element("h2", { text: title }), ...children);
}

function listing(items, render) {
  return element("ul", { className: "list" }, ...items.map((item) => element("li", {}, render(item))));
}

function rowLink(title, subtitle, href, current = false) {
  return element("a", { href, ...(current && { "aria-current": "true" }) },
    element("span", { className: "row-main", text: title }),
    subtitle ? element("span", { className: "row-extra", text: subtitle }) : null);
}

function pagination(next, params = {}, offset = pageOffset) {
  byId("pagination").replaceChildren(...[
    offset > 0 ? link("Previous", url({ view, ...params, offset: Math.max(0, offset - 15) || null })) : null,
    next == null ? null : link("Next", url({ view, ...params, offset: next })),
  ].filter(Boolean));
}

function traceRows(rows) {
  return listing(rows, (item) => rowLink(item.workflow || item.source, [item.id, fullDate(item.start),
    item.thread ? "Thread " + item.thread : "", item.with_inputs + "/" + item.calls + " inference spans with inputs"].filter(Boolean).join(" · "),
  url({ view: "traces", trace: item.id })));
}

function callFinding(row, finding) {
  const sources = [finding.source, finding.kind === "conflict" && finding.other_source].filter(Boolean).join(" and ");
  const titles = {
    0:"The choice changed at token " + finding.token_position,
    1:"A lower-ranked edit changed the proposal",
    2:"A source edit changed the proposal",
    3:kindLabels[finding.kind], 4:kindLabels[finding.kind], 5:"Review the recorded test",
    6:"Declared instructions disagree", 7:kindLabels[finding.kind]
  };
  const params = {recommendation:finding.group, trace:row.trace, call:row.call, report:finding.report,
    replay:finding.replay, decision:finding.decision, context:finding.context, thread:threadId};
  if (finding.priority <= 1) Object.assign(params,{ranking:"all",review:"lower_changed"});
  return link(titles[finding.priority] + (sources ? " · " + sources : ""),url(params),"call-finding");
}

function dialogueCall(row, showUser, focus) {
  const d = row.dialogue;
  const messages = [
    showUser && element("div", {className:"dialogue-user"},element("strong",{text:"Latest user text"}),
      element("p",{text:d.user?.text ?? (d.user?.availability === "redacted" ? "Input was redacted." : "User text unavailable.")})),
    ...d.outputs.map(output => element("div",{className:"dialogue-output"},
      element("strong",{text:output.kind === "tool_call" ? "Proposed tool · " + output.label : "Assistant"}),
      element("p",{text:output.text ?? "Output content unavailable."})))
  ];
  if (!d.output_count) messages.push(element("p",{className:"label",text:"No answer or tool proposal captured."}));
  if (d.output_count > d.outputs.length) messages.push(element("p",{className:"label",text:"Showing " + d.outputs.length + " of " + plural(d.output_count,"captured output") + "."}));
  const [first,...others] = row.findings;
  return element("section",{className:"dialogue-step",...(focus && {"aria-current":"step",id:"selected-call",tabindex:"-1"})},
    element("div",{className:"dialogue-messages"},...messages),
    first ? element("div",{className:"dialogue-evidence"},callFinding(row,first),
      others.length ? element("details",{},element("summary",{text:"Other findings on this call"}),...others.map(f=>callFinding(row,f))) : null) :
      element("p",{className:"label",text:"No finding recorded for this call."}),
    element("footer",{},element("span",{text:[d.model, fullDate(row.started), d.included+" of "+plural(d.inputs,"context item")+" captured"].filter(Boolean).join(" · ")}),
      link("Open call",url({view:"traces",trace:row.trace,call:row.call}))));
}

async function conversationView() {
  const focusTrace = query.get("focus_trace"), focusCall = query.get("focus_call");
  const focus = focusTrace && focusCall ? {focus_trace:focusTrace, focus_call:focusCall} : {};
  const page = await api("thread-calls",{...scope,thread:threadId,offset:pageOffset,...focus});
  byId("title").textContent = page.rows[0]?.thread || "Conversation";
  byId("summary").textContent = "Captured calls in time order. Select a finding to see the source and its evidence.";
  pagination(page.next_offset,{thread:threadId},page.offset);
  const groups = [];
  page.rows.forEach((row,i) => {
    const turn = row.dialogue.turn;
    let group = groups.at(-1);
    if (!turn || !group || group.turn !== turn) {
      group = {turn, rows:[], label:turn ? "Turn " + turn : "Call " + (page.offset+i+1)};
      groups.push(group);
    }
    group.rows.push(row);
  });
  return element("div",{className:"conversation-feed"},
    ...groups.map(group => {
      const firstUser = group.rows[0].dialogue.user;
      const question = group.turn && firstUser?.text && !firstUser.truncated && firstUser.text.length <= 150 ? firstUser.text : null;
      return element("article", {className:"dialogue-call"},
        question && element("p", {className:"label", text:"User turn · " + plural(group.rows.length,"call") + " on this page"}),
        element("h2", {text:question || group.label}),
        ...group.rows.map((row,i) => {
          const user = row.dialogue.user;
          const repeated = i && user?.text && user.truncated === false && user.text === group.rows[i-1].dialogue.user?.text;
          return dialogueCall(row, !(i === 0 && question) && !repeated, row.trace === focusTrace && row.call === focusCall);
        }));
    }),
    !page.rows.length && element("p",{className:"empty",text:"No matching captured call in the selected filters."}),
    link("Browse traces in this conversation",url({view:"traces",thread:threadId})));
}

function advancedHref(trace, call, item) {
  const m = item?.detail.measurement;
  const hash = new URLSearchParams({ v: "1", view: "decisions", run: trace.id, call: call.id });
  const decision = m?.location?.decision_id || call.decisions?.[0]?.id;
  if (decision) hash.set("decision", decision);
  if (m) {
    hash.set("replay", m.replay_id);
    if (item.kind === "baseline_check") hash.set("baseline","1");
    if (m.context_id) hash.set("variant", m.context_id);
    if (m.control_context_id) hash.set("control", m.control_context_id);
  }
  return "/details?" + new URLSearchParams({ run: trace.id }) + "#" + hash;
}

function prepareEdit(trace, call, item) {
  const source = item.detail.sources?.[0];
  const decision = call.decisions[0];
  if (!source?.id || !decision || !call.context.some((input) => input.id === source.id && input.request_fragment)) return null;
  const hash = new URLSearchParams({ v: "1", view: "decisions", run: trace.id, call: call.id, decision: decision.id, context: source.id });
  return link("Prepare an edit", "/details?" + new URLSearchParams({ run: trace.id }) + "#" + hash, "action");
}

function compareForm(group) {
  const result = element("div", { "aria-live": "polite" });
  const user = element("input", { name: "compare_user", placeholder: "User ID", required: "" });
  const form = element("form", {}, element("label", { text: "Compare with a user in the same filters" }, user),
    element("button", { type: "submit", text: "Compare" }));
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const otherUser = user.value.trim();
    if (!otherUser) {
      result.textContent = "Enter a user ID.";
      return;
    }
    result.textContent = "Loading comparison…";
    try {
      const filters = { ...scope, group, ...(factorId && { factor: factorId }) };
      const [current, other] = await Promise.all([api("findings", filters),
        api("findings", { ...filters, user: otherUser })]);
      const describe = (rows) => rows.length ? plural(rows[0].traces, "trace") + " · " + plural(rows[0].threads, "identified thread") : "No captured matches";
      result.replaceChildren(element("div", { className: "comparison" }, ...[
        [scope.user || "Current scope", current.rows], [otherUser, other.rows],
      ].map(([label, rows]) => element("div", {}, element("strong", { text: label }),
        element("p", { text: describe(rows) }), rows.length ? trend(rows[0].trend) : null))));
    } catch (error) {
      result.textContent = error.message;
    }
  });
  return element("details", { className: "panel" }, element("summary", { text: "Compare recurrence" }), form, result);
}

function recommendationContext(kind, detail) {
  if (kind === "candidate") {
    const other = detail.other_edits.map((edit) => element("li", { text: sourceName(edit) + " · " +
      signed(edit.change) + " change in log odds" }));
    return element("div", {}, element("p", { text: detail.prefix ? "Both alternatives were scored after this fixed prefix." :
      "Both alternatives were scored at the start of the answer." }),
      detail.prefix ? element("pre", { text: detail.prefix }) : null,
      other.length ? element("h3", { text: "Other edits in this test" }) : null,
      other.length ? element("ul", {}, ...other) : null);
  }
  if (kind === "token") {
    const point = detail.landmark;
    const selected = point.positions.find((position) => position.token === point.token);
    return element("div", {},
      element("p", { text: "For the original token " + JSON.stringify(selected.fragment) + ", the probability changed from " +
        tokenProbability(selected.baseline) + " to " + tokenProbability(selected.edited) +
        " after editing the source. Both scores use the original output prefix." }),
      element("p", { text: "At this token, the largest tested decoder-block swap was at block " +
        point.block + " (" + signed(point.block_delta) + " log probability)." }));
  }
  if (kind === "replay" && detail.edit === "joint" && detail.single_edits?.length) {
    return element("div", {},
      detail.single_edits.some(a => a.proposal?.truncated) || detail.response_pair?.original.truncated || detail.response_pair?.edited.truncated ?
        element("p", {className:"label", text:"Previews show the first 160 characters. The comparison uses the complete proposals."}) : null,
      detail.joint_pattern === "unresolved" ? element("p", {text:"At least one condition has missing or different proposals across trials. The match counts are still available."}) : null,
      element("p", { text: "Joint interaction: " + (detail.measurement.interaction_pp == null ? "unavailable" :
        signed(detail.measurement.interaction_pp) + " percentage points beyond the sum of the two individual changes.") }),
      element("details", {}, element("summary", { text: "Tested replacements" }),
        ...detail.single_edits.map((arm) => element("p", { text: arm.id + " · " + (arm.replacement || "(Removed text)") }))));
  }
  if (kind === "replacement") {
    return element("div", {}, detail.wordings > 2 ? element("p", { text: "Showing two of " + detail.wordings + " tested wordings. " +
      plural(detail.wordings - detail.consistent_wordings, "wording") + " had inconsistent counts across repeats." }) : null,
      element("details", {}, element("summary", { text: "Scoring rule and runner" }),
        element("p", { text: detail.runner }), element("pre", { text: JSON.stringify(detail.match_rule, null, 2) })));
  }
  if (kind === "shadow") {
    return element("details", {}, element("summary", { text: "Show expected proposal" }),
      element("pre", { text: JSON.stringify(detail.outcome, null, 2) }));
  }
  return null;
}

function proposalLabel(proposal, pair) {
  return proposal.kind === "tool_call" || pair.original.label !== pair.edited.label || pair.original.kind !== pair.edited.kind
    ? [proposal.kind, proposal.label, proposal.outcome_preview].join(" · ") : proposal.outcome_preview;
}

function responsePair(pair) {
  if (!pair) return null;
  return element("details", {className:"response-pair"}, element("summary", {text:pair.changed ? "Compare the changed proposals" : "Compare the matching proposals"}),
    element("div", {className:"comparison"}, ...[[pair.original,"Original"],[pair.edited,"Edited"]].map(([p, name]) =>
      element("div", {}, element("strong", {text:name}), element("code", {text:proposalLabel(p, pair)})))),
    (pair.original.truncated || pair.edited.truncated) ? element("p", {className:"label", text:"Previews show the first 160 characters. The comparison uses the complete proposals."}) : null,
    element("p", {className:"label", text:pair.baseline_trials + " original and " + pair.edited_trials +
      " edited trials. All trials completed and agreed within each setup."}));
}

const dispatchOutcomes = {
  matched_with_check:"Only with the check",
  matched_without_check:"Only without the check",
  matched_both:"Both runs matched",
  missed_both:"Neither run matched",
  mixed:"Mixed results in one conversation",
  unmeasured:"No comparison with these settings",
};

function dispatchOutcomeLinks(counts, selection) {
  return listing(Object.entries(dispatchOutcomes).filter(([name]) => counts[name] > 0), ([name,label]) =>
    rowLink(label, plural(counts[name],"conversation"),url({...selection,dispatch_outcome:name})));
}

async function dispatchDesigns() {
  const data = await api("dispatch-designs", {...scope,offset:pageOffset});
  byId("title").textContent = "Sandbox dispatch comparisons";
  byId("summary").textContent = "See which runs reached the expected state. Each pair starts from the same state and runs the same proposal with and without the check.";
  pagination(data.next_offset,{dispatches:1});
  return element("div", {className:"feed"}, link("Back to recommendations",url({})),
    data.rows.length ? element("p", {className:"row-extra",text:"A conversation counts once per setup and may appear under more than one setup."}) :
      element("p", {className:"empty",text:"No sandbox dispatch comparisons in this scope."}),
    ...data.rows.map(row => panel(factorName(row.setup.sources),
      element("p", {text:plural(row.tested,"conversation") + " tested"}),
      dispatchOutcomeLinks(row.outcomes, {dispatches:1,recommendation:row.group,factor:row.factor,dispatch_design:row.design}),
      element("details", {}, element("summary", {text:"Test settings"}),
        element("p", {className:"row-extra",text:[row.setup.guard,row.setup.runner,row.setup.tool,"Target " + row.setup.target].join(" · ")})))));
}

function dispatchCoverage(card, rows) {
  const dispatch = card.evidence.detail.sandbox_dispatch;
  if (!dispatch || !rows?.length) return null;
  const total = rows.reduce((sum,row) => sum + row.conversations,0);
  const counts = Object.fromEntries(rows.map(row => [row.outcome,row.conversations]));
  return element("section", {className:"panel dispatch-coverage","aria-label":"Dispatch outcomes across conversations"},
    element("h2", {text:"Which runs reached the expected state?"}),
    element("p", {text:plural(total,"identified conversation") + " · " + dispatch.guard + " · " + dispatch.runner}),
    dispatchOutcomeLinks(counts, {recommendation:card.id,factor:card.evidence.detail.factor,dispatch_design:dispatch.design}));
}

async function dispatchConversations() {
  const selection = {recommendation:recommendationId,factor:factorId,...dispatchSelection};
  const outcome = dispatchSelection.dispatch_outcome;
  const data = await api("dispatch-conversations",{...scope,group:recommendationId,factor:factorId,
    design:dispatchSelection.dispatch_design,outcome,offset:pageOffset});
  byId("title").textContent = dispatchOutcomes[outcome];
  byId("summary").textContent = "Open a reported comparison to see the proposal and state on that call.";
  pagination(data.next_offset,selection);
  const back = dispatchOverview ? link("Back to dispatch comparisons",url({dispatches:1})) :
    link("Back to recommendation",url({recommendation:recommendationId,factor:factorId}));
  return element("div",{className:"feed"},back,
    ...data.rows.map(row => panel(row.thread,element("p",{className:"label",text:fullDate(row.observed)}),
      element("div",{className:"actions"},...row.examples.map(example => link(
        example.outcome ? dispatchOutcomes[example.outcome] : "Open proposal comparison",
        url({...selection,trace:example.trace,call:example.call,report:example.report})))))),
    !data.rows.length && element("p",{className:"empty",text:"No conversations in this scope."}));
}

function sandboxDispatch(comparison) {
  if (!comparison) return null;
  const value = x => typeof x === "string" ? x.replaceAll("_"," ") : JSON.stringify(x);
  const stateRows = comparison.states.map(row => element("tr", {className:row.key === comparison.target ? "dispatch-target" : ""},
    element("th", {scope:"row",text:row.key}), ...[row.before,row.without_guard,row.with_guard].map((state,i) =>
      element("td", {className:i === 1 && comparison.protected && row.key === comparison.target ? "dispatch-change" : "",
        text:value(state)}))));
  return element("section", {className:"panel dispatch-comparison", "aria-label":"Sandbox dispatch comparison"},
    element("h2", {text:comparison.protected ? "The dispatch check preserved the expected state" : "Compare sandbox state after dispatch"}),
    element("p", {text:"Both runs used the same tool proposal and starting checkpoint."}),
    comparisonTable(["State","Before dispatch","Without check","With check"],stateRows),
    comparison.outcome && element("p", {className:"row-extra",text:"Expected for " + comparison.target + " · " + value(comparison.expected) +
      ". " + dispatchOutcomes[comparison.outcome] + (comparison.outcome.startsWith("matched_") &&
        comparison.outcome !== "matched_both" ? " matched." : ".")}),
    element("p", {className:"row-extra",text:"Without check · " + comparison.without_guard + " / With check · " + comparison.with_guard}),
    element("details", {}, element("summary", {text:"Replay details"}),
      element("p", {text:"Runner " + comparison.runner + " · Check " + comparison.guard}),
      element("p", {text:"Checkpoint " + comparison.checkpoint})));
}

const maskOutcomes = {
  changed_tool:"Tool changed",
  changed_arguments:"Arguments changed",
  same_proposal:"Proposal held",
  token_limit:"Token limit reached",
  unparsed_proposal:"Proposal unavailable",
  mixed:"Runs disagreed"
};

function maskOutcomeSummary(counts) {
  return Object.entries(maskOutcomes).filter(([key]) => counts[key])
    .map(([key,label]) => label + " · " + counts[key]).join("  /  ");
}

function maskComparison(mask, showOutcome = true) {
  const rows = [[mask.original,"All sources visible"],[mask.proposal,"Source mask applied"]];
  return element("div", {className:"mask-comparison"},
    showOutcome && element("h3", {text:maskOutcomes[mask.outcome]}),
    element("div", {className:"comparison"}, ...rows.map(([proposal,label]) => element("div", {},
      element("strong", {text:label}), proposal ?
        element("code", {text:proposal.label + " " + proposal.outcome_preview}) :
        element("p", {text:mask.outcome === "token_limit" ? "Stopped at the " + mask.budget + "-token limit." : "The runner could not parse a complete proposal."})))),
    element("p", {className:"row-extra", text:"Resumed at token " + (mask.start + 1) + (mask.target ? " · " + mask.target : "") +
      (mask.log_probability_change == null ? "" : " · " + signed(mask.log_probability_change) + " change in original log probability")}),
    link("Inspect this model replay", "/details?" + new URLSearchParams({run:mask.trace}) +
      mask.inspector_path.slice(mask.inspector_path.indexOf("#"))));
}

function recommendationDetail(kind, detail, action, rows = [], hasConflictTrial = false) {
  const interval = detail.measurement?.interval || detail.measurement?.difference_95_interval_pp;
  const uncertainty = interval ? "95% interval for the difference: " + interval.lower + " to " + interval.upper + " percentage points." :
    ({ replacement: "These counts come from one request. Sampling variation can also change the observed counts.",
       replay: "The supplied trials have no estimated interval for this comparison.",
       shadow: "The supplied proposal trials have no estimated interval for this comparison.",
       candidate: "These scores compare two continuations. Free generation can choose a third answer.",
       token: "This comparison holds the output prefix fixed at each position.",
       baseline_check: "The effect of a source edit on the recorded outcome is unavailable for this report.",
       source_gap: "Missing anchors can reflect incomplete capture even when the text was present.",
       conflict: hasConflictTrial ? "The declarations establish the disagreement. Each replay below measures one edit on this call." : "The disagreement is visible in the inputs. Its effect on the answer still needs a test.",
       duplicate: "The text matches exactly. Its effect on the answer still needs a test." })[kind];
  return panel(findingTitle(kind, detail), tag(kind),
    kind === "replacement" ? element("p", { className: "label", text: detail.model + " · " + plural(detail.conditions[0].baseline.attempted, "trial") + " per condition" }) : element("p", { className: "label", text: action.basis }), visual(kind, detail, rows),
    detail.joint_pattern ? null : responsePair(detail.response_pair), recommendationContext(kind, detail), element("p", { className: "uncertainty", text: uncertainty }),
    element("details", {}, element("summary", { text: "How this was measured" }),
      element("p", { text: ({
        replacement: "The sidecar compared two replay reports with the same recorded decision, runner, request, edit locations, scoring rule, sampling design and baseline counts. The reports used different replacement texts.",
        candidate: "A model runner scored two fixed continuations from the same prefix. This can run before inference or in a later experiment. The sidecar verifies the request and edit. The runner supplies the timing.",
        shadow: "A model runner proposed answers or tool calls for the baseline and edited requests. The sidecar checked the request and recomputed the match rates.",
        token: "An instrumented model runner replaced a source and swapped decoder-block activations on a fixed output path. This view shows where the original token’s score moved under those interventions.",
        replay: "A runner replayed the original request and the specified edits. The sidecar checked the request digests and counted the returned answers or tool calls that matched the recorded outcome.",
        baseline_check: "The sidecar verified the replay report and recomputed baseline matches using its declared match rule.",
        source_gap: "The sidecar verified the preflight receipt against the recorded request, source anchors and configured alternatives, then checked each alternative for complete coverage.",
        conflict: "The sidecar compared the rule key, scope and declared value on captured inputs. This check can run as soon as the workflow assembles a request.",
        duplicate: "The sidecar found the same text at separate input locations. This check can run before the workflow sends the request."
      })[kind] })),
    element("h3", { text: action.title }),
    element("p", { text: action.next }),
    detail.replacement == null ? null : element("details", {}, element("summary", { text: "Show tested replacement" }),
      element("pre", { text: detail.replacement || "(Removed text)" })));
}

function replacementLinks(detail, traceId, callId) {
  const [left, right] = detail.conditions;
  const changed = left.edits.find((edit, i) => edit.matched !== right.edits[i].matched);
  return detail.conditions.map((condition, i) => {
    const measurement = { location: { decision_id: detail.decision_id }, replay_id: condition.report_id, context_id: changed?.id };
    return link("Open wording " + (i === 0 ? "A" : "B") + " trials",
      advancedHref({ id: traceId }, { id: callId }, { detail: { measurement } }));
  });
}

function jointRecurrence(card) {
  const rows = card.joint_patterns || [];
  if (!rows.length || card.occurrences < 2) return null;
  const samples = card.evidence.detail.measurement.baseline.attempted;
  const result = element("ul", {className:"joint-recurrence"});
  for (const row of rows) {
    result.append(element("li", {}, link(jointPatterns[row.pattern],
      url({factor:card.evidence.detail.factor, recommendation:card.id, joint_pattern:row.pattern, baseline_trials:samples})),
      element("span", {className:"label", text:plural(row.conversations, "identified conversation") + " · " + plural(row.inputs, "distinct captured input") +
        " · " + plural(row.observations, "report") + (row.missing_inputs ? " · input unavailable for " + plural(row.missing_inputs, "report") : "")})));
  }
  return panel("Where did this pattern recur?", result,
    element("p", {text:"These reports used the same source edits, runner and scoring rule, with " + plural(samples, "trial") + " per condition. A conversation can appear in more than one row."}));
}

const coverageNames = {untested:"No comparable replay", partial:"Partially tested", complete:"Both sources compared on every call"};

async function conflictExamplesView() {
  const selection = {recommendation:recommendationId, factor:factorId, conflict_design:conflictDesign};
  const data = await api("conflict-examples", {...scope, group:recommendationId, factor:factorId, design:conflictDesign, offset:pageOffset});
  byId("title").textContent = "Which source edit changed the proposal?";
  byId("summary").textContent = "Only the final user message varies across these requests. Each comparison edits one source at a time.";
  pagination(data.next_offset, selection);
  return element("div", {className:"feed"}, link("Back to recommendations", url({})),
    data.rows.length && element("details", {className:"panel"}, element("summary", {text:"Test conditions"}),
      element("p", {text:data.rows[0].runner}),
      ...data.rows[0].arms.map(arm => element("p", {}, element("strong", {text:sourceName(arm.source) + " replacement · "}),
        arm.replacement + (Array.from(arm.replacement).length === 160 ? " [first 160 characters shown]" : "")))),
    ...data.rows.map(row => {
      const pair = row.arms[0].pair;
      const arms = row.arms.map(arm => {
        const result = arm.pair;
        const target = url({recommendation:arm.group, trace:row.trace, call:row.call, decision:row.decision,
          replay:row.replay, context:arm.source.id}) + "#proposal-pair";
        return element("div", {className:result.changed ? "changed-edit" : "held-edit"}, element("strong", {text:"Edit " + sourceName(arm.source)}),
          element("code", {text:proposalLabel(result.edited, result)}),
          link(result.changed ? "Changed · inspect trials" : "Held · inspect trials", target));
      });
      return panel(row.thread, element("blockquote", {text:row.question || "Question unavailable"}),
        element("p", {}, "Original proposal ", element("code", {text:proposalLabel(pair.original, pair)})),
        element("div", {className:"comparison question-edits"}, ...arms),
        element("p", {className:"row-extra", text:plural(pair.baseline_trials,"original trial") +
          " · " + row.arms.map(arm => plural(arm.pair.edited_trials,"edited trial") + " for " + sourceName(arm.source)).join(", ")}),
        row.pattern === "mixed" && element("p", {text:"Repeated reports on this request disagreed. One report is shown."}));
    }),
    !data.rows.length && element("p", {className:"empty", text:"No comparable questions in this scope."}));
}

function conflictPatternCard(pattern) {
  const [left, right] = pattern.sources.map(sourceName);
  return investigation(url({recommendation:pattern.group, factor:pattern.factor, conflict_design:pattern.design}),
    "Compare questions where different source edits changed the proposal",
    "Only the " + left + " edit changed the proposal on " + plural(pattern.left_count,"request") +
    ". Only the " + right + " edit changed it on " + plural(pattern.right_count,"request") + ". " +
    plural(pattern.inputs,"distinct request") + " tested.");
}

async function conflictCoverageView() {
  const filters = {...scope,group:recommendationId,factor:factorId};
  const selection = {recommendation:recommendationId,factor:factorId};
  const summary = (await api("conflict-coverage",filters)).rows[0];
  byId("title").textContent = "Conflict test coverage";
  if (!summary) return element("p",{className:"empty",text:"No identified conversations with this conflict in scope."});
  const category = Object.hasOwn(coverageNames,conflictCoverage) ? conflictCoverage :
    (summary.untested ? "untested" : summary.partial ? "partial" : "complete");
  const page = await api("conflict-conversations",{...filters,coverage:category,offset:pageOffset});
  byId("summary").textContent = factorName(summary.conflict.sources) + " · " + summary.conflict.rule + " · " + summary.conflict.scope;
  pagination(page.next_offset,{...selection,conflict_coverage:category});
  return element("div",{className:"feed"}, link("Back to source factor",url({factor:factorId})),
    panel(plural(summary.conversations,"conversation") + " · " + plural(summary.calls,"conflicting call"),
      element("p",{text:"A call counts as tested on both sides when one report edits each source separately against the same original proposal."}),
      element("nav",{className:"coverage-options","aria-label":"Conflict test coverage"},...Object.entries(coverageNames).map(([key,label]) =>
        link(summary[key] + " · " + label,url({...selection,conflict_coverage:key}),null,key===category)))),
    panel(coverageNames[category], page.rows.length ? listing(page.rows,row => {
      const missing = row.sources.filter(s => !row.tested_sources.includes(s.id)).map(sourceName);
      const evidence = row.tested === 2 ? "Both sources compared in one report" :
        missing.length ? "Awaiting an individual comparison · " + missing.join(" and ") :
        "Each source was tested in a separate report";
      return rowLink(row.thread,plural(row.calls,"conflicting call") + " · " + evidence,
        url({...selection,trace:row.trace,call:row.call,from_coverage:category,from_offset:pageOffset || null}));
    }) : element("p",{text:"No conversations in this category."})));
}

function conflictTest(card) {
  const test = card.conflict_test;
  if (!test) return null;
  const {trace_id:trace, call_id:call, detail} = card.evidence;
  const hasTrials = test.sources.some(s => s.comparisons.length);
  const chooseOutcome = element("select", {}, ...test.decisions.map(d => element("option", {value:d.id, text:d.label})));
  const editable = s => s.captured && s.matching_fields !== 0;
  function plan(label, first, second) {
    const action = link(label,"#");
    const setTarget = () => {
      const hash = new URLSearchParams({v:1,view:"decisions",run:trace,call,decision:chooseOutcome.value,context:first,plan:"edit"});
      if (second) hash.set("other_context",second);
      action.href = "/details?" + new URLSearchParams({run:trace}) + "#" + hash;
    };
    setTarget();
    chooseOutcome.addEventListener("change",setTarget);
    return action;
  }
  const sources = test.sources.map(result => {
    const source = detail.sources.find(s => s.id === result.id);
    const name = sourceName(source);
    const trials = result.comparisons.map(t => element("div", {}, link(plural(t.reports, "replay report") + (t.changed ? " with a changed proposal" : " with a matching proposal"),
      url({recommendation:t.group,trace,call,decision:t.decision,replay:t.replay,context:result.id}) + "#proposal-pair"),
      element("p",{className:"row-extra",text:"Example · " + proposalLabel(t.pair.original,t.pair) + " → " + proposalLabel(t.pair.edited,t.pair)})));
    const label = test.recorded_request ? "Prepare edit to " + name : "Supply request to test " + name;
    const action = test.decisions.length && editable(result) && plan(label,result.id);
    return element("div", {}, element("strong", {text:name + version(source.version)}),
      ...trials,
      !trials.length && element("p",{text:"No comparable single-source replay."}),
      action || element("p",{className:"label",text:!result.captured ? "Source text unavailable." :
        !test.decisions.length ? "No recorded answer or tool proposal in this rule scope." : "No matching field in the recorded request."}),
      result.matching_fields > 1 && element("p",{className:"label",text:"Choose the intended request field in the planner."}));
  });
  return panel(hasTrials ? "What changed when each source was edited?" : "Test one source at a time",
    element("p",{text:hasTrials ? "Each link opens one complete original and edited proposal comparison on this call." :
      "Keep the other declaration in place and compare the original and edited proposals."}),
    !test.recorded_request && element("p",{className:"uncertainty",text:"The original request body was not captured. Supply it to locate the editable field."}),
    test.decisions.length > 1 && element("label",{}, "Outcome to test ",chooseOutcome),
    element("div",{className:"comparison"},...sources),
    !detail.conflict_trials?.same_report && test.decisions.length && test.sources.length === 2 && test.sources.every(editable) &&
      plan("Compare both edits in one replay",test.sources[0].id,test.sources[1].id));
}

function findingReview(card, occurrences, dispatchSummary) {
  const { trace_id, call_id, detail } = card.evidence;
  byId("title").textContent = detail.joint_pattern ? "Two-source replay" : "Recommendation";
  byId("summary").textContent = detail.joint_pattern ? [card.evidence.thread, detail.model,
    plural(detail.measurement.baseline.attempted, "trial") + " per condition"].filter(Boolean).join(" · ") : card.action.basis;
  if (card.kind === "source_mask") {
    byId("title").textContent = "Tool proposals after masking sources";
    byId("summary").textContent = plural(card.threads, "identified conversation") + " · " + plural(card.traces,"captured request");
    return element("div", {className:"feed"}, link("Back to source factor",url({factor:factorId || detail.factor})),
      panel(findingTitle(card.kind,detail),
        element("p", {text:maskOutcomeSummary(card.mask_outcomes)}),
        element("p", {text:"Counts cover decisions with a saved replay in this scope. These tests resume from the same output prefix and use the same model and mask sources."}),
        element("p", {text:card.action.next})),
      ...occurrences.rows.map((row,i) => element("details", {className:"panel",...(i === 0 && {open:""})},
        element("summary", {text:[row.thread || "Request without a conversation ID",maskOutcomes[row.detail.mask.outcome]].join(" · ")}),
        maskComparison(row.detail.mask, false),
        row.thread_key && link("Read the captured conversation",url({view:"threads",thread:row.thread_key,focus_trace:row.trace,focus_call:row.call})))));
  }
  if (card.kind === "prefill" && openingStudy && detail.opening && traceId) {
    const context = query.get("opening_context");
    const ranked = detail.ranked.filter(r => !context || r.context_id === context);
    const checks = detail.opening.checks.filter(c => !context || c.context_id === context);
    byId("title").textContent = card.evidence.thread || "Source edit comparison";
    byId("summary").textContent = detail.model_id + " · " + detail.runtime;
    const back = link("Back to source edits",url({...openingSelection(),offset:query.get("from_offset")}));
    if (!ranked.length) return panel("This source is unavailable",back);
    const comparison = openingComparison({...detail,ranked,proposal_checks:checks},null);
    const proposals = comparison.querySelector("section[data-proposal-check]")?.closest("details");
    if (proposals) proposals.open = true;
    const audit = detail.selector_check;
    const witness = audit?.rank_review?.witness;
    const lower = witness && detail.proposal_checks[witness.lower_check];
    const matchingReadout = audit?.token_contrast && lower && (!context || lower.context_id === context);
    const editName = check => sourceName(detail.ranked.find(r => r.context_id === check.context_id));
    return element("div",{className:"feed"},back,comparison,
      matchingReadout && element("section",{className:"panel target-check"},tokenContrast(audit.token_contrast,
        ["Original","Edit " + editName(detail.proposal_checks[witness.top_check]),"Edit " + editName(lower)])),
      audit?.token_contrast_issue && element("p",{className:"label",text:audit.token_contrast_issue}));
  }
  if (card.kind === "prefill") {
    const titles = { tools:"Tool definitions changed the edit’s effect", instruction:"The instruction changed the edit’s effect",
      combined:"The edit depended on the routing setup", constant:"The edit’s effect held across setups", incomplete:"Some routing trials are missing" };
    const repeated = repeatedRouting(card);
    const oneRepeated = card.routing_profiles?.length === 1 && card.routing_profiles[0].inputs === 1 && card.routing_profiles[0].repeat_counts;
    const witnesses = oneRepeated && card.routing_profiles[0].repeat_evidence;
    const contrast = witnesses && (witnesses.tool || witnesses.proposal);
    const headline = contrast ? (witnesses.tool ? "Tool selection" : "The full proposal") + " changed in " +
      contrast.changed_runs + " of " + (contrast.changed_runs + contrast.held_runs) + " completed runs in one setup" : card.action.title;
    if (review === "lower_changed" && detail.selector_check?.rank_review?.witness) {
      byId("title").textContent = "A lower-ranked edit changed the proposal";
      byId("summary").textContent = card.evidence.thread || "Measured call";
      return element("div", {className:"feed"}, link("Back to edit rankings",url({ranking,review,quality})),
        rankWitness(detail), element("details", {className:"panel"}, element("summary", {text:"All tested source edits"}),
          firstTokenPreview(detail, true)), link("Open original span",url({view:"traces",trace:trace_id,call:call_id})));
    }
    byId("title").textContent = oneRepeated ? "Repeat trials on one input" : repeated ? "Routing results across requests" : titles[detail.routing_test?.pattern] || "First-token sensitivity";
    byId("summary").textContent = repeated ? routingCoverage(card) : prefillSummary(card.prefill_counts);
    return element("div", { className: "feed" },
      ranking ? link("Back to edit rankings",url({ranking,quality,review})) : link("Back to source factor", url({ factor: factorId || detail.factor })),
      !ranking && detail.selector_check ? link("Compare ranked edits across calls",url({ranking:"all"})) : null,
      panel(headline, repeated ? null : prefillFollowup(card), element("p", {text:contrast ?
        "Open a changed and an unchanged run below. Check the original and edited proposals before deciding whether to change the source." : card.action.next})),
      (measuredExample.trace || measuredExample.routing_input) && detail.routing_test ? link("All routing results in this scope", url({factor:factorId || detail.factor, recommendation:card.id, routing_design:detail.routing_test.design})) : null,
      repeated ? routingPatterns(card) : routingComparison(detail),
      !oneRepeated && element("p", { text: "These calls share the tested edits, model, runtime and request settings. Other user text can differ." }),
      ...occurrences.rows.map((row, i) => element("details", { className: "panel", ...(!repeated && i === 0 && { open: "" }) },
        element("summary", { text: (row.thread || "Request without a conversation ID") + " · " + findingTitle("prefill", row.detail) }),
        row.detail.selector_check?.rank_review?.status === "lower_changed" &&
          link("Inspect the lower-ranked edit that changed the proposal",url({ranking:"all",review:"lower_changed",recommendation:card.id,trace:row.trace,call:row.call})),
        firstTokenPreview(row.detail, true),
        card.routing_design_count > 1 && row.detail.routing_test ? link("Compare inputs tested with this design",
          url({recommendation:card.id, factor:factorId || detail.factor, routing_design:row.detail.routing_test.design})) : null,
        !repeated && row.trace === trace_id && row.call === call_id ? null : routingComparison(row.detail),
        link("Open original span", url({ view: "traces", trace: row.trace, call: row.call, finding: card.id })))));
  }
  const traceLink = url({ view: "traces", trace: trace_id, call: call_id, finding: card.id });
  const evidence = element("details", { className: "panel" }, element("summary", { text: "Underlying evidence" }),
    link("Open original span", traceLink),
    ...(card.kind === "replacement" ? replacementLinks(detail, trace_id, call_id) : []),
    ["replay","baseline_check"].includes(card.kind) ? link(card.kind === "baseline_check" ? "Inspect unchanged-request trials" : "Open trial comparison", advancedHref({ id: trace_id }, { id: call_id }, { kind:card.kind, detail })) : null);
  return element("div", { className: "feed" },
    link("Back to source factor", url({ factor: factorId || detail.factor })),
    card.kind === "token" ? tokenPath(card) : null, sandboxDispatch(detail.sandbox_dispatch), dispatchCoverage(card,dispatchSummary),
    card.prefix_review ? prefixReview(card.prefix_review) : card.kind === "token" ? element("details", {className:"panel"}, element("summary", {text:"Peak score change and block swap"}),
      recommendationDetail(card.kind, detail, card.action, card.trend)) : recommendationDetail(card.kind, detail, card.action, card.trend, card.conflict_test?.sources.some(s => s.comparisons.length)),
    conflictTest(card),
    card.kind === "conflict" && card.threads > 1 && factorId ? link("Review test coverage across conversations",
      url({recommendation:card.id,factor:factorId,conflict_coverage:"all"})) : null,
    jointRecurrence(card),
    measuredExample.trace ? link("See this recommendation across requests", url({ recommendation: card.id, factor: factorId })) : null,
    card.threads > 1 ? compareForm(card.id) : null, evidence,
    panel("Where this appeared", ...occurrences.rows.map((row) => element("details", { className: "occurrence" },
      element("summary", { text: (row.thread || "Request without a conversation ID") + " · " + fullDate(row.observed) }),
      element("p", { text: findingTitle(row.kind, row.detail) }), sandboxDispatch(row.detail.sandbox_dispatch), visual(row.kind, row.detail),
      element("div", { className: "actions" },
        card.prefix_review && link("Inspect the prefix scores on this call", url({recommendation:card.id,factor:detail.factor,trace:row.trace,call:row.call,report:row.detail.report_id})),
        row.detail.sandbox_dispatch && row.detail.sandbox_dispatch.design !== detail.sandbox_dispatch?.design &&
          link("Compare these dispatch settings",url({recommendation:card.id,factor:detail.factor,
            dispatch_design:row.detail.sandbox_dispatch.design,dispatch_outcome:row.detail.sandbox_dispatch.outcome})),
        row.thread_key ? link("Open conversation", url({ view: "threads", thread: row.thread_key, focus_trace:row.trace, focus_call:row.call })) : null,
        link("Open span " + row.call, url({ view: "traces", trace: row.trace, call: row.call, finding: card.id })))))));
}

function priorDispatch(history, hasCurrentDispatch) {
  const rows = history?.dispatch?.rows;
  if (!rows?.length) return null;
  const [first] = rows;
  const heading = hasCurrentDispatch ? "Compare earlier dispatch replays" :
    first.regressed || first.mixed ? "Review earlier dispatch results before using the check" :
    first.helped ? "Test the dispatch check on this request" : "Compare earlier dispatch outcomes";
  const evidence = row => {
    const e = row.example;
    return url({recommendation:e.group,factor:e.factor,trace:e.trace,call:e.call,report:e.report});
  };
  const countText = row => [[row.helped,"only with check"],[row.regressed,"only without check"],
    [row.both,"both matched"],[row.neither,"neither matched"],[row.mixed,"mixed"]]
    .filter(([count]) => count > 0).map(([count,label]) => count + " " + label).join(" · ");
  return element("section", {className:"panel", "aria-label":"Earlier dispatch replays"},
    NexusWeaveA2UI.recommendation("earlier-dispatch", {
      title:heading, heading:"h2", label:"Evidence from earlier requests",
      description:"Earlier requests shared the workflow, model, request layout and declared rule values." +
        (hasCurrentDispatch ? "" : " Run a fresh sandbox test to check what happens on this request."),
      detail:history.dispatch.truncated ? "The lookup used the latest 128 matching observations." : null,
      evidence:rows.map(row => ({title:"Open earlier sandbox comparison",
        summary:plural(row.inputs,"distinct earlier input") + " · " + countText(row) +
          " · " + row.example.guard + " · " + row.example.runner, href:evidence(row)})),
    }));
}

function priorEdits(history, hasCurrentScores) {
  if (history?.status !== "checked") return null;
  const [top] = history.edits;
  const limit = history.truncated ? " The lookup used the latest 512 matching observations." : "";
  if (!top) return history.dispatch?.rows?.length ? null :
    element("p", { className: "label", text: "No comparable source edit on two distinct earlier inputs in this scope." + limit });
  function trial(edit) {
    const e = edit.example;
    return url({recommendation:e.group, factor:e.factor, trace:e.trace, call:e.call,
      decision:e.decision, replay:e.report, context:e.context}) + (edit.proposals.changed ? "#proposal-pair" : "");
  }
  const e = top.example;
  const suggested = top.proposals.changed || top.lower;
  function counts(pairs) {
    return Object.entries(pairs).filter(([,count]) => count).map(([name,count]) => count + " " + name).join(" · ");
  }
  const rows = history.edits.map(edit => element("tr", {},
    element("th", { scope: "row" }, link(edit.example.source, trial(edit))),
    element("td", { text: edit.example.replacement || "(Removed text)" }),
    element("td", { text: counts(edit.proposals) }),
    element("td", { text: counts({lower:edit.lower, equal:edit.equal, mixed:edit.mixed}) })));
  const experiment = history.experiment;
  const ready = experiment?.status === "ready";
  const tests = experiment?.proposal_tests || [];
  const single = experiment?.total_proposal_tests === 1 && tests[0].pair;
  const heading = single ? (single.changed ? "The proposal changed" : "The proposal stayed the same") + " after editing " + e.source :
    tests.length ? "Current proposal tests for " + e.source :
    experiment?.status === "already_applied" ? e.source + " already contains the tested replacement" :
    !ready ? "Review earlier source edits" :
    suggested && hasCurrentScores ? "Earlier replays suggested testing " + e.source :
    suggested ? "Test " + e.source + " on this request" : "Review earlier source edits";
  const download = element("button", {type:"button", text:"Download edit for this request"});
  if (ready) download.addEventListener("click", () => downloadJson("source-edit.json", experiment.interventions));
  return element("section", { className: "panel replacement prior-edits" },
    element("span", { className: "label", text: tests.length ? "Test on this request" : "Evidence from earlier requests" }),
    element("h2", { text: heading }),
    experiment?.first_token_js_bits != null ? element("p", {text:"First-token distribution shift for this edit · " +
      Number(experiment.first_token_js_bits.toPrecision(3)) + " bits of Jensen–Shannon divergence."}) : null,
    ...tests.map(test => {
      const pair = responsePair(test.pair);
      if (pair) pair.open = Boolean(single);
      return element("div", {}, pair || element("p", {text:"No consistent proposal pair. Original trials completed " +
        test.baseline.completed + "/" + test.baseline.attempted + " times and edited trials completed " +
        test.edited.completed + "/" + test.edited.attempted + " times."}),
        element("details", {}, element("summary", {text:"Runner and report"}),
          element("p", {text:test.runner}), element("p", {text:test.report})));
    }),
    experiment?.total_proposal_tests > tests.length ? element("p", {text:"Showing five of " + experiment.total_proposal_tests + " matching reports."}) : null,
    element("p", { text: "Replacing " + e.source + " with " + JSON.stringify(e.replacement) +
      (ready && experiment.interventions[0].replacement !== e.replacement ? " (excerpt)" : "") +
      (top.proposals.changed ? " changed the generated proposal on " + top.proposals.changed :
        " reduced the match rate on " + top.lower) + " of " + plural(top.requests, "distinct earlier input") + "." }),
    element("p", { text: "These requests shared the model, request settings, source versions and positions. Their counts describe earlier answers." + limit }),
    element("div", {className:"actions"}, ready && download,
      link(top.proposals.changed ? "Compare an earlier answer or tool call" : "Inspect an earlier replay", trial(top))),
    element("details", {}, element("summary", { text: "Compare tested edits" }),
      element("div", {className:"routing-table", role:"region", "aria-label":"Earlier source edits", tabindex:"0"},
        element("table", {}, element("thead", {}, element("tr", {}, ...["Source", "Replacement", "Generated proposal", "Match rate"].map(text => element("th", { scope: "col", text })))),
          element("tbody", {}, ...rows))),
      element("p", { className: "label", text: "Each input counts once per edit. Mixed means completed comparisons disagreed. Unresolved means at least one report lacked a consistent pair of completed proposals." })));
}

function proposalComparisons(probe) {
  const checks = probe.proposal_checks;
  const proposal = (p) => p.kind === "model_output" ? p.outcome_preview : p.label + " · " + p.outcome_preview;
  if (!checks.length) return element("p", { text: "No completed single-trial proposal comparison is attached for these edits." });
  return element("details", {}, element("summary", { text: "Compare generated proposals" }),
    ...checks.map((check) => {
      const edit = probe.ranked.find((row) => row.context_id === check.context_id);
      return element("section", {"data-proposal-check":check.report, "data-context":check.context_id}, element("h3", { text: (edit.source || edit.context_id) +
        (check.changed ? " · proposal changed" : " · same proposal") }),
        element("div", { className: "comparison" },
          element("div", {}, element("strong", { text: "Original proposal" }), element("code", { text: proposal(check.original) })),
          element("div", {}, element("strong", { text: "Edited proposal" }), element("code", { text: proposal(check.edited) }))),
        element("p", { className: "label", text: (check.baseline_trials ? plural(check.baseline_trials,"original trial") + " · " + plural(check.edited_trials,"edited trial") : "One completed proposal per condition") + " · " + check.runner }));
    }));
}

function routingInstructions(test) {
  return element("details", {}, element("summary", {text:"Compare instruction text"}),
    element("div", {className:"comparison"}, ...test.instructions.map((text, i) =>
      element("div", {}, element("strong", {text:i ? "Alternate" : "Original"}), element("p", {text})))));
}

function routingComparison(probe) {
  if (probe.routing_issue) return element("p", {className:"label", text:probe.routing_issue});
  const test = probe.routing_test;
  if (!test) return null;
  const source = probe.ranked.find(row => row.context_id === test.context_id);
  const toolList = (names) => names.length ? names.join(", ") : "No tools";
  const trialText = trial => trial.status === "completed"
    ? trial.label + (trial.listed === false ? " (unlisted)" : "") + " · " + trial.outcome
    : "Trial failed";
  const rows = test.cells.map((cell, index) => element("tr", {className:cell.changed ? "changed" : "", "data-routing-setup":index},
    element("th", {scope:"row", text:index < 2 ? "Original" : "Alternate"}),
    element("td", {text:(index % 2 ? "Alternate · " : "Original · ") + toolList(test.inventories[index % 2])}),
    ...cell.trials.map(trial => element("td", {}, element("code", {text:trialText(trial)}))),
    element("td", {text:cell.changed == null ? "Incomplete" : cell.changed && !cell.tool_changed ? "Changed · same tool selection" : cell.changed ? "Changed" : "Unchanged"})));
  return element("section", {className:"panel routing-comparison"},
    element("h2", {text:"When did the source edit change the proposal?"}),
    element("p", {text:"Each row tests the same edit to " + (source.source || source.context_id) +
      " on one request. Only the instruction text and tool definitions vary across rows."}),
    element("div", {className:"routing-table", role:"region", "aria-label":"Four routing setups", tabindex:"0"},
      element("table", {}, element("thead", {}, element("tr", {}, ...["Instruction", "Tools", "Original proposal", "Edited proposal", "Result"]
        .map(text => element("th", {scope:"col", text})))), element("tbody", {}, ...rows))),
    routingInstructions(test),
    test.other_user_text?.length ? element("details", {}, element("summary", {text:"Other user messages"}),
      ...test.other_user_text.map(text => element("p", {text})),
      element("small", {text:"Up to three preceding or following text messages, in request order."})) : null,
    element("p", {className:"label", text:"One proposal per condition · " + test.runner}));
}

function openingSelection() {
  return {opening:1, recommendation:recommendationId, factor:factorId, slot:openingSlot};
}

async function openingCoverageView() {
  const filters = {...scope, group:recommendationId, factor:factorId};
  const selection = openingSelection();
  const [summary, sources, examples] = await Promise.all([api("opening-coverage",filters), api("opening-sources",filters),
    api("opening-conversations",{...filters,...(openingSlot && {slot:openingSlot}),offset:pageOffset})]);
  const group = summary.rows[0];
  byId("title").textContent = "When the opening token held";
  pagination(examples.next_offset,selection);
  if (!group) return panel("No scored conversations in this scope");
  byId("summary").textContent = plural(group.affected,"conversation") + " had a changed proposal. " +
    group.tested + " of " + plural(group.scored,"scored conversation") + " had a completed comparison for an edit that kept the same token on top.";
  const arms = [...sources.rows];
  let next = sources.next_offset;
  while (next != null) {
    const page = await api("opening-sources",{...filters,offset:next});
    arms.push(...page.rows);
    next = page.next_offset;
  }
  const selected = arms.find(a => a.slot === openingSlot);
  const name = arm => arm.source || arm.pointer;
  return element("div",{className:"feed"},link("Back to recommendations",url({})),
    element("section",{className:"panel replacement"},element("h2",{text:"Which source edits changed a proposal?"}),
      comparisonTable(["Source edited","Opening held","Proposal tested","Proposal changed"],arms.map(arm => {
        const duplicate = arms.some(a => a.slot !== arm.slot && name(a) === name(arm));
        const location = duplicate ? " · " + arm.pointer + (arm.text_range ? " · byte " + arm.text_range.start_byte : "") : "";
        return element("tr",{},element("th",{scope:"row"},arm.affected ? link(name(arm) + location,
          url({...selection,slot:arm.slot}),null,arm.slot===openingSlot) : name(arm) + location),
          element("td",{text:arm.scored}),element("td",{text:arm.tested}),element("td",{},String(arm.affected),
            arm.tested > 0 && element("meter",{min:0,max:arm.tested,value:arm.affected,"aria-label":name(arm) + " changed among tested conversations"})));
      })),
      element("p",{className:"row-extra",text:"Each column counts conversations. The same conversation can appear under two sources. A change needs at least one completed original and edited proposal comparison."})),
    panel(selected ? "Changes after editing " + name(selected) : "Conversations with a changed proposal",
      openingSlot && link("Show all sources",url({...selection,slot:null})),
      examples.rows.length ? listing(examples.rows,row => rowLink(row.question || row.thread,
        [row.thread,row.source].filter(Boolean).join(" · "),url({...selection,trace:row.trace,call:row.call,opening_context:row.context,
          from_offset:pageOffset || null}))) : element("p",{text:"No changed proposals for this selection."})));
}

function openingComparison(probe, question) {
  const checks = probe.proposal_checks || [];
  const flips = probe.ranked.filter(edit => edit.opening_changed);
  const source = edit => edit.source || edit.context_id;
  const held = probe.ranked.find(edit => !edit.opening_changed &&
    checks.some(check => check.context_id === edit.context_id && check.changed));
  let heading = "The same opening token led under every tested edit";
  if (held) heading = "Editing " + source(held) + " changed the proposal with the same opening token on top";
  else if (flips.length === 1) heading = "Editing " + source(flips[0]) + " changed the most likely opening token";
  else if (flips.length) heading = plural(flips.length, "edit") + " changed the most likely opening token";
  const rows = probe.ranked.map(edit => {
    const matches = checks.filter(check => check.context_id === edit.context_id);
    const results = new Set(matches.map(check => check.status === "varied" ? "Trials varied" :
      check.status === "incomplete" ? "Incomplete" : check.changed ? "Changed" : "Held"));
    const outcome = [...results].join(" / ") || "Unmeasured";
    return element("tr", {className:results.has("Changed") ? "changed" : ""}, element("th", {scope:"row",text:source(edit)}),
      element("td", {text:edit.opening_changed ? "Changed" : "Held"}), element("td", {text:outcome}));
  });
  return element("section", {className:"panel replacement opening-comparison"},
    element("span", {className:"label", text:checks.length ? "Opening scores and proposal trials" : "Before the first output token"}), element("h2", {text:heading}),
    question && element("blockquote", {text:question}),
    element("table", {}, element("thead", {}, element("tr", {}, ...["Source edited","Most likely opening","Complete proposal"]
      .map(text => element("th", {scope:"col",text})))), element("tbody", {}, ...rows)),
    !checks.length && element("p", {text:flips.length === 1
      ? "Test the complete proposal after editing " + source(flips[0]) + "."
      : "Test the complete proposals to see whether a difference appears later in the answer."}),
    checks.some(check => check.original) && proposalComparisons({...probe,proposal_checks:checks.filter(check => check.original)}),
    element("details", {}, element("summary", {text:"First-token measurements"}),
      element("p", {text:probe.model_id + " · " + probe.model_revision + " · " + probe.runtime}),
      ...probe.ranked.map(edit => element("p", {text:source(edit) + " · token ID " + probe.top_token_id + " → " +
        edit.top_token_id + " · " + Number(edit.js_bits.toPrecision(3)) + " bits of Jensen–Shannon divergence"}))));
}

function firstTokenPreview(probe, recorded = false, tested = false) {
  const [top] = probe.ranked;
  const tied = probe.ranked.filter((row) => row.js_bits === top.js_bits).length;
  const name = (row) => row.source || row.context_id;
  const bits = (value) => Number(value.toPrecision(3)).toString() + " bits";
  const checks = probe.proposal_checks || [];
  const outcomes = new Map();
  for (const check of checks) {
    const value = check.changed ? "Changed" : "Held";
    const prior = outcomes.get(check.context_id);
    outcomes.set(check.context_id, prior && prior !== value ? "Results differed" : value);
  }
  const describe = {"Changed":"The top edit changed the completed proposal.",
    "Held":"The completed proposal stayed the same after the top edit.",
    "Results differed":"Completed comparisons disagreed about the top edit."};
  const hasOpening = top.opening_changed !== undefined;
  const rows = probe.ranked.map((row) => element("tr", {},
    element("th", { scope: "row", title: row.version || row.context_id }, name(row),
      probe.selector_check?.first === row.context_id && element("small",{text:"First listed edit"})),
    element("td", {}, element("meter", { min: 0, max: top.js_bits || 1, value: row.js_bits, "aria-label": name(row) + " divergence" })),
    element("td", { text: bits(row.js_bits) }),
    hasOpening && element("td", {text:row.opening_changed ? "Changed" : "Held"}),
    recorded && element("td", {text:outcomes.get(row.context_id) || "No completed comparison"})));
  return element("section", { className: "panel replacement" },
    element("span", { className: "label", text: recorded ? "Captured first-token measurement" : "Before the first output token" }),
    element("h2", { text: top.js_bits === 0 ? "The tested edits left the first-token distribution unchanged" :
      tied === 1 ? (recorded || tested ? "Largest shift from editing " : "Test the edit to ") + name(top) : plural(tied, "source edit") + " tied for the largest shift" }),
    element("p", { text: "Largest shift across " + plural(rows.length, "tested edit") + " · " + bits(top.js_bits) +
      " of Jensen–Shannon divergence." + (recorded || tested ? "" : " Replay the original and edited requests to see whether the answer or tool call changes.") }),
    recorded && tied === 1 && element("p", {text:describe[outcomes.get(top.context_id)] || "The effect of the top edit on the completed proposal is unmeasured."}),
    tied === 1 && top.before !== undefined ? element("div", { className: "comparison" },
      element("div", {}, element("strong", { text: "Original" }), element("code", { text: top.before })),
      element("div", {}, element("strong", { text: "Edited" }), element("code", { text: top.operation === "remove_json" ? "(Removed)" : top.replacement }))) : null,
    recorded && probe.tool_count !== undefined ? element("p", { className: "label",
      text: "Original request · " + plural(probe.tool_count, "available tool") +
        (probe.tool_names.length ? " · " + probe.tool_names.join(", ") : "") }) : null,
    recorded && probe.selector_check?.token_contrast_issue && element("p",{text:probe.selector_check.token_contrast_issue}),
    recorded && probe.reference_issue && element("p",{text:probe.reference_issue}),
    recorded && probe.answer_reference ? referenceDetail(probe) : null,
    recorded ? proposalComparisons(probe) : null,
    element("details", {}, element("summary", { text: "Compare source edits" }),
      element("table", {}, element("thead", {}, element("tr", {}, ...["Source", "Relative to largest shift", "Divergence", ...(hasOpening ? ["Opening token"] : []), ...(recorded ? ["Completed proposal"] : [])].map(
        (text) => element("th", { scope: "col", text })))), element("tbody", {}, ...rows)),
      element("p", { className: "label", text: probe.model_id + " · " + probe.model_revision + " · " + probe.runtime })));
}

function targetCriterion(expected, rule) {
  const value = expected.kind === "model_output" ? expected.outcome_preview : expected.label + " · " + expected.outcome_preview;
  if (rule.kind === "kind_and_label") return expected.kind + " · " + expected.label + " · Match kind and label";
  if (rule.kind === "outcome_fields") return value + " · Match fields " + rule.pointers.join(", ");
  if (rule.kind === "text_capture") return value + " · Match captured text · " + rule.pattern;
  return value + " · Exact match";
}

const rankingLabels = {ranked_only:"Only the ranked edit changed the proposal", first_only:"Only the first listed edit changed it",
  both:"Both edits changed it", neither:"Neither edit changed it", missing:"Missing a completed pair", mixed:"Completed pairs disagreed"};

const qualityLabels = {ranked_lost:"Matches lost after the ranked edit",ranked_gained:"Matches gained after the ranked edit",
  first_lost:"Matches lost after the first listed edit",first_gained:"Matches gained after the first listed edit"};

const reviewLabels = {
  lower_changed:"A lower-ranked edit changed the proposal while the top edit held",
  top_changed:"The top edit changed the proposal",
  all_held:"All tested edits kept the original proposal",
  incomplete:"Evidence is incomplete",
  mixed:"Reports disagreed",
  tied:"Several edits tied for the largest shift"
};

function tokenContrast(readout, conditions = ["Original","Top edit","Lower-ranked edit"]) {
  const names = readout.tokens.map((token,i) => readout.stop[i] ? "End of answer" : JSON.stringify(token));
  const percent = new Intl.NumberFormat(undefined,{style:"percent",maximumFractionDigits:1});
  const rows = readout.probabilities.map((pair,i) => element("tr",{},
    element("th",{scope:"row",text:conditions[i]}),
    ...pair.map((value,j) => element("td",{}, element("meter",{min:0,max:1,value,"aria-label":names[j]+" probability"}),
      element("small",{text:percent.format(value)+(j===(i===2?1:0) ? " · chosen" : "")})))));
  const prefix = readout.position === 1 ? "The first output token differed." :
    "All three answers started with " + JSON.stringify(readout.prefix) + ". Their next token differed.";
  return element("section",{"aria-label":"Token probabilities"},element("h3",{text:"The choice changed at token " + readout.position}),
    element("p",{text:prefix}),comparisonTable(["Request",...names],rows),
    element("small",{text:"Greedy rerun on the same checkpoint. Each column shows one token’s probability in the full vocabulary."}));
}

function rankWitness(probe) {
  const witness = probe.selector_check.rank_review.witness;
  const top = probe.proposal_checks[witness.top_check];
  const lower = probe.proposal_checks[witness.lower_check];
  const proposal = choice => choice.kind === "model_output" ? choice.outcome_preview : choice.label + " · " + choice.outcome_preview;
  const rows = [top, lower].map(check => {
    const index = probe.ranked.findIndex(row => row.context_id === check.context_id);
    const edit = probe.ranked[index];
    const heading = edit.source || edit.context_id;
    const input = element("details", {}, element("summary", {text:"Source edit"}),
      element("p", {text:edit.before}), element("code", {text:edit.operation === "remove_json" ? "(Removed)" : edit.replacement}),
      element("small", {text:[edit.version,edit.pointer].filter(Boolean).join(" · ")}));
    return element("tr", {}, element("th", {scope:"row", text:heading}, element("small", {text:"Rank " + (index+1) + " of " + probe.ranked.length}), input),
      element("td", {}, element("meter", {min:0,max:probe.ranked[0].js_bits,value:edit.js_bits,"aria-label":heading+" divergence"}),
        element("small", {text:Number(edit.js_bits.toPrecision(3)) + " bits"})),
      element("td", {}, element("code", {text:proposal(check.edited)}),element("small", {text:check.changed ? "Changed" : "Held"})));

  });
  return element("section", {className:"panel target-check rank-witness","aria-label":"Measured source contrast"},
    element("h2", {text:"Two edits on the same request"}),
    element("p", {}, element("strong", {text:"Original proposal"}), " · ", element("code", {text:proposal(top.original)})),
    comparisonTable(["Source","First-token shift","Edited proposal"],rows),
    probe.selector_check.token_contrast && tokenContrast(probe.selector_check.token_contrast),
    probe.selector_check.token_contrast_issue && element("p",{text:probe.selector_check.token_contrast_issue}),
    element("p", {className:"label",text:"Each source was edited separately. The shifts were measured before generation. The proposals came from one completed trial per condition."}),
    element("details", {}, element("summary",{text:"Measurement and replay"}),element("p",{text:[probe.model_id,probe.model_revision,probe.runtime].join(" · ")}),
      element("p",{text:[top.runner,top.report,top.decision].filter(Boolean).join(" · ")})));
}

function referenceDetail(probe) {
  const first = probe.selector_check.first;
  const top = probe.ranked[0].context_id;
  const checks = probe.proposal_checks.filter(c => c.reference_match && (c.context_id === first || c.context_id === top));
  const rows = checks.map(check => {
    const selector = check.context_id === top ? (first === top ? "Ranked and first" : "Ranked") : "First listed";
    const source = probe.ranked.find(r => r.context_id === check.context_id).source;
    const answers = [check.original, check.edited].map((out, i) => element("td", {},
      element("code", {text:out.outcome_preview}), element("small", {text:check.reference_match[i] ? "Matched" : "Missed"})));
    return element("tr", {}, element("th", {scope:"row", text:selector}, element("small", {text:source})), ...answers);
  });
  return element("section", {className:"reference-detail target-check", "aria-label":"Reference answer check"},
    element("h3", {text:"Reference answer check"}),
    element("p", {text:"Accepted word sequences: " + probe.answer_reference.aliases.join(" · ")}),
    rows.length ? comparisonTable(["Edit","Original answer","Edited answer"], rows) :
      element("p", {text:"No completed text comparison for these selectors."}),
    element("small", {text:"The check uses the full answer. A matching word sequence can occur in a factually incorrect answer."}));
}

function comparisonTable(headers, rows) {
  return element("table", {}, element("thead", {}, element("tr", {}, ...headers.map(text => element("th", {scope:"col", text})))),
    element("tbody", {}, ...rows));
}

function referencePanel(rows, total) {
  if (!rows.length) return null;
  const sum = predicate => rows.reduce((n, row) => n + (predicate(row.reference) ? row.calls : 0), 0);
  const scored = sum(r => r.status === "scored");
  const original = sum(r => r.status === "scored" && r.original);
  const mixed = sum(r => r.status === "mixed");
  const selectors = ["ranked","first"].map(selector => {
    const cells = ["lost","gained"].map(change => {
      const lost = change === "lost";
      const eligible = lost ? original : scored - original;
      const count = sum(r => r.status === "scored" && r.original === lost && r[selector] !== r.original);
      const key = selector + "_" + change;
      const label = count + " of " + eligible;
      const value = count ? element("a", {href:url({ranking:"all",quality:key}), text:label, "aria-label":qualityLabels[key] + ": " + label}) :
        eligible ? label : "No eligible calls";
      return element("td", {}, value);
    });
    return element("tr", {}, element("th", {scope:"row", text:selector === "ranked" ? "Ranked edit" : "First listed edit"}), ...cells);
  });
  return element("section", {className:"panel target-check", "aria-label":"Reference matches across calls"},
    element("h2", {text:"Did edits preserve reference answers?"}),
    element("p", {text:scored + " of " + plural(total,"call") + " had a scored reference comparison. " + plural(original,"original answer") + " matched."}),
    mixed ? element("p", {text:"Completed reference checks disagreed in " + plural(mixed,"call") + "."}) : null,
    comparisonTable(["Selector","Lost a match","Gained a match"], selectors));
}

async function rankingView() {
  const filtered = ranking !== "all";
  const [summary, reference, audits, examples] = await Promise.all([api("ranking-results",scope),api("ranking-reference",scope),api("ranking-review",scope),
    api("ranking-examples",{...scope, ...(filtered && {result:ranking}), ...(quality && {quality}), ...(review && {review}), offset:pageOffset})]);
  const counts = Object.fromEntries(summary.rows.map(row => [row.result,row.calls]));
  const total = summary.rows.reduce((n,row) => n + row.calls, 0);
  const same = summary.rows.reduce((n,row) => n + row.same_edit, 0);
  const scored = total - (counts.missing || 0) - (counts.mixed || 0);
  byId("title").textContent = "Which edit changed the proposal?";
  byId("summary").textContent = "Compare the edit with the largest first-token shift against the first edit on the workflow’s list.";
  pagination(examples.next_offset,{ranking,quality,review});
  if (!total) return panel("No rankings in this scope",element("p",{text:"This comparison needs a verified first-token report with at least two edits."}));
  const calls = element("section",{}, element("h2",{text:review ? reviewLabels[review] : quality ? qualityLabels[quality] : filtered ? rankingLabels[ranking] : "Measured calls"}),
      (review || quality || filtered) && link("Show all calls",url({ranking:"all"})),
      listing(examples.rows,row => rowLink(row.thread || row.call,[row.source,review ? reviewLabels[row.review] : rankingLabels[row.result]].filter(Boolean).join(" · "),
        url({ranking,quality,review,recommendation:row.group,trace:row.trace,call:row.call}))));
  return element("div",{className:"feed"},link("Back to recommendations",url({})),
    (quality || review) && calls,
    referencePanel(reference.rows,total),
    panel("Which changes would a top-edit test miss?",
      listing(audits.rows, row => rowLink(reviewLabels[row.review],plural(row.calls,"call"),url({ranking:"all",review:row.review}),row.review===review)),
      element("p", {className:"label",text:"Each call counts once. A lower-ranked change needs a replay of both edits in the same report. All edits must finish to establish that every proposal held."})),
    panel(scored + " of " + plural(total,"call") + " had a scored pair",
      scored > 0 && element("p",{text:"The ranked edit changed the proposal in " + ((counts.ranked_only || 0) + (counts.both || 0)) +
        " of " + scored + " scored calls. The first listed edit changed it in " + ((counts.first_only || 0) + (counts.both || 0)) + "."}),
      listing(Object.entries(rankingLabels).filter(([key]) => counts[key]), ([key,label]) =>
        rowLink(label,plural(counts[key],"call"),url({ranking:key}),key === ranking)),
      element("p",{className:"label",text:"Both selectors chose the same edit in " + plural(same,"call") +
        ". A pair uses completed single trials from the same report. Counts describe the selected calls, including repeated inputs."})),
    !quality && !review && calls);
}

function targetTokenScores(path, paired = true) {
  const positions = path.token_ids;
  const conditions = path.conditions;
  const firstMisses = conditions.filter(row => row.first_lower === 0).length;
  const headline = firstMisses === 0 ? "The opening token had the top score in every condition" :
    firstMisses === conditions.length ? "Another token led at the first position in every scored condition" :
    "Another token led at the first position in " + firstMisses + " of " + conditions.length + " conditions";
  const percent = n => n > 0 && n < 0.0001 ? "<0.01%" : Intl.NumberFormat(undefined,
    {style:"percent", maximumFractionDigits:2}).format(n);
  const token = s => JSON.stringify(s);
  const select = element("select", {"aria-label":"Target token position"}, ...positions.map((id,i) =>
    element("option", {value:i,text:(i + 1) + " · " + token(conditions[0].steps[i].text) + " · token " + id})));
  const body = element("tbody");
  const prefix = element("p", {className:"label"});
  function selectPosition() {
    const index = Number(select.value);
    prefix.textContent = index === 0 ? (path.assistant_prefix ? "After the supplied assistant prefix " + token(path.assistant_prefix) + "." : "At the answer boundary, before any output token.") :
      "Scores at this position assume the preceding " + plural(index, "target token") + ".";
    body.replaceChildren(...conditions.map(row => {
      const step = row.steps[index];
      return element("tr", {},element("th",{scope:"row",text:row.label}),
        element("td", {},element("meter", {min:0,max:1,value:Math.exp(step.log_prob),"aria-label":"Target token probability"}),
          element("small",{text:percent(Math.exp(step.log_prob))})),
        element("td", {},element("code",{text:token(step.top_text),title:"Token " + step.top_id}),
          element("small",{text:percent(Math.exp(step.top_log_prob))})));
    }));
  }
  select.addEventListener("change",selectPosition);
  selectPosition();
  return element("details", {className:"target-tokens"}, element("summary", {text:"Before generation · " + headline}),
    element("label", {}, "Target token ",select), prefix,
    element("table", {},element("thead", {},element("tr", {},
      ...["Condition","Target token probability","Highest-scoring token"].map(text => element("th",{scope:"col",text})))), body),
    element("small",{text:"The scorer reads the full vocabulary using the supplied prefix." + (paired ? " The completed proposal comes from a separate run." : "")}),
    element("small",{text:[path.model,path.revision,path.runtime].join(" · ")}));
}

function declaredTarget(check, question, focusOpening = false) {
  const proposal = p => p.kind === "model_output" ? p.outcome_preview : p.label + " · " + p.outcome_preview;
  const totals = {met:"Met", missed:"Missed", mixed:"Mixed", incomplete:"Incomplete"};
  const rows = [check.baseline, ...check.edits, ...(check.joint ? [check.joint] : [])].map((a, i) => {
    const c = a.counts;
    const score = c.completed - (c.unscored || 0);
    return element("tr", {"data-target-status":a.status},
      element("th", {scope:"row"}, a.label,
        a.edit ? element("small", {text:(a.control ? "Negative control · " : "") +
          (a.edit.operation === "remove_json" ? "Removed" : "Replaced with “" + a.edit.replacement + "”"),
          title:a.edit.pointer + (a.edit.version ? " · " + a.edit.version : "")}) : null),
      element("td", {}, element("code", {text:a.proposal ? proposal(a.proposal) : c.completed === c.attempted ? "Varied across trials" : "Incomplete trials"}),
        i && a.changed != null ? element("small", {text:a.changed ? "Changed" : "Unchanged"}) : null),
      element("td", {}, element("strong", {text:totals[a.status]}),
        element("small", {text:c.matched + " of " + score + " scored trials matched" +
          (score < c.attempted ? " · " + (c.attempted - score) + " missing or unscored" : "")})));
  });
  const scores = (check.token_paths || []).map(path => {
    const chart = targetTokenScores(path);
    chart.open = focusOpening && path.conditions.length === rows.length && path.conditions.every(c => c.first_lower === 0);
    return chart;
  });
  return element("section", {className:"panel target-check"},
    element("span", {className:"label", text:"Test against the declared outcome"}),
    element("h2", {text:focusOpening ? "Compare the opening token across edits" : check.next}),
    question ? element("p", {text:question}) : null,
    element("p", {}, "Target ", element("code", {text:targetCriterion(check.expected,check.match_rule)})),
    ...(focusOpening ? scores : []),
    element("table", {}, element("thead", {}, element("tr", {},
      ...["Condition", "Proposal", "Target result"].map(text => element("th", {scope:"col", text})))),
      element("tbody", {}, ...rows)),
    element("small", {text:check.runner}), ...(focusOpening ? [] : scores));
}

function targetPatternRow(group) {
  const opening = group.miss_signals.opening;
  return rowLink("Every trial missed the target in " + plural(group.missed, "conversation"),
    "Target " + targetCriterion(group.setup.expected, group.setup.rule) + " · " + group.setup.sources.join(" + ") +
      " · " + plural(group.missed_inputs, "distinct request") + " among the misses" +
      (opening ? " · Another opening token led under every edit in " + plural(opening, "conversation") : ""),
    url({targets:1, design:group.design}));
}

const missLabels = {
  opening: "Compare the opening token",
  suffix: "Inspect text after the target",
  other: "Compare conditions where the target opening kept the top score",
  varied: "Compare the different signals",
  unscored: "Check gaps in token scores"
};
const missDescriptions = {
  opening:"Another token outscored the target’s first token in the original and every edited request.",
  suffix:"At least one condition consistently produced the target text followed by more text.",
  other:"The target’s first token reached or tied the highest score in at least one condition.",
  varied:"The score reports or experiments in a conversation gave different signals.",
  unscored:"At least one experiment lacked a score report covering every tested condition."
};

async function targetPatternView() {
  const filters = {...scope, ...(targetDesign ? {design:targetDesign} : {})};
  const examplesFilter = {...filters, ...(missSignal && {miss_signal:missSignal})};
  const back = link("Back to recommendations", url({}));
  if (targetDesign && traceId) {
    const result = await api("target-check", {...examplesFilter, trace:traceId, call:query.get("call"), report:query.get("report")}, true);
    if (!result) {
      pagination(null);
      return panel("Experiment unavailable in this scope", link("Back to target results", url(targetSelection)));
    }
    byId("title").textContent = "Target result";
    byId("summary").textContent = [result.example.thread, fullDate(result.example.started)].join(" · ");
    const download = element("button", {type:"button", text:"Download recorded trials"});
    download.addEventListener("click", () => downloadJson("target-trials.json", result.trials));
    pagination(null);
    const target = declaredTarget(result.check, result.question && result.question.text + (result.question.truncated ? "…" : ""), missSignal === "opening");
    return element("div", {className:"feed"}, link("Back to target results", url(targetSelection)), target,
      element("div", {className:"actions"}, download, link("Open the captured call", url({view:"traces",trace:traceId,call:result.example.call}))));
  }
  const groups = await api("target-patterns", {...filters, offset:targetDesign ? 0 : pageOffset});
  byId("title").textContent = "Recurring target misses";
  if (!targetDesign) {
    byId("summary").textContent = "At least two identified conversations missed the declared target in every scored condition of the same experiment.";
    pagination(groups.next_offset, targetSelection);
    return element("div", {className:"feed"}, back, groups.rows.length ? listing(groups.rows, targetPatternRow) :
      element("p", {className:"empty",text:"No recurring target misses in this scope."}));
  }
  const group = groups.rows[0];
  if (!group) {
    pagination(null);
    return element("div", {className:"feed"},back,element("p",{className:"empty",text:"No recorded experiments for this design in the current scope."}));
  }
  const data = await api("target-examples", {...examplesFilter, offset:pageOffset});
  const resultText = {missed:"Every trial missed",reached:"At least one trial reached the target",incomplete:"Incomplete trials"};
  byId("title").textContent = group.setup.sources.join(" + ");
  byId("summary").textContent = plural(group.conversations, "tested conversation") + " · " + group.setup.runner;
  const counts = [[group.missed,"Missed in every trial"],[group.reached,"Reached in each experiment"],
    [group.varied,"Varied across experiments"],[group.incomplete,"Incomplete"]];
  const expected = group.setup.expected;
  const signals = Object.entries(missLabels).filter(([key]) => group.miss_signals[key] > 0);
  pagination(data.next_offset, targetSelection);
  return element("div", {className:"feed"}, link("Back to recurring target misses", url({targets:1})),
    panel("Did any condition reach the target?",
      element("p", {}, "Target ", element("code", {text:targetCriterion(expected,group.setup.rule)})),
      element("dl", {className:"target-counts"}, ...counts.map(([n,text]) =>
        element("div", {}, element("dt",{text}), element("dd",{text:n})))),
      element("p", {text:"Each conversation counts once. Reached means at least one trial matched in each experiment. A missing or unscored proposal trial makes the conversation incomplete."})),
    signals.length && !missSignal ? element("section", {className:"panel", "aria-label":"Evidence from missed conversations"},
      element("h2", {text:"Where to look next"}), listing(signals, ([key,label]) =>
        rowLink(label, plural(group.miss_signals[key], "conversation") + " · " + missDescriptions[key],
          url({...targetSelection,miss_signal:key})))) : null,
    element("section", {}, element("h2", {text:missSignal ? missLabels[missSignal] : "Experiments"}),
      missSignal && link("Show all experiments",url({...targetSelection,miss_signal:null})),
      missSignal && element("p", {text:missDescriptions[missSignal]}),
      listing(data.rows, row => rowLink(row.question ? row.question.text + (row.question.truncated ? "…" : "") : row.thread,
      [resultText[row.result],row.question && row.thread,row.user,fullDate(row.started)].filter(Boolean).join(" · "),
      url({...targetSelection,trace:row.trace,call:row.call,report:row.report})))));
}

function pendingTokenScores(paths) {
  const heading = element("h2");
  const chart = element("div");
  const select = element("select", {"aria-label":"Scored continuation"}, ...paths.map((p,i) =>
    element("option", {value:i,text:p.continuation})));
  function show() {
    const path = paths[Number(select.value)];
    const [original, ...edits] = path.conditions;
    const gained = edits.filter(row => row.steps[0].log_prob === row.steps[0].top_log_prob);
    heading.textContent = original.first_lower === 0 && gained.length
      ? "Opening token " + JSON.stringify(original.steps[0].text) + " reached the top score in " + gained.length + " of " + plural(edits.length, "edited condition")
      : "Compare token support for " + JSON.stringify(path.continuation);
    const details = targetTokenScores(path, false);
    details.open = true;
    chart.replaceChildren(details);
  }
  select.addEventListener("change", show);
  show();
  return element("section", {className:"panel target-check"}, element("span", {className:"label",text:"Scored before a proposal"}), heading,
    element("label", {}, "Continuation ", select), chart);
}

function renderComparison(report) {
  const labels = {unchanged:"Identical input tokens", changed:"Different input tokens", unrenderable:"Renderer rejected the edited request"};
  return element("section", {className:"panel", "aria-label":"Model input rendering"},
    element("h2", {text:report.unchanged ? "Check fields that left the model input unchanged" : "Field removal checks"}),
    element("p", {text:report.renderer + " · " + plural(report.baseline_tokens,"original input token")}),
    ...report.fields.map(field => element("p", {}, element("code", {text:field.pointer}),
      " · " + labels[field.status] + (field.input_tokens == null ? "" : " (" + field.input_tokens + " tokens)"))),
    report.unchanged ? element("p", {text:"Inspect how your chat template serializes the fields with identical tokens. Each trial removed one field before rendering."}) : null);
}

function pendingPreview() {
  const output = element("section", { className: "feed", "aria-live": "polite" });
  const input = element("input", { type: "file", accept: "application/json,.json" });
  let sequence = 0;
  async function show(load, followUp = null) {
    const currentLoad = ++sequence;
    output.textContent = "Checking the request…";
    try {
      const report = await load();
      if (currentLoad !== sequence) return;
      const current = [], findings = [], probes = new Map();
      const targets = report.target_checks || [];
      const prefixReviews = report.prefix_reviews || [];
      const prefixReports = new Set(prefixReviews.map(r => r.report));
      const prefixTrials = new Set(prefixReviews.flatMap(r => r.runs.map(run => run.report)));
      const linked = new Set(targets.flatMap(c => (c.token_paths || []).map(p => p.report)));
      const standalone = (report.continuation_paths || []).filter(paths => !linked.has(paths[0].report));
      const experiment = report.history?.experiment;
      const dispatchCards = report.recommendations.flatMap(({detail}) => detail.sandbox_dispatch ? [sandboxDispatch(detail.sandbox_dispatch)] : []);
      const dispatchHistory = priorDispatch(report.history, dispatchCards.length > 0);
      const sourceGap = report.recommendations.find(row => row.kind === "source_gap");
      for (const { kind, detail, action } of report.recommendations) {
        if (kind === "source_gap" || kind === "tool_scope" || kind === "render_gap") continue;
        if (kind === "shadow" && experiment?.status === "ready" && experiment.edited_request_sha256 === detail.edited_request_sha256 &&
            experiment.interventions[0].context_id === detail.sources[0].id &&
            experiment.proposal_tests.some(test => test.report === detail.report_id && test.runner === detail.runner)) continue;
        const card = recommendationDetail(kind, detail, action);
        if (kind === "candidate" && (prefixReports.has(detail.report_id) || linked.has(detail.report_id) || standalone.some(paths => paths[0].report === detail.report_id))) {
          findings.push(element("details", {},element("summary",{text:"Compare candidate margins for " + sourceName(detail.sources[0])}),card));
        } else if (kind === "candidate") {
          if (!probes.has(detail.report_id)) {
            const group = element("div", { className: "feed" }, card);
            probes.set(detail.report_id, group);
            current.push(group);
          } else {
            probes.get(detail.report_id).append(element("details", {}, element("summary", {
              text: "Inspect the edit to " + sourceName(detail.sources[0]) }), card));
          }
        } else if (kind === "shadow" && targets.some(check => check.report_id === detail.report_id && check.edits.length > 1)) {
          findings.push(element("details", {}, element("summary", {text:"Inspect the edit to " + sourceName(detail.sources[0])}), card));
        } else if (kind === "shadow") current.push(card);
        else findings.push(card);
      }
      const checked = experiment?.total_proposal_tests > 0;
      const testedTop = checked && report.prefill?.ranked[0]?.context_id === experiment.interventions[0].context_id &&
        report.prefill.ranked[0].request_sha256 === experiment.edited_request_sha256;
      const history = priorEdits(report.history, current.length > 0 || Boolean(report.prefill));
      const coverage = report.scope_coverage;
      const sources = report.source_coverage;
      const gap = coverage && (!coverage.declarations || coverage.known_values < coverage.declarations);
      const opening = report.prefill?.ranked[0]?.opening_changed !== undefined;
      output.replaceChildren(...[
        element("p", { className: "label", text: plural(report.captured_inputs, "captured input") +
          " · " + plural(report.total, "finding") + (report.prefill ? " · First-token test attached" : "") }),
        report.render_check && renderComparison(report.render_check),
        pendingToolScope(report.tool_scope, report.tool_map, report.recommendations.find(r => r.kind === "tool_scope")),
        overlappingAnchors(report.anchor_overlap),
        sources && element("section",{className:"panel","aria-label":"Required source coverage"},
          element("h2",{text:(sources.matched ? "Sources captured for " : "Check sources for ") + sources.name}),
          requiredSourceList(sources),
          sourceGap && sourceMismatchEvidence(sourceGap.detail),
          sourceGap && element("p",{text:sourceGap.action.next})),
        gap && element("section",{className:"panel","aria-label":"Rule capture gap"},
          element("h2",{text:"Check capture for " + coverage.scope.key}),
          element("p",{text:coverage.declarations ? coverage.known_values + " of " + coverage.declarations +
            " declarations supplied a value in scope “" + coverage.scope.scope + "”." :
            "No captured declaration matched scope “" + coverage.scope.scope + "”."}),
          element("p",{text:"Check the rule name and source declarations in the request builder."})),
        !prefixReviews.length && dispatchHistory,
        ...prefixReviews.flatMap((r,i) => [prefixReview(r,report.example_question,i === 0 ? followUp : null),
          blockInvestigation([{review:r,index:r.reversals[0] || 1}])]),
        ...dispatchCards,
        opening && openingComparison(report.prefill, report.example_question),
        ...targets.map(check => opening || prefixTrials.has(check.report_id) ? element("details", {className:"panel"}, element("summary", {text:"Review the declared target"}),
          declaredTarget(check,report.example_question)) : declaredTarget(check, report.example_question)), ...standalone.map(paths => prefixReports.has(paths[0].report) ?
          element("details", {className:"panel"},element("summary",{text:"Inspect individual token probabilities"}),pendingTokenScores(paths)) : pendingTokenScores(paths)),
        ...report.unverified_probes.map((issue) => element("p", { className: "empty", text: issue })),
        report.prefill ? routingComparison(report.prefill) : null,
        checked ? history : null,
        report.prefill && !opening && !(testedTop && report.prefill.ranked.length === 1) ? firstTokenPreview(report.prefill, false, testedTop) : null,
        ...current, prefixReviews.length && dispatchHistory, report.screen_suggestion ? screenReview(report.screen_suggestion) : null,
        checked ? null : history, prefixReviews.length ? null : followUp, ...findings,
        report.total > 15 ? element("p", { className: "label", text: "Showing the first 15 recommendations." }) : null,
        !gap && !report.render_check && !standalone.length && !targets.length && !report.total && !report.prefill && !checked && !report.screen_suggestion ? element("p", { className: "empty", text: "No supported issues or measured shifts in this request." }) : null].filter(Boolean));
    } catch (error) {
      if (currentLoad === sequence) output.textContent = error.message;
    }
  }
  input.addEventListener("change", () => {
    const file = input.files[0];
    if (!file) return;
    show(async () => {
      if (file.size > 2 * 1024 * 1024) throw new Error("Choose a JSON request under 2 MiB.");
      const submitted = JSON.parse(await file.text());
      const envelope = submitted.attributes ? { request: submitted } : submitted;
      const response = await fetch("/api/sidecar/preview", { method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify(envelope) });
      const result = await response.json();
      if (!response.ok) throw new Error(result.message);
      return result;
    });
  });
  const policy = element("button", { type: "button", text: "Try a policy conflict" });
  policy.addEventListener("click", () => show(() => api("preview-example", { scenario: "policy" }),
    element("div", {className:"actions"},
      link("Compare edits at a supplied answer prefix", url({view:"preview", scenario:"prefix-before"}), "action"),
      link("Compare instructions using a text classifier", url({view:"preview", scenario:"screen"})))));
  const routing = element("button", { type: "button", text: "Try a measured tool choice" });
  const prefillLink = link("Check the same edits before the first token", url({ view: "preview", scenario: "prefill" }), "action");
  const routingLinks = element("div", {className:"actions"},
    link("Inspect proposals under a stock-only scope",url({view:"preview",scenario:"routing-scope"}),"action"), prefillLink,
    link("Test instruction text and available tools", url({view:"preview",scenario:"routing-test"}), "action"));
  routing.addEventListener("click", () => show(() => api("preview-example", { scenario: "routing" }), routingLinks));
  const history = element("button", { type: "button", text: "Check a new request against earlier edits" });
  history.addEventListener("click", () => {
    const scores = element("button", { type: "button", text: "Compare with model scores for this request" });
    scores.addEventListener("click", () => show(() => api("preview-example", { scenario: "scored-history" })));
    show(() => api("preview-example", { scenario: "prior-only" }), element("div", {className:"actions"},
      link("See the tested edit on this request", url({view:"preview",scenario:"checked-history"})), scores));
  });
  const scenario = query.get("scenario");
  const exchange = scope.workflow === "exchange-agent" &&
    link("Check the next exchange",url({view:"preview",scenario:"exchange-approval"}),"action");
  const windowQuestions = [["window-earlier", "First question"], ["window-followup", "Follow-up"], ["window-unopened", "Unopened journal"], ["window-fresh", "Fresh question"]];
  const questions = element("nav", {className:"actions", "aria-label":"Journal questions"},
    ...windowQuestions.map(([key, label]) => key === scenario ? element("strong", {text:label, "aria-current":"page"}) :
      link(label, url({view:"preview", scenario:key}))));
  if (["exchange-approval", "exchange-measured"].includes(scenario)) {
    const measured = scenario === "exchange-measured";
    show(() => api("preview-example",{scenario}), measured ? null :
      link("Inspect saved tool-name scores and proposals", url({view:"preview",scenario:"exchange-measured"}), "action"));
    return element("div", {className:"feed"}, output,
      link(measured ? "Back to pending request" : "Check another request",
        url({view:"preview",scenario:measured ? "exchange-approval" : null})));
  } else if (scenario === "prefix-before" || scenario === "prefix-checked") {
    const before = scenario === "prefix-before";
    show(() => api("preview-example", {scenario}), link(before ? "Compare with freely generated answers" : "See the scores before generation",
      url({view:"preview",scenario:before ? "prefix-checked" : "prefix-before"}),"action"));
  } else if (scenario === "opening-before" || scenario === "opening-checked") {
    show(() => api("preview-example", {scenario}), scenario === "opening-before"
      ? link("Check the complete proposals",url({view:"preview",scenario:"opening-checked"}),"action")
      : link("See the pre-generation evidence",url({view:"preview",scenario:"opening-before"})));
  } else if (scenario === "window-before") {
    show(() => api("preview-example", {scenario}),
      link("Compare with generated answers",url({view:"preview",scenario:"window-earlier"}),"action"));
  } else if (windowQuestions.some(([key]) => key === scenario)) {
    show(() => api("preview-example", {scenario}), element("div",{className:"feed"}, questions,
      scenario === "window-earlier" ? link("See the scores before running proposal trials",url({view:"preview",scenario:"window-before"})) : null));
  } else if (["screen", "scored-history", "checked-history", "warranty", "routing", "policy", "history", "prefill", "routing-test", "routing-scope"].includes(scenario)) {
    const next = scenario === "screen" ? link("Test a 45-day answer across questions", url({view:"preview", scenario:"window-earlier"}), "action") :
      scenario === "prefill" ? link("Compare the measured tool proposals", url({ view: "preview", scenario: "routing" }), "action") :
      scenario === "routing" ? routingLinks : scenario === "checked-history" ?
        link("Compare the two source edits on this request", url({view:"preview",scenario:"scored-history"}), "action") : null;
    show(() => api("preview-example", { scenario }), next);
  }
  return element("div", { className: "feed" }, panel("Review an assembled request",
    element("label", { text: "Load a preflight request or a request bundled with probe results" }, input),
    element("div", { className: "actions" }, ...(exchange ? [exchange] : [history, routing, policy]))), output);
}

function traceDetail({ trace, findings, activation_reports, prefill_issues = {}, target_checks = {}, prefix_reviews = {} }) {
  const selectedCall = query.get("call");
  const selectedSpan = query.get("span");
  const spans = new Map((trace.spans || []).map((span) => [span.id, span]));
  const calls = new Map(trace.calls.map((call) => [call.span_id || call.id, call]));
  const children = new Map();
  for (const span of spans.values()) {
    const parent = spans.has(span.parent_span_id) ? span.parent_span_id : null;
    if (!children.has(parent)) children.set(parent, []);
    children.get(parent).push(span);
  }
  const initial = selectedCall || selectedSpan || trace.calls[0]?.id;
  const call = trace.calls.find((entry) => entry.id === initial || entry.span_id === initial);
  const span = spans.get(call?.span_id || initial);
  const seen = new Set();
  const rows = [];
  function visit(sibling, depth) {
    if (seen.has(sibling.id)) return;
    seen.add(sibling.id);
    const entry = calls.get(sibling.id);
    rows.push({ span: sibling, entry, depth });
    for (const child of children.get(sibling.id) || []) visit(child, depth + 1);
  }
  for (const root of children.get(null) || []) visit(root, 0);
  for (const rest of spans.values()) visit(rest, 0);
  for (const [id, entry] of calls) if (!seen.has(id)) rows.push({ span: { id }, entry, depth: 0 });

  byId("title").textContent = "Trace";
  const thread = JSON.stringify([trace.source, trace.organization_id, trace.user_id, trace.conversation_id]);
  byId("summary").replaceChildren(trace.id, ...(trace.conversation_id ?
    [" · ", link(trace.conversation_id, url({ view: "threads", thread, ...(call && {focus_trace:trace.id,focus_call:call.id}) }))] : []));
  const matching = findings.filter((item) => item.call_id === call?.id);
  const focus = matching.find((item) => item.group === query.get("finding"));
  const detailItems = focus ? [focus, ...matching.filter((item) => item !== focus)] : matching;
  const evidence = detailItems.map((item) => panel(findingTitle(item.kind, item.detail),
    tag(item.kind), element("p", { text: findingDetail(item.kind, item.detail) }), visual(item.kind, item.detail),
    element("p", { className: "label", text: "Evidence at " + (call.span_id || call.id) }),
    ["replay","baseline_check"].includes(item.kind) ? link(item.kind === "baseline_check" ? "Inspect unchanged-request trials" : "Open trial comparison", advancedHref(trace, call, item), "action") :
      item.kind === "conflict" || item.kind === "duplicate" ? prepareEdit(trace, call, item) : null));
  const capture = call?.context.filter((item) => item.availability === "included") || [];
  const unavailable = call ? call.context.length - capture.length : 0;
  const inputs = (call?.context || []).map((item) => element("details", {}, element("summary", { text: sourceName(item) + version(item.source_version) }),
    element("p", { className: "label", text: item.evidence.field + (item.evidence.message_index == null ? "" : " · message " + item.evidence.message_index) }),
    item.text == null ? element("p", { text: item.availability === "redacted" ? "Input was redacted" : "Input text unavailable" }) : element("pre", { text: item.text })));
  const detail = panel(call?.model || span?.name || (call ? "Inference span" : "Span"),
    element("p", { className: "label", text: span?.id || call?.id || "No recorded span selected" }),
    call ? element("p", { text: capture.length ? plural(capture.length, "captured input") : "Captured inputs unavailable" }) : null,
    unavailable ? element("p", { className: "label", text: plural(unavailable, "input") + " missing or redacted" }) : null,
    call && prefill_issues[call.id] ? element("p", { text: "Unverified first-token report · " + prefill_issues[call.id] }) : null,
    ...inputs,
    ...(call?.decisions || []).map((decision) => element("details", {},
      element("summary", { text: decision.label }), element("pre", { text: JSON.stringify(decision.outcome, null, 2) }))),
    call ? link(activation_reports ? "Open detailed investigation and token measurements" :
      "Open detailed investigation", advancedHref(trace, call)) : null);
  return element("div", { className: "detail" }, panel("Span tree", listing(rows, ({ span: row, entry, depth }) =>
    rowLink("↳ ".repeat(Math.min(depth, 6)) + (row.name || entry?.model || row.id),
      entry ? (entry.turn_id ? "Turn " + entry.turn_id : "Captured call") : "Parent span",
      url({ view, trace: trace.id, ...(entry ? { call: entry.id } : { span: row.id }) }), entry?.id === initial || row.id === initial))),
  element("div", { className: "feed" }, ...(prefix_reviews[call?.id] || []).map(review => prefixReview(review)), ...(target_checks[call?.id] || []).map(check => declaredTarget(check)), ...evidence, focus ? compareForm(focus.group) : null, detail));
}

async function render(refresh = false) {
  let body;
  if (view === "traces" && traceId) {
    body = traceDetail(await api("trace", { id: traceId }));
  } else if (view === "threads" && threadId) {
    body = await conversationView();
  } else if (view === "findings") {
    if (sourceHistoryKey) {
      body = await sourceHistory();
    } else if (prefixCohorts) {
      body = await prefixCohort();
    } else if (openingStudy && recommendationId && factorId && !traceId) {
      body = await openingCoverageView();
    } else if (conflictDesign && recommendationId && factorId) {
      body = await conflictExamplesView();
    } else if (conflictCoverage && recommendationId && factorId) {
      body = await conflictCoverageView();
    } else if (ranking && !recommendationId) {
      body = await rankingView();
    } else if (targetPatterns) {
      body = await targetPatternView();
    } else if (recommendationId && jointSelection) {
      const selection = {recommendation:recommendationId, factor:factorId, ...jointSelection};
      const data = await api("occurrences", {...scope, group:recommendationId, factor:factorId, ...jointSelection, offset:pageOffset});
      byId("title").textContent = jointPatterns[jointSelection.joint_pattern] || "Joint edit comparisons";
      byId("summary").textContent = "Open a reported comparison to see all four conditions.";
      body = element("div", {className:"feed"}, link("Back to recommendation", url({recommendation:recommendationId, factor:factorId})),
        data.rows.length ? listing(data.rows, row => rowLink(row.thread || "Request without a conversation ID",
          [factorName(row.detail.sources), fullDate(row.observed)].join(" · "),
          url({recommendation:recommendationId, factor:factorId, trace:row.trace, call:row.call,
            decision:row.detail.measurement.location.decision_id, replay:row.detail.measurement.replay_id}))) :
          element("p", {className:"empty", text:"No matching comparisons in this scope."}));
      pagination(data.next_offset, selection);
    } else if (recommendationId && factorId && dispatchSelection && !traceId) {
      body = await dispatchConversations();
    } else if (recommendationId) {
      const filters = { ...scope, ...(threadId && {thread:threadId}), ...measuredExample, group: recommendationId, ...(factorId && { factor: factorId }) };
      const [cards, occurrences] = await Promise.all([api("findings", filters),
        api("occurrences", { ...filters, offset: pageOffset })]);
      if (cards.rows.length) {
        const card = cards.rows[0];
        const dispatch = card.evidence.detail.sandbox_dispatch;
        const dispatchSummary = !traceId && dispatch ? await api("dispatch-summary",{...filters,
          factor:card.evidence.detail.factor,design:dispatch.design}) : null;
        body = findingReview(card,occurrences,dispatchSummary?.rows);
        if (dispatchSelection) body.prepend(link("Back to dispatch outcomes",url({recommendation:recommendationId,factor:factorId,...dispatchSelection})));
        if (threadId || cards.rows[0].evidence.thread_key) {
          const key = threadId || cards.rows[0].evidence.thread_key;
          const conversationLink = link(threadId ? "Back to conversation" : "See this call in its conversation",
            url({view:"threads",thread:key,focus_trace:cards.rows[0].evidence.trace_id,focus_call:cards.rows[0].evidence.call_id}));
          if (threadId) body.prepend(conversationLink);
          else body.append(conversationLink);
        }
        if (screenedPattern && screenedPairs) body.prepend(link("Back to flagged pair", url(screenSelection)));
        if (query.has("from_coverage")) body.prepend(link("Back to conflict test coverage",
          url({recommendation:recommendationId,factor:factorId,conflict_coverage:query.get("from_coverage"),offset:query.get("from_offset")})));
        if (comparisonResult) body.prepend(link("Back to compared conversations", url({proposal_result:comparisonResult,coverage_factor:comparisonScope})));
        if (outcomeCategory) body.prepend(link("Back to conversations", url({level:outcomeLevel, ...cohortMode, cohort_factor:cohortFactor})));
      }
      pagination(occurrences.next_offset, { recommendation: recommendationId, factor: factorId, level:outcomeCategory && outcomeLevel, ...cohortMode, cohort_factor:cohortFactor, proposal_result:comparisonResult, coverage_factor:comparisonScope, ...measuredExample });
    } else if (screenedPairs) {
      const detail = Boolean(screenedPattern);
      const selection = detail ? {pattern:screenedPattern, relation:screenedRelation} : {};
      const [groups, examples] = await Promise.all([api("screened-pairs", {...scope, ...selection, offset:detail ? 0 : pageOffset}),
        detail ? api("screened-pair-examples", {...scope, ...selection, offset:pageOffset}) : null]);
      const data = examples || groups;
      const group = detail && groups.rows[0];
      byId("title").textContent = group ? pairName(group.sources) : "Flagged source pairs";
      byId("summary").textContent = detail ? "Calls awaiting a source edit appear first. Open a comparison to see its trials. Each call shows up to two examples per source for changed and matching proposals." :
        "A classifier flagged these instructions for possible conflict or overlap. Choose a pair to see where an edit changed the answer and where a test is still missing.";
      body = element("div", {className:"feed"}, link(detail ? "Back to flagged pairs" : "Back to recommendations", url(detail ? {screens:1} : {})),
        group ? sourceOutcomes(group) : null,
        detail ? element("div", {className:"feed"}, ...data.rows.map(pairExample)) : listing(data.rows, screenedRow),
        data.rows.length ? null : element("p", {className:"empty",text:"No flagged source pairs in this scope."}));
      pagination(data.next_offset, screenSelection);
    } else if (dispatchOverview) {
      body = await dispatchDesigns();
    } else if (withoutFindings) {
      byId("title").textContent = "Conversations without a finding";
      byId("summary").textContent = "These conversations have captured calls. When a text classifier flags a source pair, you can inspect the texts and prepare an experiment here.";
      const data = await api("conversations-without-findings", {...scope, offset:pageOffset});
      body = element("div", {className:"feed"}, link("Back to recommendations", url({})),
        ...data.rows.map(captureReview), data.rows.length ? null : element("p", {className:"empty", text:"No conversations match this filter."}));
      pagination(data.next_offset, {coverage:"without-findings"});
    } else if (Object.hasOwn(comparisonLabels, comparisonResult)) {
      const selection = {proposal_result:comparisonResult, coverage_factor:comparisonScope};
      const filters = {...scope, result:comparisonResult, offset:pageOffset};
      if (comparisonScope) filters.factor = comparisonScope;
      const data = await api("proposal-conversations", filters);
      byId("title").textContent = comparisonLabels[comparisonResult];
      byId("summary").textContent = comparisonResult === "no_pair" ?
        "These findings have no comparable pair of completed proposals. Open a finding to see what was captured." :
        comparisonResult === "both" ? "Each conversation has an edit that changed a proposal and a comparison that preserved one. The inputs, edits or test conditions can differ." :
        "Open a comparison to see what the model produced with the original and edited inputs.";
      body = element("div", {className:"feed"}, link("Back to coverage", url({factor:comparisonScope}) + "#proposal-coverage"),
        ...data.rows.map(row => panel(row.thread, element("p", {className:"label", text:[row.org,row.user,fullDate(row.latest)].filter(Boolean).join(" · ")}),
          element("div", {className:"actions"}, ...Object.entries(row.examples).map(([outcome,e]) =>
            link(({changed:"See a changed proposal",held:"See a matching proposal",no_pair:"Inspect the finding"})[outcome],
              url({...selection, factor:e.factor, recommendation:e.group, trace:e.trace, call:e.call, decision:e.decision,
                replay:e.replay, report:e.report, context:e.context}) + witnessAnchor(e.witness)))))),
        data.rows.length ? null : element("p", {className:"empty", text:"No conversations in this scope."}));
      pagination(data.next_offset, selection);
    } else if (outcomeCategory) {
      byId("title").textContent = outcomeCategory.title;
      byId("summary").textContent = outcomeCategory.description;
      if (outcomeGrouping !== "conversation" && !cohortFactor) {
        const data = await api("outcome-factors", {...scope, ...cohortMode, level:outcomeLevel, offset:pageOffset});
        body = element("div", {className:"feed"}, cohortNavigation(),
          data.rows.length ? element("section", {className:"cohort-list"},
            element("h2", {text:(editedSources ? "Edits that changed a proposal" : "Sources in " + plural(data.rows[0].cohort_size, "conversation"))}),
            element("p", {text:editedSources ? "Each row counts conversations where that edit changed a generated answer or tool proposal in at least one completed comparison. A conversation can appear in several rows." : "Each group lists the sources in a finding. A conversation can appear in more than one group."}), listing(data.rows, cohortRow)) :
            element("p", {className:"empty", text:"No source groups have this evidence level in the current scope."}));
        pagination(data.next_offset, {level:outcomeLevel, by:outcomeGrouping});
      } else {
        const filters = {...scope, ...cohortMode, level:outcomeLevel, offset:pageOffset};
        if (cohortFactor) filters.cohort_factor = cohortFactor;
        const data = await api("conversation-outcomes", filters);
        body = element("div", {className:"feed"}, cohortNavigation(),
          cohortFactor && data.rows.length ? element("h2", {text:factorName(data.rows[0].sources)}) : null,
          data.rows.length ? listing(data.rows, row => rowLink(row.thread,
            [!cohortFactor && factorName(row.sources), row.witness?.tool_changed == null ? kindLabels[row.kind] :
              (row.witness.tool_changed ? "Tool selection changed" : "Proposal changed with the same tool selection"),
              row.level === 3 && (row.unchanged_pair ? "Paired proposals stayed the same" : "Score evidence only for this finding"), row.user, fullDate(row.observed)].filter(Boolean).join(" · "),
            url({level:outcomeLevel, ...cohortMode, cohort_factor:cohortFactor, factor:row.factor, recommendation:row.group, trace:row.trace, call:row.call,
              decision:row.decision, report:row.report, replay:row.replay, context:row.context}) + witnessAnchor(row.witness))) :
            element("p", {className:"empty", text:"No conversations have this evidence level in the current scope."}));
        pagination(data.next_offset, {level:outcomeLevel, ...cohortMode, cohort_factor:cohortFactor});
      }
    } else if (factorId && query.has("variation")) {
      const data = await api("replay-variations", { ...scope, factor: factorId, ...positionDesign, offset: pageOffset });
      byId("title").textContent = "When does the edit matter?";
      byId("summary").textContent = "The same source edit had different effects on distinct requests. Each replay scored its own recorded outcome.";
      if (positionPair) {
        const row = data.rows.find((item) => item.position_pair === positionPair && item.latest_position_result);
        byId("title").textContent = "Source-position test";
        byId("summary").textContent = row ? factorName(row.changed_example.sources) + " · Positions " +
          row.changed_example.fragment_order.position + " and " + row.equal_example.fragment_order.position +
          " of " + row.changed_example.fragment_order.total : "The selected comparison is unavailable in this scope.";
        body = element("div", { className: "feed" }, link("Back to source factor", url({ factor: factorId })),
          row ? positionExperiment(row) : null);
        pagination(null);
      } else {
        body = element("div", { className: "feed" }, link("Back to source factor", url({ factor: factorId })),
          ...data.rows.map(variationComparison),
          !data.rows.length ? element("p", { className: "empty", text: "No contrasting replay results in this scope." }) : null);
        pagination(data.next_offset, { factor: factorId, variation: 1 });
      }
    } else if (factorId && query.has("order")) {
      const data = await api("order-tests", { ...scope, factor: factorId, offset: pageOffset });
      const result = data.rows[0];
      byId("title").textContent = "Source order";
      byId("summary").textContent = result ? "Swapping the two source messages changed a single-edit match count in " +
        result.differing + " of " + plural(result.total, "matched comparison") + "." : "No matched orders in this scope.";
      body = element("div", { className: "feed" }, link("Back to source factor", url({ factor: factorId })),
        result ? element("p", { text: "Each cell shows trials matching the recorded outcome. Repeat the swap on fresh requests to see whether the pattern holds." }) : null,
        ...data.rows.map(orderComparison));
      pagination(data.next_offset, { factor: factorId, order: 1 });
    } else if (factorId) {
      const filters = { ...scope, factor: factorId };
      const [data, impact, orders, variations, factors] = await Promise.all([api("findings", { ...filters, offset: pageOffset }),
        api("impact", filters), api("order-tests", filters), api("replay-variations", filters), api("factors", filters)]);
      const test = factors.rows[0]?.position_test;
      byId("title").textContent = data.rows.length ? factorName(data.rows[0].evidence.detail.sources) : "Source factor";
      byId("summary").textContent = "Each recommendation below uses its own comparison and outcome.";
      body = element("div", { className: "feed" }, link("All source factors", url({explore:1})),
        data.rows.length ? proposalCoverage(impact, factorId) : null,
        test ? link("Review the source-position test", positionLink(factorId, test), "action") : null,
        variations.rows.length ? link("Compare where the edit mattered", url({ factor: factorId, variation: 1 }), "action") : null,
        orders.rows.length ? link("Compare the two source orders", url({ factor: factorId, order: 1 }), "action") : null,
        ...data.rows.map(findingCard), !data.rows.length ? element("p", { className: "empty", text: "No evidence in this scope." }) : null);
      pagination(data.next_offset, { factor: factorId });
    } else if (query.has("cards")) {
      const data = await api("findings", {...scope,offset:pageOffset});
      body = element("div",{className:"feed"},link("Back to recommendations",url({})), ...data.rows.map(findingCard));
      pagination(data.next_offset,{cards:1});
    } else if (!query.has("explore") && !pageOffset && location.hash !== "#proposal-coverage") {
      const [impact, cards] = await Promise.all([api("impact", scope), api("findings", scope)]);
      if (!cards.rows.length && !Object.keys(scope).length) {
        const health = await api("health");
        const {traces, inference_spans: calls} = health.coverage;
        byId("title").textContent = traces ? "Telemetry received" : "Connect your workflow";
        const next = !traces ? "Configure your workflow’s OpenTelemetry batch exporter, then send one model request." :
          !calls ? "Set gen_ai.operation.name on a model span so the receiver can identify the inference." :
          health.content_capture ? "Findings appear when captured sources disagree, repeat, or match another supported check." :
          "This workspace retains metadata. To investigate a source conflict, use a separate capture-enabled database with redacted staging inputs.";
        byId("content").replaceChildren(element("section",{className:"panel","aria-label":"Receiver setup"},
          element("p",{text:next}),
          !traces && element("pre",{},element("code",{text:"export OTEL_EXPORTER_OTLP_TRACES_PROTOCOL=http/protobuf\n" +
            "export OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=" + health.otlp_http_endpoint})),
          !traces && link("Inspect a sample exchange request",url({view:"preview",scenario:"exchange-approval"})),
          traces > 0 && element("p",{text:plural(traces,"trace")+" · "+plural(calls,"inference span")}),
          traces > 0 && link("Threads",url({view:"threads"})),
          element("p",{},link("Check receiver status","/api/sidecar/health"))));
        byId("content").removeAttribute("data-signature");
        return;
      }
      const signature = JSON.stringify([impact,cards]);
      if (refresh && byId("content").dataset.signature === signature) return;
      byId("content").dataset.signature = signature;
      const existing = byId("content").querySelector(".canvas-home");
      if (existing) {
        existing.updateEvidence(impact, cards);
        return;
      }
      const partners = await api("chat-config");
      body = investigationCanvas(impact, cards, partners.models);
    } else {
      const [impact, factors, screens, targets, prefixes] = await Promise.all([api("impact", scope), api("factors", { ...scope, offset: pageOffset }),
        pageOffset ? Promise.resolve({rows:[]}) : api("screened-pairs", scope),
        pageOffset ? Promise.resolve({rows:[]}) : api("target-patterns", scope),
        pageOffset ? Promise.resolve({rows:[]}) : api("prefix-groups",scope)]);
      const cohort = prefixes.rows[0];
      const comparisons = cohort?.matched_questions > 1 ? await api("prefix-reference-pairs",{...scope,design:cohort.design}) : {rows:[]};
      const comparison = comparisons.rows[0];
      const signature = JSON.stringify([impact, factors, screens, targets, prefixes, comparison]);
      if (refresh && byId("content").dataset.signature === signature) return;
      byId("content").dataset.signature = signature;
      const losses = !pageOffset && impact.ranked_reference_losses;
      const opening = !pageOffset && impact.opening_gap;
      const lower = !pageOffset && impact.lower_edit_changes;
      const pattern = !pageOffset && impact.conflict_pattern;
      const gap = !pageOffset && impact.conflict_gap;
      const measuredFactor = factors.rows.some(f => ["routing_repeats","routing","order","replacement","joint_proposal","joint","variation","replay","shadow"].includes(f.signal));
      const prefixLead = cohort && (comparison || cohort.restored || cohort.lost || (cohort.needs_trial && !measuredFactor) || (cohort.conversations > 1 && (cohort.exceptions || cohort.confirmed))) &&
        !losses && !opening && !lower && !pattern && !gap;
      const featured = !prefixLead && !losses && !opening && !lower && !pattern && !gap && !pageOffset && factors.rows[0];
      const remaining = featured ? factors.rows.slice(1) : factors.rows;
      body = element("div", {className:"overview"},
        losses ? investigation(url({ranking:"all",quality:"ranked_lost"}),
          "Inspect " + plural(losses,"call") + " where the ranked edit lost a reference match",
          "Compare each original answer with the answer after the source edit.") :
        opening ? investigation(url({opening:1,recommendation:opening.group,factor:opening.factor}),
          "Review " + plural(opening.affected,"conversation") + " where the top opening token held and the proposal changed",
          opening.tested + " of " + plural(opening.scored,"scored conversation") + " had a completed proposal comparison for an edit that kept the opening token on top.") :
        lower ? investigation(url({ranking:"all",review:"lower_changed"}),
          "Inspect " + plural(lower,"call") + " where a lower-ranked edit changed the proposal",
          plural(impact.lower_edit_conversations,"linked conversation") + ". The edit with the largest first-token shift kept the original proposal.") :
        pattern ? conflictPatternCard(pattern) :
        gap ? investigation(url({recommendation:gap.group,factor:gap.factor,conflict_coverage:gap.untested ? "untested" : "partial"}),
          "Compare the conflicting sources in " + plural(gap.untested+gap.partial,"conversation"),
          factorName(gap.conflict.sources) + " · " + gap.complete + " of " + plural(gap.conversations,"conversation") + " with both source edits compared at every conflicting call.") :
        prefixLead ? prefixInvestigation(cohort,comparison) :
        featured ? element("section", {"aria-label":"Recommended investigation"}, factorRow(featured, true)) : null,
        impactPanel(impact),
        cohort && !prefixLead ? element("details", {className:"panel"},element("summary",{text:"Source edits at supplied prefixes"}),
          listing([cohort],prefixGroupRow),link("All scored prefixes",url({prefixes:1}))) : null,
        targets.rows.length ? element("section", {className:"factors"}, element("h2", {text:"Recurring target misses"}),
          listing(targets.rows.slice(0,1), targetPatternRow), targets.rows.length > 1 ? link("Review all recurring misses",url({targets:1})) : null) : null,
        screens.rows.length ? element("section", {className:"factors"}, element("h2", {text:"Flagged source pairs"}),
          listing(screens.rows.slice(0,1), screenedRow), screens.rows.length > 1 ? link("Review all flagged pairs", url({screens:1})) : null) : null,
        remaining.length ? element("section", {className:"factors"}, element("h2", {text:featured ? "Other source factors" : "Source factors"}),
          listing(remaining, factor => factorRow(factor))) : null,
        !factors.rows.length && !screens.rows.length && !targets.rows.length && !cohort ? element("p", {className:"empty", text:"No recommendations in this scope."}) : null);
      pagination(factors.next_offset);
    }
  } else {
    const data = await api(view, { ...scope, ...(threadId && {thread:threadId}), offset: pageOffset });
    if (data.rows.length) body = view === "traces" ? traceRows(data.rows) : listing(data.rows, (item) => rowLink(item.thread,
      [item.org, item.user, plural(item.trace_count, "trace"), fullDate(item.start)].filter(Boolean).join(" · "), url({ view, thread: item.id })));
    pagination(data.next_offset,{thread:threadId});
  }
  byId("content").replaceChildren(body || element("p", { className: "empty", text: view === "findings" ?
    "No recommendations in this scope. Cards appear as supported evidence arrives." : "No captured records in this scope." }));
  if (view === "threads") byId("selected-call")?.focus();
}

async function status() {
  const health = await api("health", { ...scope, ...(threadId && { thread: threadId }) });
  const { coverage } = health;
  byId("environment").textContent = health.demo ? "Demo workspace" : "Live workspace";
  byId("status").textContent = [
    health.demo ? "Saved demo evidence" : plural(coverage.traces, "captured trace"),
    coverage.with_inputs + "/" + coverage.inference_spans + " inference spans with inputs",
    coverage.with_rules + "/" + coverage.inference_spans + " with declared rules",
    coverage.with_request + "/" + coverage.inference_spans + " with replayable request captured",
    health.queue_drops ? plural(health.queue_drops, "queue drop") : null,
  ].filter(Boolean).join(" · ");
  if (coverage.traces) byId("status").append(" · ",link("Threads",url({view:"threads"})));
  return health;
}

async function main() {
  const hash = new URLSearchParams(location.hash.slice(1));
  if (hash.get("v") === "1" && ["conversation", "decisions", "timeline", "patterns", "compare", "pending"].includes(hash.get("view"))) {
    const params = new URLSearchParams();
    if (hash.has("a_run")) {
      params.set("run", hash.get("a_run"));
      if (hash.has("b_run")) params.set("other", hash.get("b_run"));
    } else if (hash.has("thread")) params.set("thread", hash.get("thread"));
    else if (hash.has("run")) params.set("run", hash.get("run"));
    else params.set("legacy", "1");
    location.replace("/details?" + params + location.hash);
    return;
  }
  const views = { findings: "Recommendations", preview: "Before inference", traces: "Traces", threads: "Threads" };
  byId("navigation").replaceChildren(...Object.entries(views).filter(([key]) => ["findings", "preview"].includes(key)).map(([key, label]) => link(label, url({ view: key }), null, view === key || (key === "findings" && view !== "preview"))));
  byId("title").textContent = views[view];
  byId("summary").textContent = { findings: "",
    preview: "Check an assembled request and any supplied tests before committing a decision.",
    traces: "Open a trace to inspect its spans and captured sources.",
    threads: "Open a conversation to read its calls and findings in order." }[view];
  if (view === "preview") {
    byId("filters").hidden = true;
    byId("capture").hidden = true;
    byId("content").replaceChildren(pendingPreview());
    return;
  }
  const form = byId("filter-form");
  byId("reset").href = url({ view }, {});
  byId("filter-count").textContent = Object.keys(scope).length ? " · " + Object.keys(scope).length : "";
  byId("scope").replaceChildren(...Object.entries(scope).map(([key, value]) => element("span", { className: "chip", "data-filter":key, text: filterNames[key] + " · " + value })));
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    const values = Object.fromEntries([...new FormData(form)].filter(([, value]) => value.trim()).map(([key, value]) => [key, value.trim()]));
    if (!values.meta_key || !values.meta_value) {
      delete values.meta_key;
      delete values.meta_value;
    }
    location.href = url({ view, source_key:sourceHistoryKey, unthreaded_trace:query.get("unthreaded_trace"), explore:query.get("explore"), cards:query.get("cards"), trace: traceId, thread: threadId, call: query.get("call"),
      span: query.get("span"), finding: query.get("finding"), factor: factorId, order: query.get("order"), variation: query.get("variation"),
      dispatches:dispatchOverview ? 1 : null,
      ...dispatchSelection,
      ...(prefixCohorts && {prefixes:1,...Object.fromEntries(["design","compare_refs","matched","pair_setup","ref_edit",
        "edit_index","edit_outcome","reference","ref_set","ref_outcome","ref_prediction","needs_trial","verdict","source","unnamed","report"]
        .filter(k => query.has(k)).map(k => [k,query.get(k)]))}),
      opening:openingStudy ? 1 : null,slot:openingSlot,opening_context:query.get("opening_context"),ranking, quality, review, conflict_design:conflictDesign, conflict_coverage:conflictCoverage,from_coverage:query.get("from_coverage"),pair: positionPair, ...positionDesign, ...jointSelection, recommendation: recommendationId, by:query.get("by"), level:outcomeCategory && outcomeLevel, cohort_factor:cohortFactor, proposal_result:comparisonResult, coverage_factor:comparisonScope, coverage:withoutFindings ? "without-findings" : null, ...screenSelection, ...targetSelection, ...(targetPatterns && traceId ? {report:query.get("report")} : {}), ...measuredExample }, values);
  });
  const health = await status();
  const meta = form.elements.namedItem("meta_key");
  for (const key of health.metadata_keys) meta.add(element("option", { value: key, text: key }));
  if (scope.meta_key && !health.metadata_keys.includes(scope.meta_key)) meta.add(element("option", { value: scope.meta_key, text: scope.meta_key }));
  for (const [key, value] of Object.entries(scope)) form.elements.namedItem(key).value = value;
  await render();
  if (location.hash === "#proposal-coverage") {
    const coverage = byId("content").querySelector(".proposal-coverage");
    if (coverage) { coverage.open = true; coverage.querySelector("summary").focus(); }
  }
  if (location.hash === "#proposal-pair" && (measuredExample.replay || measuredExample.report) && measuredExample.trace) {
    const pair = byId("content").querySelector(".response-pair, .mask-comparison");
    if (pair) {
      const details = pair.closest("details");
      if (details) {
        details.open = true;
        details.querySelector("summary").focus();
      }
    } else {
      const joint = byId("content").querySelector("[data-joint-edit]");
      if (joint) { joint.tabIndex = -1; joint.focus(); }
    }
  }
  if (hash.has("arm") && measuredExample.replay) {
    const arm = [...byId("content").querySelectorAll("[data-replay-arm]")].find(node => node.dataset.replayArm === hash.get("arm"));
    if (arm) {
      arm.tabIndex = -1;
      arm.focus();
    }
  }
  if (location.hash.startsWith("#proposal-check=") && measuredExample.trace && measuredExample.call) {
    const target = new URLSearchParams(location.hash.slice("#proposal-check=".length));
    const check = [...byId("content").querySelectorAll("[data-proposal-check]")].find(node =>
      node.dataset.proposalCheck === target.get("report") && node.dataset.context === target.get("context"));
    if (check) {
      check.closest("details").open = true;
      check.tabIndex = -1;
      check.focus();
    }
  }
  const setup = /^#routing-setup-([0-3])$/.exec(location.hash);
  if (setup && measuredExample.trace && measuredExample.call) {
    const row = byId("content").querySelector('[data-routing-setup="' + setup[1] + '"]');
    if (row) {
      row.tabIndex = -1;
      row.focus();
    }
  }
  let refreshing = false;
  if (view === "findings" && !sourceHistoryKey && !factorId && !recommendationId && !comparisonResult && !withoutFindings && !screenedPairs && !prefixCohorts && !targetPatterns && !ranking && !outcomeCategory && !health.demo) window.setInterval(() => {
    if (!refreshing && !document.hidden && !byId("content").contains(document.activeElement)) {
      refreshing = true;
      Promise.all([status(), render(true)]).catch((error) => { byId("error").textContent = error.message; })
        .finally(() => { refreshing = false; });
    }
  }, 15000);
}

main().catch((error) => { byId("error").textContent = error.message; });
