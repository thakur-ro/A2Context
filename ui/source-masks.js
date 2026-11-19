"use strict";

const masksByDecisionInput = new Map();
const masksBySourceItem = new Map();
const visibilityEffectFormat = new Intl.NumberFormat(undefined, {
  maximumFractionDigits: 5, signDisplay: "exceptZero",
});

function visibilityInteraction(baseline, first, second, joint) {
  return (joint - first) - (second - baseline);
}

function largestTokenEffect(scores, measure) {
  let peak = { token: 0, effect: measure(scores[0], 0) };
  for (let token = 1; token < scores.length; token++) {
    const effect = measure(scores[token], token);
    if (Math.abs(effect) > Math.abs(peak.effect)) peak = { token, effect };
  }
  return peak;
}

function sourceMaskMeasurements(report) {
  const measured = new Map(report.sources.filter(({ masked_log_probs }) => masked_log_probs)
    .map(({ context_id, masked_log_probs }) => [context_id, masked_log_probs]));
  const interactions = new Map();
  for (const [index, pair] of (report.pairs || []).entries()) {
    const [first, second] = pair.context_ids.map((id) => measured.get(id));
    const peak = largestTokenEffect(pair.masked_log_probs, (joint, token) =>
      visibilityInteraction(report.baseline_log_probs[token], first[token], second[token], joint));
    for (const id of pair.context_ids) {
      const previous = interactions.get(id);
      if (!previous || Math.abs(peak.effect) > Math.abs(previous.effect)) {
        interactions.set(id, { ...peak, pair: index });
      }
    }
  }
  return [...measured].map(([context_id, scores]) => ({
    location: report.location,
    reportId: report.id,
    model: report.model_id,
    context_id,
    mask: largestTokenEffect(scores, (masked, token) => masked - report.baseline_log_probs[token]),
    interaction: interactions.get(context_id),
  }));
}

function indexSourceMasks(reports) {
  for (const report of reports) {
    if (report.kind !== "source_visibility") continue;
    for (const measurement of sourceMaskMeasurements(report)) {
      const input = { ...measurement.location, context_id: measurement.context_id };
      for (const [index, key] of [
        [masksByDecisionInput, contextReplayKey(input)], [masksBySourceItem, sourceItemKey(input)],
      ]) {
        if (!index.has(key)) index.set(key, []);
        index.get(key).push(measurement);
      }
    }
  }
}

function appendSourceMaskEvidence(parent, measurements) {
  for (const { location, reportId, model, context_id, mask, interaction } of measurements) {
    const add = (label, { token, effect }, focus) => {
      const link = occurrenceButton(location, label + " at token " + (token + 1) + " · " + visibilityEffectFormat.format(effect), {
        activation_id: reportId, visibility_token: token, ...focus,
      });
      link.title = model + " · log probability units";
      parent.append(link);
    };
    add("Largest mask effect", mask, { visibility_source: context_id });
    if (interaction) add("Largest pair interaction", interaction, { visibility_pair: interaction.pair });
  }
}
