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
