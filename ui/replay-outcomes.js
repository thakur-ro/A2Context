"use strict";

const replayOutcomes = new Map();
const outcomeStatuses = {
  match: "Recorded match",
  alternative: "Alternative outcome",
  unscored: "Unscored",
  failed: "Failed",
};

function replayReportKey(location, id) {
  return JSON.stringify([location.run_id, location.call_id, location.decision_id, id]);
}

function indexReplayOutcomes() {
  for (const { replay_index, buckets } of data.replay_outcomes || []) {
    const report = data.replays[replay_index];
    if (report) replayOutcomes.set(replayReportKey(report.location, report.id), { report, buckets });
  }
}

function conditionTrials(report, condition) {
  if (condition === 0) return report.baseline;
  return condition <= report.variants.length ? report.variants[condition - 1].trials : report.pair.trials;
}

function editedCondition(report, contextId) {
  return contextId === undefined ? report.variants.length + 1 :
    report.variants.findIndex(({ intervention }) => intervention.context_id === contextId) + 1;
}

function outcomeCount(bucket, condition) {
  return bucket.counts.find((entry) => entry.condition === condition)?.count || 0;
}

function appendReplayOutcomes(container, location, replayId, contextId) {
  const distribution = replayOutcomes.get(replayReportKey(location, replayId));
  if (!distribution) return;
  const { report, buckets } = distribution;
  const condition = editedCondition(report, contextId);
  const details = node("details", undefined, "replay-outcome-comparison");
  details.append(node("summary", "Compare trial outcomes"));
  details.addEventListener("toggle", () => {
    if (details.open && details.childElementCount === 1) {
      renderOutcomeComparison(details, report, buckets, condition);
    }
  });
  container.append(details);
}

function renderOutcomeComparison(details, report, buckets, condition) {
  const rows = buckets.map((bucket) => ({
    bucket,
    baseline: outcomeCount(bucket, 0),
    edited: outcomeCount(bucket, condition),
  })).filter(({ baseline, edited }) => baseline || edited);
  const order = Object.keys(outcomeStatuses);
  rows.sort((a, b) => order.indexOf(a.bucket.status) - order.indexOf(b.bucket.status)
    || Math.abs(b.edited - b.baseline) - Math.abs(a.edited - a.baseline));

  const table = node("table", undefined, "replay-outcome-table");
  table.createCaption().textContent = replayMatchLabel(report.match_rule) + " · Counts out of attempted trials";
  const heading = table.createTHead().insertRow();
  const editedLabel = condition > report.variants.length ? "Both inputs edited" : "Edited input";
  for (const label of ["Trial outcome", "Baseline", editedLabel]) {
    const header = node("th", label);
    header.scope = "col";
    heading.append(header);
  }
  const body = table.createTBody();
  const more = node("button", "Show more outcomes");
  const editedTrials = conditionTrials(report, condition);
  const appendPage = () => {
    for (const { bucket, baseline, edited } of rows.slice(body.rows.length, body.rows.length + 12)) {
      const row = body.insertRow();
      const outcome = node("th");
      outcome.scope = "row";
      renderOutcomeLabel(outcome, report, bucket);
      row.append(outcome);
      appendOutcomeCount(row.insertCell(), baseline, report.baseline.length);
      appendOutcomeCount(row.insertCell(), edited, editedTrials.length);
    }
    more.hidden = body.rows.length === rows.length;
  };
  more.addEventListener("click", appendPage);
  appendPage();
  const scroll = node("div", undefined, "effects-scroll");
  scroll.append(table);
  details.append(scroll, more);
}

function renderOutcomeLabel(cell, report, { status, example, selection }) {
  cell.append(node("strong", outcomeStatuses[status]));
  const choice = conditionTrials(report, example.condition)[example.trial].choice;
  if (!choice) return;
  cell.append(node("small", [labels[choice.kind] || choice.kind, choice.label].join(" · "), "anchor"));
  const value = outcomePreview(report, choice, selection);
  if (value !== undefined) cell.append(node("pre", value.length > 200 ? value.slice(0, 200) + "…" : value, "replay-outcome-value"));
  const sample = node("details", undefined, "replay-outcome-sample");
  sample.append(node("summary", "Example trial"));
  sample.addEventListener("toggle", () => {
    if (sample.open && sample.childElementCount === 1) {
      const condition = example.condition === 0 ? "Baseline" :
        report.variants[example.condition - 1]?.intervention.context_id || "Joint edit";
      sample.append(node("small", condition + " · Trial " + (example.trial + 1), "anchor"),
        node("pre", readable(choice.outcome)));
    }
  });
  cell.append(sample);
}

function outcomePreview(report, choice, selection) {
  if (selection && typeof selection === "object") {
    return Object.entries(selection).map(([pointer, value]) => pointer + " = " + JSON.stringify(value)).join("\n");
  }
  if (selection !== undefined) return JSON.stringify(selection);
  if (!report.match_rule || report.match_rule.kind === "exact") return readable(choice.outcome);
}

function appendOutcomeCount(cell, count, attempted) {
  cell.textContent = count + "/" + attempted;
  const share = node("meter");
  share.max = attempted;
  share.value = count;
  share.setAttribute("aria-hidden", "true");
  cell.append(share);
}

function appendRecurringOutcomes(card, occurrences) {
  const alternatives = new Map();
  for (const occurrence of occurrences) {
    if (occurrence.difference_pp === null) continue;
    const { location, replay_id, context_id } = occurrence;
    const distribution = replayOutcomes.get(replayReportKey(location, replay_id));
    if (!distribution) continue;
    const { report, buckets } = distribution;
    const condition = editedCondition(report, context_id);
    for (const bucket of buckets) {
      if (bucket.alternative_id === undefined) continue;
      const baseline = outcomeCount(bucket, 0);
      const edited = outcomeCount(bucket, condition);
      if (baseline === edited) continue;
      const choice = conditionTrials(report, bucket.example.condition)[bucket.example.trial].choice;
      if (!alternatives.has(bucket.alternative_id)) {
        alternatives.set(bucket.alternative_id, { report, bucket, choice, gains: new Set(), losses: new Set(), tests: [] });
      }
      const alternative = alternatives.get(bucket.alternative_id);
      (edited > baseline ? alternative.gains : alternative.losses).add(replayDecisionKey(location));
      alternative.tests.push({ occurrence, baseline, edited });
    }
  }
  const recurring = [...alternatives.values()].filter(({ gains, losses }) => gains.size > 1 || losses.size > 1)
    .sort((a, b) => Math.max(b.gains.size, b.losses.size) - Math.max(a.gains.size, a.losses.size));
  if (!recurring.length) return;
  const details = node("details", undefined, "recurring-outcomes");
  details.append(node("summary", countLabel(recurring.length, "recurring alternative")));
  for (const { report, bucket, choice, gains, losses, tests } of recurring) {
    const entry = node("details", undefined, "recurring-outcome");
    const knownThreads = new Set(tests.map(({ occurrence }) => patternThreadKey(occurrence)).filter(Boolean));
    const counts = ["Higher in " + countLabel(gains.size, "decision"), "lower in " + countLabel(losses.size, "decision")];
    if (knownThreads.size) counts.push(countLabel(knownThreads.size, "known thread"));
    const value = outcomePreview(report, choice, bucket.selection);
    const preview = value?.replace(/\s+/g, " ").trim();
    const title = preview && preview.length > 70 ? preview.slice(0, 70) + "…" : preview;
    entry.append(node("summary", [choice.label, title, ...counts].filter(Boolean).join(" · ")));
    entry.addEventListener("toggle", () => {
      if (!entry.open || entry.childElementCount > 1) return;
      entry.append(node("small", labels[choice.kind] || choice.kind, "anchor"));
      if (value !== undefined) entry.append(node("pre", value));
      for (const { occurrence, baseline, edited } of tests) {
        const name = [occurrence.organization_id, occurrence.user_id, occurrence.conversation_id || occurrence.location.run_id]
          .filter(Boolean).join(" / ");
        const counts = baseline + "/" + occurrence.baseline.attempted + " → " + edited + "/" + occurrence.variant.attempted;
        entry.append(occurrenceButton(occurrence.location, name + " · " + counts, occurrence));
      }
    });
    details.append(entry);
  }
  card.append(details);
}
