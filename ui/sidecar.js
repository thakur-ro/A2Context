"use strict";

const byId = (id) => document.getElementById(id);
const query = new URLSearchParams(location.search);
const view = ["traces", "threads", "preview"].includes(query.get("view")) ? query.get("view") : "findings";
const filterNames = { after: "From", before: "Before", org: "Organization", user: "User",
  workflow: "Workflow", meta_key: "Metadata key", meta_value: "Metadata value" };
const scope = Object.fromEntries(Object.keys(filterNames).filter((key) => query.has(key)).map((key) => [key, query.get(key)]));
const kindLabels = { prefill: "First-token test", replacement: "Replacement comparison", conflict: "Rule conflict", duplicate: "Exact repetition", replay: "Verified replay",
  candidate: "Candidate scores", shadow: "Proposal trials", token: "Token measurements" };
const recommendationId = query.get("recommendation");
const factorId = query.get("factor");
const traceId = query.get("trace");
const measuredExample = Object.fromEntries(["trace", "call", "replay", "context", "routing_design", "report"]
  .filter((key) => recommendationId && query.has(key)).map((key) => [key, query.get(key)]));
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

function url(params, filters = scope) {
  const search = new URLSearchParams(filters);
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined || value === null || value === "") search.delete(key);
    else search.set(key, value);
  }
  return search.size ? "?" + search : "/";
}

async function api(path, params = {}) {
  const response = await fetch("/api/sidecar/" + path + "?" + new URLSearchParams(params));
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
  const singles = (detail.single_edits || []).map((arm) => ["Edit " + arm.id, arm.summary]);
  const arms = [["Baseline", m.baseline, detail.baseline_answer], ...singles,
    [detail.edit === "joint" ? "Joint edit" : "Source edit", m.variant, detail.edited_answer], ["Control edit", m.control]];
  return element("div", { className: "comparison" }, ...arms.filter(([, data]) => data).map(([label, data, answer]) =>
    element("div", {}, element("strong", { text: label }),
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

function visual(kind, detail, rows = []) {
  if (kind === "prefill") return firstTokenPreview(detail, true);
  const [label, body] = kind === "replacement" ? ["Trials matching the recorded outcome", replacementVisual(detail)] :
    kind === "conflict" ? ["Captured declarations", sourceCells(detail.sources, detail.values)] :
    kind === "replay" || kind === "shadow" ? ["Matches by trial arm", replayTrials(detail)] :
    kind === "candidate" ? [detail.alternatives.map((entry) => entry.id).join(" vs ") + " at a fixed prefix", candidateVisual(detail)] :
    kind === "token" ? ["Change in output-token log probability", tokenVisual(detail.landmark)] :
    rows.length ? ["Occurrences by day (UTC)", trend(rows)] : ["Repeated sources", sourceCells(detail.sources)];
  return element("div", { className: "visual" }, element("span", { className: "label", text: label }), body);
}

function findingTitle(kind, detail) {
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
  if (detail.effect_pp === 0 && detail.response_pair?.changed) return "The proposal changed with an equal match rate";
  if (detail.joint_only) return "Only the joint edit changed the match rate";
  const amount = Math.abs(detail.effect_pp);
  const points = amount < 0.01 ? "less than 0.01 percentage points" :
    plural(Number(amount.toFixed(2)), "percentage point");
  if (detail.effect_pp === 0) return "Measured match rates were equal";
  if (detail.edit === "control") return "Target and control differed by " + points;
  return "Match rate " + (detail.effect_pp > 0 ? "rose by " : "fell by ") + points;
}

function findingDetail(kind, detail) {
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

const outcomeLevels = [
  {key:"proposal", title:"Generated proposal changed", description:"Every listed conversation has a source edit that changed a complete replayed answer or tool proposal."},
  {key:"rate", title:"Replay match rate changed", description:"A change in the fraction of proposals meeting a scoring rule is the strongest recorded result here."},
  {key:"score", title:"Model score shifted", description:"A first-token, candidate or output-token score moved. These conversations have no verified proposal change or match-rate shift in this scope."},
  {key:"tested", title:"Tested without a measured shift", description:"A verified comparison had equal scores or match rates. A matched rate alone can conceal differences in the generated text."},
  {key:"inputs", title:"Captured inputs only", description:"Declared rules conflict or input text repeats. These conversations have no measured comparison among their findings in this scope."},
];
const outcomeLevel = Number(query.get("level"));
const outcomeCategory = outcomeLevels[outcomeLevel - 1];

function impactPanel(impact) {
  return element("section", { className: "impact", "aria-label": "Conversation impact" },
    element("div", {}, element("h2", { text: "Conversations with findings" }),
      element("strong", { className: "impact-number", text: impact.conversations }),
      element("p", { text: plural(impact.factors, "source factor") + (impact.unlinked_requests ? " · " + plural(impact.unlinked_requests, "additional request") + " without a conversation ID" : "") })),
    element("div", { className: "impact-breakdown" },
      element("p", {text:"Strongest evidence per conversation"}),
      ...outcomeLevels.flatMap(({key, title}, index) => impact.evidence_counts[key] ? [
        element("a", {className:"impact-outcome", href:url({level:index + 1})},
          element("strong", {text:impact.evidence_counts[key]}), element("span", {text:title}),
          element("span", {"aria-hidden":"true",text:"→"}))] : []),
      replayCoverage(impact)));
}

function positionLink(factor, test) {
  return url({ factor, variation: 1, pair: test.pair, group: test.group,
    baseline_trials: test.baseline_trials, edit_trials: test.edit_trials });
}

function positionReadout(result) {
  const { present, replaced } = result;
  if (present.compared < 2 || replaced.compared < 2) return "Position test has incomplete trials";
  return "Changed proposals · source present " + present.changed + "/2 · source replaced " + replaced.changed + "/2";
}

function factorRow(factor, featured = false) {
  const signals = { routing_repeats:"Repeats disagreed", routing: factor.routing_inputs > 1 ? "Routing tests on " + plural(factor.routing_inputs, "input") : "Instruction and tool test", prefill: "First-token sensitivity", variation: "Edit effects varied across inputs", order: "Results varied with source order", replacement: "Results varied across wordings", joint: "Only the joint edit changed the match rate", flip: "Choice flipped", replay: "Replay outcome shifted", shadow: "Proposal comparison",
    token: "Token score shifted", score: "Preference shifted", conflict: "Conflicting rules",
    duplicate: "Repeated input", dialogue: "Repeated dialogue", equal: "Equal match rates" };
  const step = { routing_repeats:"Check why the same edit gave different results", routing: factor.routing_inputs > 1 ? "See where the source edit mattered" : "Compare the four routing setups", prefill: "Review which edits shifted the first token", variation: "Compare where this edit changed the outcome", order: "Compare the two source orders", replacement: "Compare the two replacement texts", joint: "Inspect the joint edit",
    flip: "Check which edit flipped the choice", replay: "Compare the original and edited outcomes",
    shadow: "Check the proposed answers or tool calls", token: "Locate the shift along the answer",
    score: "Check which alternative gained ground", conflict: "Compare the conflicting declarations",
    duplicate: "Check whether one copy can be removed", dialogue: "Check the repeated message positions",
    equal: "Review the equal match rates" };
  const test = factor.signal === "variation" && factor.position_test;
  const isolated = test && test.result.present.changed === 2 && test.result.replaced.compared === 2 && test.result.replaced.changed === 0;
  const headline = isolated ? "Moving this source changed both proposals" : "Review the source-position test";
  const name = factorName(factor.sources);
  const counts = [factor.org, plural(factor.conversations, "identified conversation"), plural(factor.requests, "request"),
    plural(factor.recommendations, "recommendation")].filter(Boolean).join(" · ");
  const disputed = factor.disputed_routing;
  const href = disputed ? url({factor:factor.id, recommendation:disputed.group, routing_design:disputed.design}) :
    test ? positionLink(factor.id, test) : url({ factor: factor.id, ...(factor.signal === "order" && { order: 1 }), ...(factor.signal === "variation" && { variation: 1 }) });
  return element("a", { className: featured ? "factor-row featured" : "factor-row",
    href },
    element("div", {},
      featured ? element("span", { className: "first-step", text: test ? "Measured position test" : "Recommended next step" }) : null,
      element("strong", { text: featured ? (test ? headline : step[factor.signal]) : name }),
      featured ? element("span", { className: "source-names", text: name }) : null,
      element("span", { className: "row-extra", text: counts })),
    element("span", { className: ["conflict", "duplicate", "dialogue"].includes(factor.signal) ? "basis" : "basis measured",
      text: test ? positionReadout(test.result) : signals[factor.signal] }),
    element("span", { className: "arrow", "aria-hidden": "true", text: "→" }));
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
      const href = URL.createObjectURL(new Blob([JSON.stringify(plan, null, 2)], { type: "application/json" }));
      const download = element("a", { href, download: "position-test-" + plan.pair.slice(0, 12) + ".json" });
      download.click();
      setTimeout(() => URL.revokeObjectURL(href), 1000);
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
      repeatedRouting(card) ? element("p", {text:routingCoverage(card)}) : kind === "prefill" ? prefillFollowup(card) : element("p", { text: findingDetail(kind, detail) }),
      element("p", { className: "label", text: plural(card.occurrences, "observation") + " · " + plural(card.traces, "request") }),
      link("Review recommendation", url({ factor: factorId, recommendation: card.id }), "action")),
    element("div", { className: "suggestion" }, element("span", { className: "label", text: "Next experiment" }),
      element("p", { text: detail.joint_only ? "Test the two sources together" : card.action.title })));
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

function pagination(next, params = {}) {
  byId("pagination").replaceChildren(...[
    pageOffset > 0 ? link("Previous", url({ view, ...params, offset: Math.max(0, pageOffset - 15) || null })) : null,
    next == null ? null : link("Next", url({ view, ...params, offset: next })),
  ].filter(Boolean));
}

function traceRows(rows) {
  return listing(rows, (item) => rowLink(item.workflow || item.source, [item.id, fullDate(item.start),
    item.thread ? "Thread " + item.thread : "", item.with_inputs + "/" + item.calls + " inference spans with inputs"].filter(Boolean).join(" · "),
  url({ view: "traces", trace: item.id })));
}

function advancedHref(trace, call, item) {
  const m = item?.detail.measurement;
  const hash = new URLSearchParams({ v: "1", view: "decisions", run: trace.id, call: call.id });
  const decision = m?.location?.decision_id || call.decisions?.[0]?.id;
  if (decision) hash.set("decision", decision);
  if (m) {
    hash.set("replay", m.replay_id);
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
    const percent = new Intl.NumberFormat(undefined, { style: "percent", maximumSignificantDigits: 4 });
    return element("div", {},
      element("p", { text: "For the original token " + JSON.stringify(selected.fragment) + ", the probability changed from " +
        percent.format(Math.exp(selected.baseline)) + " to " + percent.format(Math.exp(selected.edited)) +
        " after editing the source. Both scores use the original output prefix." }),
      element("p", { text: "At this token, the largest tested decoder-block swap was at block " +
        point.block + " (" + signed(point.block_delta) + " log probability)." }));
  }
  if (kind === "replay" && detail.edit === "joint" && detail.single_edits?.length) {
    return element("div", {},
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

function responsePair(pair) {
  if (!pair) return null;
  const proposal = p => p.kind === "tool_call" || pair.original.label !== pair.edited.label || pair.original.kind !== pair.edited.kind
    ? [p.kind, p.label, p.outcome_preview].join(" · ") : p.outcome_preview;
  return element("details", {className:"response-pair"}, element("summary", {text:pair.changed ? "Compare the changed proposals" : "Compare the matching proposals"}),
    element("div", {className:"comparison"}, ...[[pair.original,"Original"],[pair.edited,"Edited"]].map(([p, name]) =>
      element("div", {}, element("strong", {text:name}), element("code", {text:proposal(p)})))),
    (pair.original.truncated || pair.edited.truncated) ? element("p", {className:"label", text:"Previews show the first 160 characters. The comparison uses the complete proposals."}) : null,
    element("p", {className:"label", text:pair.baseline_trials + " original and " + pair.edited_trials +
      " edited trials. All trials completed and agreed within each setup."}));
}

function recommendationDetail(kind, detail, action, rows = []) {
  const interval = detail.measurement?.interval || detail.measurement?.difference_95_interval_pp;
  const uncertainty = interval ? "95% interval for the difference: " + interval.lower + " to " + interval.upper + " percentage points." :
    ({ replacement: "These counts come from one request. Sampling variation can also change the observed counts.",
       replay: "The supplied trials have no estimated interval for this comparison.",
       shadow: "The supplied proposal trials have no estimated interval for this comparison.",
       candidate: "These scores compare two continuations. Free generation can choose a third answer.",
       token: "This comparison holds the output prefix fixed at each position.",
       conflict: "The disagreement is visible in the inputs. Its effect on the answer still needs a test.",
       duplicate: "The text matches exactly. Its effect on the answer still needs a test." })[kind];
  return panel(findingTitle(kind, detail), tag(kind),
    kind === "replacement" ? element("p", { className: "label", text: detail.model + " · " + plural(detail.conditions[0].baseline.attempted, "trial") + " per condition" }) : element("p", { className: "label", text: action.basis }), visual(kind, detail, rows),
    responsePair(detail.response_pair), recommendationContext(kind, detail), element("p", { className: "uncertainty", text: uncertainty }),
    element("details", {}, element("summary", { text: "How this was measured" }),
      element("p", { text: ({
        replacement: "The sidecar compared two replay reports with the same recorded decision, runner, request, edit locations, scoring rule, sampling design and baseline counts. The reports used different replacement texts.",
        candidate: "A model runner scored two fixed continuations from the same prefix. This can run before inference or in a later experiment. The sidecar verifies the request and edit. The runner supplies the timing.",
        shadow: "A model runner proposed answers or tool calls for the baseline and edited requests. The sidecar checked the request and recomputed the match rates.",
        token: "An instrumented model runner replaced a source and swapped decoder-block activations on a fixed output path. This view shows where the original token’s score moved under those interventions.",
        replay: "A runner replayed the original request and the specified edits. The sidecar checked the request digests and counted the returned answers or tool calls that matched the recorded outcome.",
        conflict: "The sidecar compared the rule key, scope and declared value on captured inputs. This check can run as soon as the workflow assembles a request.",
        duplicate: "The sidecar found the same text at separate input locations. This check can run before the workflow sends the request."
      })[kind] })),
    element("h3", { text: detail.joint_only ? "Keep one source available while testing removal" : action.title }),
    element("p", { text: detail.joint_only ? "Repeat both single edits and the joint edit with a second replacement. A different wording can change the result." : action.next }),
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

function review(card, occurrences) {
  const { trace_id, call_id, detail } = card.evidence;
  byId("title").textContent = "Recommendation";
  byId("summary").textContent = card.action.basis;
  if (card.kind === "prefill") {
    const titles = { tools:"Tool definitions changed the edit’s effect", instruction:"The instruction changed the edit’s effect",
      combined:"The edit depended on the routing setup", constant:"The edit’s effect held across setups", incomplete:"Some routing trials are missing" };
    const repeated = repeatedRouting(card);
    const oneRepeated = card.routing_profiles?.length === 1 && card.routing_profiles[0].inputs === 1 && card.routing_profiles[0].repeat_counts;
    const witnesses = oneRepeated && card.routing_profiles[0].repeat_evidence;
    const contrast = witnesses && (witnesses.tool || witnesses.proposal);
    const headline = contrast ? (witnesses.tool ? "Tool selection" : "The full proposal") + " changed in " +
      contrast.changed_runs + " of " + (contrast.changed_runs + contrast.held_runs) + " completed runs in one setup" : card.action.title;
    byId("title").textContent = oneRepeated ? "Repeat trials on one input" : repeated ? "Routing results across requests" : titles[detail.routing_test?.pattern] || "First-token sensitivity";
    byId("summary").textContent = repeated ? routingCoverage(card) : prefillSummary(card.prefill_counts);
    return element("div", { className: "feed" }, link("Back to source factor", url({ factor: factorId || detail.factor })),
      panel(headline, repeated ? null : prefillFollowup(card), element("p", {text:contrast ?
        "Open a changed and an unchanged run below. Check the original and edited proposals before deciding whether to change the source." : card.action.next})),
      measuredExample.trace && detail.routing_test ? link("All routing results in this scope", url({factor:factorId || detail.factor, recommendation:card.id, routing_design:detail.routing_test.design})) : null,
      repeated ? routingPatterns(card) : routingComparison(detail),
      !oneRepeated && element("p", { text: "These calls share the tested edits, model, runtime and request settings. Other user text can differ." }),
      ...occurrences.rows.map((row, i) => element("details", { className: "panel", ...(!repeated && i === 0 && { open: "" }) },
        element("summary", { text: (row.thread || "Request without a conversation ID") + " · " + findingTitle("prefill", row.detail) }),
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
    card.kind === "replay" ? link("Open trial comparison", advancedHref({ id: trace_id }, { id: call_id }, { detail })) : null);
  return element("div", { className: "feed" },
    link("Back to source factor", url({ factor: factorId || detail.factor })),
    recommendationDetail(card.kind, detail, card.action, card.trend),
    measuredExample.trace ? link("See this recommendation across requests", url({ recommendation: card.id, factor: factorId })) : null,
    card.threads > 1 ? compareForm(card.id) : null, evidence,
    panel("Where this appeared", ...occurrences.rows.map((row) => element("details", { className: "occurrence" },
      element("summary", { text: (row.thread || "Request without a conversation ID") + " · " + fullDate(row.observed) }),
      element("p", { text: findingTitle(row.kind, row.detail) }), visual(row.kind, row.detail),
      element("div", { className: "actions" },
        row.thread_key ? link("Open conversation", url({ view: "threads", thread: row.thread_key })) : null,
        link("Open span " + row.call, url({ view: "traces", trace: row.trace, call: row.call, finding: card.id })))))));
}

function priorEdits(history, hasCurrentScores) {
  if (history?.status !== "checked") return null;
  const [top] = history.edits;
  const limit = history.truncated ? " The lookup used the latest 512 matching observations." : "";
  if (!top) return element("p", { className: "label", text: "No comparable source edit on two distinct earlier inputs in this scope." + limit });
  function trial(edit) {
    const e = edit.example;
    const measurement = { location: { decision_id: e.decision }, replay_id: e.report, context_id: e.context };
    return advancedHref({ id: e.trace }, { id: e.call }, { detail: { measurement } });
  }
  const e = top.example;
  const rows = history.edits.map(edit => element("tr", {},
    element("th", { scope: "row" }, link(edit.example.source, trial(edit))),
    element("td", { text: edit.example.replacement || "(Removed text)" }),
    element("td", { text: edit.lower }), element("td", { text: edit.equal }), element("td", { text: edit.mixed })));
  return element("section", { className: "panel replacement prior-edits" },
    element("span", { className: "label", text: "Evidence from earlier requests" }),
    element("h2", { text: top.lower && hasCurrentScores ? "Earlier replays suggested testing " + e.source :
      top.lower ? "Test " + e.source + " on this request" : "Review earlier source edits" }),
    element("p", { text: "Replacing " + e.source + " with " + JSON.stringify(e.replacement) +
      " reduced the match rate on " + top.lower + " of " + plural(top.requests, "distinct earlier input") + "." }),
    element("p", { text: "These requests shared the model, request settings, source versions and positions. Their counts describe earlier answers." + limit }),
    link("Inspect an earlier replay", trial(top), "action"),
    element("details", {}, element("summary", { text: "Compare tested edits" }),
      element("table", {}, element("thead", {}, element("tr", {}, ...["Source", "Replacement", "Lower", "Equal", "Mixed"].map(text => element("th", { scope: "col", text })))),
        element("tbody", {}, ...rows)),
      element("p", { className: "label", text: "Each count is a distinct earlier input. Mixed means repeat tests on that input disagreed about whether the match rate fell." })));
}

function proposalComparisons(probe) {
  const checks = probe.proposal_checks;
  const proposal = (p) => p.kind === "model_output" ? p.outcome_preview : p.label + " · " + p.outcome_preview;
  if (!checks.length) return element("p", { text: "No completed single-trial proposal comparison is attached for these edits." });
  return element("details", {}, element("summary", { text: "Compare generated proposals" }),
    ...checks.map((check) => {
      const edit = probe.ranked.find((row) => row.context_id === check.context_id);
      return element("section", {}, element("h3", { text: (edit.source || edit.context_id) +
        (check.changed ? " · proposal changed" : " · same proposal") }),
        element("div", { className: "comparison" },
          element("div", {}, element("strong", { text: "Original proposal" }), element("code", { text: proposal(check.original) })),
          element("div", {}, element("strong", { text: "Edited proposal" }), element("code", { text: proposal(check.edited) }))),
        element("p", { className: "label", text: "One completed proposal per condition · " + check.runner }));
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

function firstTokenPreview(probe, recorded = false) {
  const [top] = probe.ranked;
  const tied = probe.ranked.filter((row) => row.js_bits === top.js_bits).length;
  const name = (row) => row.source || row.context_id;
  const bits = (value) => Number(value.toPrecision(3)).toString() + " bits";
  const rows = probe.ranked.map((row) => element("tr", {},
    element("th", { scope: "row", text: name(row), title: row.version || row.context_id }),
    element("td", {}, element("meter", { min: 0, max: top.js_bits || 1, value: row.js_bits, "aria-label": name(row) + " divergence" })),
    element("td", { text: bits(row.js_bits) })));
  return element("section", { className: "panel replacement" },
    element("span", { className: "label", text: recorded ? "Captured first-token measurement" : "Before the first output token" }),
    element("h2", { text: top.js_bits === 0 ? "The tested edits left the first-token distribution unchanged" :
      tied === 1 ? (recorded ? "Largest shift from editing " : "Test the edit to ") + name(top) : plural(tied, "source edit") + " tied for the largest shift" }),
    element("p", { text: "Largest shift across " + plural(rows.length, "tested edit") + " · " + bits(top.js_bits) +
      " of Jensen–Shannon divergence." + (recorded ? "" : " Replay the original and edited requests to see whether the answer or tool call changes.") }),
    tied === 1 && top.before !== undefined ? element("div", { className: "comparison" },
      element("div", {}, element("strong", { text: "Original" }), element("code", { text: top.before })),
      element("div", {}, element("strong", { text: "Edited" }), element("code", { text: top.operation === "remove_json" ? "(Removed)" : top.replacement }))) : null,
    recorded && probe.tool_count !== undefined ? element("p", { className: "label",
      text: "Original request · " + plural(probe.tool_count, "available tool") +
        (probe.tool_names.length ? " · " + probe.tool_names.join(", ") : "") }) : null,
    recorded ? proposalComparisons(probe) : null,
    element("details", {}, element("summary", { text: "Compare source edits" }),
      element("table", {}, element("thead", {}, element("tr", {}, ...["Source", "Relative to largest shift", "Divergence"].map(
        (text) => element("th", { scope: "col", text })))), element("tbody", {}, ...rows)),
      element("p", { className: "label", text: probe.model_id + " · " + probe.model_revision + " · " + probe.runtime })));
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
      for (const { kind, detail, action } of report.recommendations) {
        const card = recommendationDetail(kind, detail, action);
        if (kind === "candidate") {
          if (!probes.has(detail.report_id)) {
            const group = element("div", { className: "feed" }, card);
            probes.set(detail.report_id, group);
            current.push(group);
          } else {
            probes.get(detail.report_id).append(element("details", {}, element("summary", {
              text: "Inspect the edit to " + sourceName(detail.sources[0]) }), card));
          }
        } else if (kind === "shadow") current.push(card);
        else findings.push(card);
      }
      output.replaceChildren(...[
        element("p", { className: "label", text: plural(report.captured_inputs, "captured input") +
          " · " + plural(report.total, "current-request finding") + (report.prefill ? " · First-token test attached" : "") }),
        ...report.unverified_probes.map((issue) => element("p", { className: "empty", text: issue })),
        report.prefill ? routingComparison(report.prefill) : null,
        report.prefill ? firstTokenPreview(report.prefill) : null,
        ...current, priorEdits(report.history, current.length > 0 || Boolean(report.prefill)), followUp, ...findings,
        report.total > 15 ? element("p", { className: "label", text: "Showing the first 15 recommendations." }) : null,
        !report.total && !report.prefill ? element("p", { className: "empty", text: "No supported issues or measured shifts in this request." }) : null].filter(Boolean));
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
  policy.addEventListener("click", () => show(() => api("preview-example", { scenario: "policy" })));
  const routing = element("button", { type: "button", text: "Try a measured tool choice" });
  const prefillLink = link("Check the same edits before the first token", url({ view: "preview", scenario: "prefill" }), "action");
  const routingLinks = element("div", {className:"actions"}, prefillLink,
    link("Test instruction text and available tools", url({view:"preview",scenario:"routing-test"}), "action"));
  routing.addEventListener("click", () => show(() => api("preview-example", { scenario: "routing" }), routingLinks));
  const history = element("button", { type: "button", text: "Check a new request against earlier edits" });
  history.addEventListener("click", () => {
    const scores = element("button", { type: "button", text: "Compare with model scores for this request" });
    scores.addEventListener("click", () => show(() => api("preview-example", { scenario: "scored-history" })));
    show(() => api("preview-example", { scenario: "prior-only" }), scores);
  });
  const scenario = query.get("scenario");
  if (["scored-history", "warranty", "routing", "policy", "history", "prefill", "routing-test"].includes(scenario)) {
    const next = scenario === "prefill" ? link("Compare the measured tool proposals", url({ view: "preview", scenario: "routing" }), "action") :
      scenario === "routing" ? routingLinks : null;
    show(() => api("preview-example", { scenario }), next);
  }
  return element("div", { className: "feed" }, panel("Review an assembled request",
    element("label", { text: "Load a preflight request or a request bundled with probe results" }, input),
    element("div", { className: "actions" }, history, routing, policy)), output);
}

function traceDetail({ trace, findings, activation_reports, prefill_issues = {} }) {
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
    [" · ", link(trace.conversation_id, url({ view: "threads", thread }))] : []));
  const matching = findings.filter((item) => item.call_id === call?.id);
  const focus = matching.find((item) => item.group === query.get("finding"));
  const detailItems = focus ? [focus, ...matching.filter((item) => item !== focus)] : matching;
  const evidence = detailItems.map((item) => panel(findingTitle(item.kind, item.detail),
    tag(item.kind), element("p", { text: findingDetail(item.kind, item.detail) }), visual(item.kind, item.detail),
    element("p", { className: "label", text: "Evidence at " + (call.span_id || call.id) }),
    item.kind === "replay" ? link("Open trial comparison", advancedHref(trace, call, item), "action") :
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
  element("div", { className: "feed" }, ...evidence, focus ? compareForm(focus.group) : null, detail));
}

async function render(refresh = false) {
  let body;
  if (view === "traces" && traceId) {
    body = traceDetail(await api("trace", { id: traceId }));
  } else if (view === "threads" && threadId) {
    const data = await api("traces", { ...scope, thread: threadId, offset: pageOffset });
    byId("title").textContent = data.rows[0]?.thread || "Thread";
    byId("summary").textContent = "Traces in chronological order.";
    body = data.rows.length ? traceRows(data.rows) : null;
    pagination(data.next_offset, { thread: threadId });
  } else if (view === "findings") {
    if (recommendationId) {
      const filters = { ...scope, ...measuredExample, group: recommendationId, ...(factorId && { factor: factorId }) };
      const [cards, occurrences] = await Promise.all([api("findings", filters),
        api("occurrences", { ...filters, offset: pageOffset })]);
      if (cards.rows.length) {
        body = review(cards.rows[0], occurrences);
        if (outcomeCategory) body.prepend(link("Back to conversations", url({level:outcomeLevel})));
      }
      pagination(occurrences.next_offset, { recommendation: recommendationId, factor: factorId, level:outcomeCategory && outcomeLevel, ...measuredExample });
    } else if (outcomeCategory) {
      const data = await api("conversation-outcomes", {...scope, level:outcomeLevel, offset:pageOffset});
      byId("title").textContent = outcomeCategory.title;
      byId("summary").textContent = outcomeCategory.description;
      body = element("div", {className:"feed"}, link("All source factors", url({})),
        data.rows.length ? listing(data.rows, row => rowLink(row.thread,
          [factorName(row.sources), kindLabels[row.kind], row.level === 3 && (row.unchanged_pair ? "Paired proposals stayed the same" : "Score evidence only for this finding"), row.user, fullDate(row.observed)].filter(Boolean).join(" · "),
          url({level:outcomeLevel, factor:row.factor, recommendation:row.group, trace:row.trace, call:row.call,
            report:row.report, replay:row.replay, context:row.context}))) :
          element("p", {className:"empty", text:"No conversations have this evidence level in the current scope."}));
      pagination(data.next_offset, {level:outcomeLevel});
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
      body = element("div", { className: "feed" }, link("All source factors", url({})),
        data.rows.length ? replayCoverage(impact) : null,
        test ? link("Review the source-position test", positionLink(factorId, test), "action") : null,
        variations.rows.length ? link("Compare where the edit mattered", url({ factor: factorId, variation: 1 }), "action") : null,
        orders.rows.length ? link("Compare the two source orders", url({ factor: factorId, order: 1 }), "action") : null,
        ...data.rows.map(findingCard), !data.rows.length ? element("p", { className: "empty", text: "No evidence in this scope." }) : null);
      pagination(data.next_offset, { factor: factorId });
    } else {
      const [impact, factors] = await Promise.all([api("impact", scope), api("factors", { ...scope, offset: pageOffset })]);
      const signature = JSON.stringify([impact, factors]);
      if (refresh && byId("content").dataset.signature === signature) return;
      byId("content").dataset.signature = signature;
      body = element("div", { className: "overview" }, impactPanel(impact),
        element("section", { className: "factors" }, element("h2", { text: "Source factors" }),
          factors.rows.length ? listing(factors.rows, (factor) => factorRow(factor, !pageOffset && factor === factors.rows[0])) :
            element("p", { className: "empty", text: "No recommendations in this scope." })));
      pagination(factors.next_offset);
    }
  } else {
    const data = await api(view, { ...scope, offset: pageOffset });
    if (data.rows.length) body = view === "traces" ? traceRows(data.rows) : listing(data.rows, (item) => rowLink(item.thread,
      [item.org, item.user, plural(item.trace_count, "trace"), fullDate(item.start)].filter(Boolean).join(" · "), url({ view, thread: item.id })));
    pagination(data.next_offset);
  }
  byId("content").replaceChildren(body || element("p", { className: "empty", text: view === "findings" ?
    "No recommendations in this scope. Cards appear as supported evidence arrives." : "No captured records in this scope." }));
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
  byId("summary").textContent = { findings: "See which conversations warrant a closer look, then choose a source to investigate.",
    preview: "Check an assembled request and any supplied tests before committing a decision.",
    traces: "Open a trace to inspect its spans and captured sources.",
    threads: "Browse traces grouped by their recorded conversation." }[view];
  if (view === "preview") {
    byId("filters").hidden = true;
    byId("capture").hidden = true;
    byId("content").replaceChildren(pendingPreview());
    return;
  }
  const form = byId("filter-form");
  byId("reset").href = url({ view }, {});
  byId("filter-count").textContent = Object.keys(scope).length ? " · " + Object.keys(scope).length : "";
  byId("scope").replaceChildren(...Object.entries(scope).map(([key, value]) => element("span", { className: "chip", text: filterNames[key] + " · " + value })));
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    const values = Object.fromEntries([...new FormData(form)].filter(([, value]) => value.trim()).map(([key, value]) => [key, value.trim()]));
    if (!values.meta_key || !values.meta_value) {
      delete values.meta_key;
      delete values.meta_value;
    }
    location.href = url({ view, trace: traceId, thread: threadId, call: query.get("call"),
      span: query.get("span"), finding: query.get("finding"), factor: factorId, order: query.get("order"), variation: query.get("variation"),
      pair: positionPair, ...positionDesign, recommendation: recommendationId, level:outcomeCategory && outcomeLevel, ...measuredExample }, values);
  });
  const health = await status();
  const meta = form.elements.namedItem("meta_key");
  for (const key of health.metadata_keys) meta.add(element("option", { value: key, text: key }));
  if (scope.meta_key && !health.metadata_keys.includes(scope.meta_key)) meta.add(element("option", { value: scope.meta_key, text: scope.meta_key }));
  for (const [key, value] of Object.entries(scope)) form.elements.namedItem(key).value = value;
  await render();
  const setup = /^#routing-setup-([0-3])$/.exec(location.hash);
  if (setup && measuredExample.trace && measuredExample.call) {
    const row = byId("content").querySelector('[data-routing-setup="' + setup[1] + '"]');
    if (row) {
      row.tabIndex = -1;
      row.focus();
    }
  }
  if (view === "findings" && !factorId && !recommendationId && !outcomeCategory && !health.demo) window.setInterval(() => {
    if (!document.hidden && !byId("content").contains(document.activeElement)) {
      Promise.all([status(), render(true)]).catch((error) => { byId("error").textContent = error.message; });
    }
  }, 15000);
}

main().catch((error) => { byId("error").textContent = error.message; });
