let activationPatternBlock = 0;

const activationDateFields = {
  from: "activation-from", through: "activation-through",
  from_b: "activation-from-b", through_b: "activation-through-b",
};
const utcDayNanos = 86_400_000_000_000n;

function activationPeriod(from, through) {
  const start = from ? timestampNanos(from + "T00:00:00Z") : null;
  const end = through ? timestampNanos(through + "T00:00:00Z") : null;
  return {
    start, end: end === null ? null : end + utcDayNanos,
    valid: (!from || start !== null) && (!through || end !== null)
      && (start === null || end === null || start <= end),
    label: from && through ? (from === through ? from : from + " to " + through)
      : from ? "From " + from : through ? "Through " + through : "All dates",
  };
}

function inActivationPeriod(item, period) {
  if (period.start === null && period.end === null) return true;
  const time = timestampNanos(item.started_at);
  return time !== null && (period.start === null || time >= period.start)
    && (period.end === null || time < period.end);
}

function setupActivationDates() {
  const update = () => { linkNotice(); renderPatterns(); };
  element("activation-dates").addEventListener("change", update);
  element("activation-clear-dates").addEventListener("click", () => {
    for (const id of Object.values(activationDateFields)) element(id).value = "";
    element("activation-compare-dates").checked = false;
    update();
  });
}

function writeActivationDateLink(params) {
  const compareDates = element("activation-compare-dates").checked;
  const comparing = compareDates || Boolean(patternComparison.scope);
  if (compareDates) params.set("compare_dates", "1");
  for (const [key, id] of Object.entries(activationDateFields)) {
    const value = element(id).value;
    if (value && (comparing || !key.endsWith("_b"))) params.set(key, value);
  }
}

function restoreActivationDateLink(params) {
  const compareDates = params.get("compare_dates") === "1";
  const comparing = compareDates || Boolean(params.get("scope_b"));
  element("activation-compare-dates").checked = compareDates;
  let valid = true;
  let dated = false;
  for (const [key, id] of Object.entries(activationDateFields)) {
    const value = comparing || !key.endsWith("_b") ? params.get(key) ?? "" : "";
    const input = element(id);
    input.value = value;
    valid &&= input.value === value && input.validity.valid;
    dated ||= Boolean(value);
  }
  element("activation-dates").open = comparing || dated;
  return valid;
}

function readActivationPeriods(secondScope) {
  const comparing = secondScope || element("activation-compare-dates").checked;
  element("activation-compare-dates-control").hidden = secondScope;
  element("activation-period-b").hidden = !comparing;
  const fields = Object.values(activationDateFields).map(element);
  const active = comparing ? fields : fields.slice(0, 2);
  const periods = [activationPeriod(fields[0].value, fields[1].value)];
  if (comparing) periods.push(activationPeriod(fields[2].value, fields[3].value));
  const message = active.some((input) => !input.validity.valid) ? "Enter complete UTC dates."
    : periods.some((period) => !period.valid) ? "The Through date must be on or after the From date." : "";
  element("activation-dates").querySelector("summary").textContent = comparing ? "Date ranges"
    : periods[0].label === "All dates" || message ? "Date range" : "Date range · " + periods[0].label;
  const status = element("activation-date-status");
  status.hidden = !message;
  status.textContent = message;
  return message ? null : periods;
}

function activationDecisionKey({ location }) {
  return JSON.stringify([location.run_id, location.call_id, location.decision_id]);
}

function summarizeActivationEffects(occurrences, block) {
  const decisions = new Map();
  let minimum = Infinity;
  let maximum = -Infinity;
  for (const occurrence of occurrences) {
    const before = occurrence.baseline_margin;
    const after = occurrence.patched_margins[block];
    const delta = after - before;
    minimum = Math.min(minimum, delta);
    maximum = Math.max(maximum, delta);
    const key = activationDecisionKey(occurrence);
    const summary = decisions.get(key) || { lower: true, flipped: true };
    summary.lower &&= delta < 0;
    summary.flipped &&= before > 0 && after < 0;
    decisions.set(key, summary);
  }
  let lower = 0;
  let flipped = 0;
  for (const decision of decisions.values()) {
    lower += Number(decision.lower);
    flipped += Number(decision.flipped);
  }
  return { minimum, maximum, lower, flipped, decisions: decisions.size };
}

function activationGroupsByPeriod(groups, selections, query) {
  return groups.flatMap((group) => {
    const cohorts = selections.map(({ scope, thread, period }) => group.occurrences.filter((item) =>
      inPatternScope(item, scope.organization, scope.user)
      && (!thread || thread === patternThreadKey(item, group))
      && inActivationPeriod(item, period)
    ));
    const sameSource = [group.source_id, group.before, group.after, group.model_id, group.runtime, group.checkpoint.label];
    const matching = cohorts.flat();
    const searchable = [...sameSource, ...matching.map((item) => item.context_id)].join(" ").toLowerCase();
    return matching.length && searchable.includes(query) ? [{ group, cohorts }] : [];
  });
}

function renderActivationPatterns(list, scope, scopes, query) {
  const patterns = data.activation_patterns || [];
  updatePatternThreadFilter(scopedPatternGroups(patterns, scope.organization, scope.user));
  const second = comparisonPatternFilter(scopes, patterns);
  const periods = readActivationPeriods(Boolean(second));
  if (!periods) return;
  const selections = periods.map((period, index) => index && second
    ? { ...second, period }
    : { scope, thread: state.patternThread, period });
  const groups = activationGroupsByPeriod(patterns, selections, query);
  if (!groups.length) {
    list.append(node("p", "No measured answer checkpoints match these filters.", "muted"));
    return;
  }
  const comparing = periods.length === 2;
  const panel = node("article", undefined, "panel pattern activation-patterns");
  panel.append(node("h2", "Recurring activation effects"));
  if (!comparing) panel.append(node("p", activationCoverage(groups.flatMap(({ cohorts }) => cohorts[0])), "count"));
  panel.append(node("p", "Each row compares the same source edit at an exact answer prefix. A lower score means less support for the original token. A flip means the alternative became preferred.", "muted"));
  const layerCount = groups.reduce((largest, { group }) => Math.max(largest, group.layer_count), 0);
  const picker = activationBlockPicker(layerCount);
  if (!Number.isInteger(activationPatternBlock) || activationPatternBlock < 0 || activationPatternBlock >= layerCount) activationPatternBlock = 0;
  picker.select.value = String(activationPatternBlock);
  panel.append(picker.label);
  const scroller = node("div", undefined, "activation-scroll");
  scroller.tabIndex = 0;
  scroller.setAttribute("role", "region");
  scroller.setAttribute("aria-label", "Recurring source effects at the selected block");
  const table = node("table", undefined, "activation-pattern-table");
  const header = table.createTHead().insertRow();
  const labels = comparing ? ["Source edit", "Answer checkpoint", second ? "Scope A" : "Period A", second ? "Scope B" : "Period B"]
    : ["Source edit", "Answer checkpoint", "Change in log odds", "Lower score", "Flipped", "Evidence"];
  const captions = second ? [patternScopeCaption(scope, element("pattern-thread")),
    patternScopeCaption(second.scope, element("pattern-compare-thread"))] : [];
  for (const [index, label] of labels.entries()) {
    const cell = node("th", label);
    cell.scope = "col";
    if (comparing && index >= 2) {
      const period = index - 2;
      if (captions[period]) cell.append(node("small", captions[period], "activation-date-range"));
      cell.append(node("small", periods[period].label, "activation-date-range"),
        node("small", activationCoverage(groups.flatMap(({ cohorts }) => cohorts[period])), "activation-date-range"));
    }
    header.append(cell);
  }
  const body = table.createTBody();
  scroller.append(table);
  const evidence = node("div", undefined, "activation-pattern-evidence");
  function draw() {
    body.replaceChildren();
    evidence.replaceChildren();
    for (const { group, cohorts } of groups) {
      const { checkpoint, occurrences } = group;
      const row = body.insertRow();
      const name = group.source_id || occurrences[0].context_id;
      const heading = node("th", name.replaceAll("-", " "));
      heading.scope = "row";
      heading.append(node("small", group.before + " → " + group.after, "muted"));
      row.append(heading);
      const where = row.insertCell();
      where.append(node("strong", checkpoint.label));
      where.append(node("small", checkpoint.alternatives.join(" vs. ") + " · token " + (checkpoint.output_token + 1)
        + " · prefix "
        + checkpoint.fixed_prefix_token_sha256.slice(0, 8), "muted"));
      const model = node("small", group.model_id + " · " + group.runtime, "activation-pattern-model");
      model.title = "Model revision " + group.model_revision;
      where.append(model);
      if (activationPatternBlock >= group.layer_count) {
        const cell = row.insertCell();
        cell.colSpan = labels.length - 2;
        cell.textContent = "Block unavailable";
        continue;
      }
      for (const [index, selected] of cohorts.entries()) {
        const period = !comparing ? "" : second
          ? [labels[index + 2], captions[index], periods[index].label].join(" · ")
          : labels[index + 2];
        appendActivationPeriod(row, evidence, group, selected, name, period);
      }
    }
  }
  picker.select.addEventListener("change", () => {
    activationPatternBlock = Number(picker.select.value);
    linkNotice();
    draw();
  });
  draw();
  panel.append(scroller, evidence);
  list.append(panel);
}

function activationCoverage(occurrences) {
  const decisions = new Set(occurrences.map(activationDecisionKey));
  const threads = new Set(occurrences.map((item) => patternThreadKey(item)).filter((key) => key !== null));
  return countLabel(threads.size, "identified conversation") + " · " + countLabel(decisions.size, "measured decision");
}

function appendActivationPeriod(row, evidence, group, occurrences, name, period) {
  if (!occurrences.length) {
    row.insertCell().textContent = "No measurements";
    return;
  }
  const result = summarizeActivationEffects(occurrences, activationPatternBlock);
  const minimum = activationMargin.format(result.minimum);
  const range = result.minimum === result.maximum ? minimum : minimum + " to " + activationMargin.format(result.maximum);
  const lower = result.lower + "/" + result.decisions;
  const flipped = result.flipped + "/" + result.decisions;
  const open = node("button", countLabel(result.decisions, "decision"));
  open.type = "button";
  open.setAttribute("aria-label", "Open " + open.textContent + " for " + name + " at " + group.checkpoint.label
    + (period ? " in " + period : ""));
  open.addEventListener("click", () => renderActivationOccurrences(evidence, group, occurrences, name, activationPatternBlock, period));
  if (period) {
    const cell = row.insertCell();
    cell.className = "activation-period-cell";
    cell.append(node("strong", range + " log odds"), node("small", lower + " lower · " + flipped + " flipped", "muted"), open);
  } else {
    row.insertCell().textContent = range;
    row.insertCell().textContent = lower;
    row.insertCell().textContent = flipped;
    row.insertCell().append(open);
  }
}

function renderActivationOccurrences(parent, group, occurrences, name, block, period) {
  parent.replaceChildren();
  const heading = node("h3", name.replaceAll("-", " ") + " at " + group.checkpoint.label);
  heading.tabIndex = -1;
  const note = node("p", [period, group.model_id, group.runtime, "block " + block].filter(Boolean).join(" · "), "muted");
  const first = occurrences[0];
  const report = data.activation_reports.find((item) => item.id === first.report_id
    && activationDecisionKey(item) === activationDecisionKey(first));
  const prefix = report.checkpoints[first.checkpoint_index].answer_prefix;
  const list = node("ul", undefined, "activation-occurrences");
  for (const item of occurrences.slice().sort(compareTimes)) {
    const row = node("li");
    const anchor = occurrenceButton(item.location, item.conversation_id || item.location.run_id, {
      activation_id: item.report_id, checkpoint_index: item.checkpoint_index, block, source_swap: item.context_id,
    });
    const owner = [item.organization_id, item.user_id].filter(Boolean).join(" / ");
    const scores = activationMargin.format(item.baseline_margin) + " → " + activationMargin.format(item.patched_margins[block]) + " log odds";
    row.append(anchor, node("span", [scores, owner].filter(Boolean).join(" · ")));
    const time = Date.parse(item.started_at);
    if (Number.isFinite(time)) {
      const timestamp = node("time", conversationTime.format(new Date(time)) + " UTC", "muted");
      timestamp.dateTime = item.started_at;
      row.append(timestamp);
    }
    list.append(row);
  }
  parent.append(heading, note, node("p", "Spaces appear as · in the fixed answer prefix.", "muted"),
    node("pre", prefix.replaceAll(" ", "·") || "Start of the answer"), list);
  heading.focus();
}
