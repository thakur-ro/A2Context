"use strict";

// The scores on a page share the recorded output prefix at each position.
function tokenPath(card) {
  const {detail} = card.evidence;
  const {landmark, location: measured} = detail;
  const pageSize = 16;
  const fragment = text => text || "Partial character";
  const excerpt = value => value.text + (value.truncated ? "…" : "");
  const percent = new Intl.NumberFormat(undefined, {style:"percent", maximumSignificantDigits:4});
  const region = element("section", {className:"panel answer-path", "aria-label":"Source edits along the answer"});
  const title = element("h2", {text:"How did source edits affect this answer?"});
  const result = element("div", {"aria-live":"polite", text:"Loading token measurements…"});
  region.append(title, result);
  let currentRequest = 0;

  async function load(index, focus = false) {
    const request = ++currentRequest;
    const offset = Math.floor(index / pageSize) * pageSize;
    result.setAttribute("aria-busy", "true");
    try {
      const page = await api("token-path", {...scope, trace:measured.run_id, call:measured.call_id,
        decision:measured.decision_id, report:detail.report_id, group:card.id, factor:detail.factor,
        offset, block:landmark.block});
      if (request !== currentRequest) return;
      const metadata = new Map(page.edits.map(edit => [edit.context_id, edit]));
      const tokens = element("div", {className:"answer-tokens", role:"group", "aria-label":"Recorded output tokens"});
      const readout = element("div", {className:"token-readout"});
      const maximum = Math.max(...landmark.positions.map(p => Math.abs(p.edited - p.baseline)));
      const buttons = page.tokens.map(row => {
        const text = row.fragment.text;
        const button = element("button", {type:"button", text:text.trim() ? excerpt(row.fragment) : (text ? "␣" : "◌"),
          "aria-label":"Token " + (row.output_token + 1) + " · " + JSON.stringify(fragment(text))});
        const strongest = row.sources.reduce((peak, s) => Math.max(peak, Math.abs(s.edited_prompt_log_probability - row.baseline_log_probability)), 0);
        button.style.setProperty("--shift", 100 * strongest / maximum + "%");
        button.addEventListener("click", () => select(row));
        return button;
      });
      function goTo(token) {
        const local = page.tokens.find(row => row.output_token === token);
        if (!local) return load(token, true);
        select(local);
        buttons[page.tokens.indexOf(local)].focus();
      }
      function select(row) {
        page.tokens.forEach((other, i) => buttons[i].setAttribute("aria-pressed", String(other === row)));
        const baseline = row.baseline_log_probability;
        const sources = row.sources.map(s => ({edit:metadata.get(s.context_id), score:s.edited_prompt_log_probability,
          delta:s.edited_prompt_log_probability - baseline, other:s.other_edited_delta})).sort((a,b) => Math.abs(b.delta) - Math.abs(a.delta));
        const ranks = element("ol", {className:"edit-ranks"});
        for (const {edit, score, delta} of sources) {
          const name = sourceName(edit.source);
          const measure = element("div", {className:"edit-result"},
            element("strong", {text:name}), element("span", {text:percent.format(Math.exp(score))}),
            element("span", {className:"edit-delta", text:signed(delta) + " log p"}));
          const jump = edit.peak && edit.peak.token !== row.output_token ? element("button", {type:"button",
            text:"Go to this source’s largest shift · token " + (edit.peak.token + 1)}) : null;
          if (jump) jump.addEventListener("click", () => goTo(edit.peak.token));
          ranks.append(element("li", {}, measure,
            element("details", {}, element("summary", {text:"Inspect edit to " + name}),
              element("p", {text:JSON.stringify(excerpt(edit.before)) + " → " + JSON.stringify(excerpt(edit.after))}),
              edit.tool_selection_changed == null ? null : element("p", {text:edit.tool_selection_changed ? "The proposed tool selection changed." :
                edit.generated_choice_changed ? "The proposed tool stayed the same and its arguments changed." : "The proposed tool and arguments stayed the same."}),
              element("p", {text:edit.output_changed ? "Generated text changed after this edit" : "Generated text stayed the same after this edit"}),
              element("p", {className:"label", text:"Original generation"}), element("pre", {text:excerpt(page.baseline_output)}),
              element("p", {className:"label", text:"Edited generation"}), element("pre", {text:excerpt(edit.output)}), jump)));
        }
        const tokenLabel = "Token " + (row.output_token + 1) + " of " + page.total_output_tokens + " · " + JSON.stringify(excerpt(row.fragment));
        const strongest = sources[0];
        const ties = sources.filter(s => Math.abs(s.delta) === Math.abs(strongest.delta)).length;
        const finding = strongest.delta === 0 ? "All tested edits left this token’s measured probability unchanged." :
          ties > 1 ? plural(ties, "source edit") + " tied for the largest absolute change at this token." :
          "Editing " + sourceName(strongest.edit.source) + (strongest.delta < 0 ? " lowered" : " raised") +
            " this token’s probability from " + percent.format(Math.exp(baseline)) + " to " + percent.format(Math.exp(strongest.score)) + ".";
        const other = row.other_occurrence;
        const compare = other == null ? null : element("button", {type:"button", text:"Compare the same token at position " + (other + 1)});
        if (compare) compare.addEventListener("click", () => goTo(other));
        readout.replaceChildren(element("h3", {text:tokenLabel}), element("p", {className:"token-finding", text:finding}),
          element("p", {text:"Edits are ordered by the absolute change in log probability. Earlier output tokens stay fixed."}),
          element("div", {className:"edit-result label", "aria-hidden":"true"},
            element("span", {text:"Source edited"}), element("span", {text:"Edited probability"}), element("span", {text:"Change"})), ranks,
          link("Inspect decoder-block measurements for this token", "/details?" + new URLSearchParams({run:measured.run_id}) + row.inspector_path.slice(row.inspector_path.indexOf("#"))));
        if (compare) {
          const comparison = element("div", {className:"repeat-token"});
          if (ties === 1) comparison.append(element("p", {text:"For the " + sourceName(strongest.edit.source) +
            " edit, the shift was " + signed(strongest.delta) + " here and " + signed(strongest.other) +
            " at position " + (other + 1) + ". Each position uses its own recorded prefix."}));
          comparison.append(compare);
          readout.append(comparison);
        }
        const share = new URL(window.location.href);
        share.hash = "output-token-" + (row.output_token + 1);
        history.replaceState(null, "", share);
      }
      tokens.append(...buttons);
      const nav = element("div", {className:"actions"});
      for (const [label, next] of [["Earlier tokens", offset - pageSize], ["Later tokens", page.next_offset]]) {
        if (next == null || next < 0) continue;
        const button = element("button", {type:"button", text:label});
        button.addEventListener("click", () => load(next, true));
        nav.append(button);
      }
      const untested = page.captured_sources - page.edits.length;
      const coverage = element("p", {className:"label", text:plural(page.edits.length, "tested edit") + " · " +
        page.captured_sources + (page.captured_sources === 1 ? " captured context entry" : " captured context entries")});
      const untestedList = untested ? element("details", {}, element("summary", {text:untested + " captured inputs without a source-edit measurement"}),
        element("p", {text:page.untested_examples.map(sourceName).join(", ") + (untested > page.untested_examples.length ? " …" : "")})) : null;
      result.replaceChildren(coverage, element("p", {text:"Select an output token. The bar beneath it shows the largest absolute score shift among the tested edits, scaled to the largest shift in this answer."}), tokens, nav, readout);
      if (untestedList) result.append(untestedList);
      const selected = page.tokens.find(row => row.output_token === index);
      select(selected);
      if (focus) buttons[page.tokens.indexOf(selected)].focus();
    } catch (error) {
      if (request === currentRequest) result.replaceChildren(element("p", {role:"alert", text:error.message}));
    } finally {
      if (request === currentRequest) result.removeAttribute("aria-busy");
    }
  }
  const linked = /^#output-token-([1-9]\d*)$/.exec(window.location.hash);
  const requested = linked ? Number(linked[1]) - 1 : landmark.token;
  load(requested < landmark.total_tokens ? requested : landmark.token);
  return region;
}
