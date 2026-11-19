"use strict";

const screensByCall = new Map();
const screenLabels = {
  exact_text_reuse: "Exact text reuse",
  possible_conflict: "Possible conflict",
  possible_overlap: "Possible overlap",
};
const screenReasons = {
  no_scorer: "No scorer supplied",
  too_long: "Input exceeds the scorer's token limit",
  unavailable: "Scores unavailable",
};
const screenPercent = new Intl.NumberFormat(undefined, { style: "percent", maximumFractionDigits: 1 });

function contextScreenForCall(run, call) {
  return screensByCall.get(contextCallKey({ run_id: run.id, call_id: call.id }));
}

function renderContextScreen(parent, run, call, open) {
  const screen = contextScreenForCall(run, call);
  if (!screen) return;
  const report = screen.report;
  const flagged = report?.pairs.filter((pair) => pair.suggestion) || [];
  const label = "Context screening · " + (report ? countLabel(flagged.length, "pair") + " to review" : "could not verify");
  evidenceSection(parent, label, open, (section) => {
    if (!report) {
      section.append(node("p", screen.issue), evidence(screen.evidence));
      return;
    }
    const skipped = report.pairs.filter(({ status }) => status === "unscored").length;
    const coverage = report.pairs.length + " of " + report.total_pairs + " pairs screened"
      + (skipped ? " · " + skipped + " without model scores" : "");
    section.append(node("p", coverage, "muted"));
    if (report.scorer) {
      section.append(node("small", [report.scorer.model_id, report.scorer.revision, report.scorer.runtime].join(" · "), "anchor"));
    }
    const sources = new Map(call.context.map((item) => [item.id, item]));
    function appendPair(container, pair) {
      const card = node("details", undefined, "finding");
      const items = pair.sources.map((index) => sources.get(report.sources[index].context_id));
      const sourceNames = items.map((item) => item.source_id || [item.role,
        item.evidence.message_index === null ? item.id : "message " + (item.evidence.message_index + 1),
      ].filter(Boolean).join(" "));
      const status = screenLabels[pair.suggestion] || screenReasons[pair.reason] || "No pair flagged";
      card.append(node("summary", status + " · " + sourceNames.join(" and ")));
      for (const item of items) card.append(contextDetails(item, true));
      if (pair.status === "scored") {
        for (const [direction, scores] of [["First → second", pair.left_to_right], ["Second → first", pair.right_to_left]]) {
          const scoresText = Object.entries(scores).map(([name, value]) => name + " " + screenPercent.format(value)).join(" · ");
          card.append(node("p", direction + " · " + scoresText, "muted"));
        }
      }
      container.append(card);
    }
    flagged.forEach((pair) => appendPair(section, pair));
    const remaining = report.pairs.filter((pair) => !pair.suggestion);
    if (remaining.length) {
      const other = node("details");
      other.append(node("summary", countLabel(remaining.length, "other screened pair")));
      remaining.forEach((pair) => appendPair(other, pair));
      section.append(other);
    }
    section.append(evidence(screen.evidence));
  });
}
