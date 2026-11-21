"use strict";

// The scores on a page share the recorded output prefix at each position.
function tokenPath(card, {sharePosition = true, inline = false, filters = scope} = {}) {
  const {detail} = card.evidence;
  const {landmark, location: measured} = detail;
  const pageSize = 16;
  const fragment = text => text || "Partial character";
  const excerpt = value => value.text + (value.truncated ? "…" : "");
  const percent = new Intl.NumberFormat(undefined, {style:"percent", maximumSignificantDigits:4});
  const region = element("section", {className:"panel answer-path", "aria-label":"Source edits along the answer"});
  const title = element("h2", {text:"How did source edits affect this answer?"});
  const headline = element("p", {className:"token-finding","aria-live":"polite"});
  const result = element("div", {"aria-live":"polite", text:"Loading token measurements…"});
  region.append(title, ...(inline ? [headline] : []), result);
  let currentRequest = 0;
  const linkedSelection = new URLSearchParams(sharePosition ? location.hash.slice(1) : "");
  let linkedGeneration = linkedSelection.get("generation");

  async function load(index, focus = false) {
    const request = ++currentRequest;
    const offset = Math.floor(index / pageSize) * pageSize;
    result.setAttribute("aria-busy", "true");
    try {
      const page = await api("token-path", {...filters, trace:measured.run_id, call:measured.call_id,
        decision:measured.decision_id, report:detail.report_id, group:card.id, factor:detail.factor,
        offset, block:landmark.block});
      if (request !== currentRequest) return;
      const metadata = new Map(page.edits.map(edit => [edit.context_id, edit]));
      const readout = element("div", {className:"token-readout"});
      const buttons = page.tokens.map(row => {
        const text = row.fragment.text;
        const button = element("button", {type:"button", text:text.trim() ? excerpt(row.fragment) : (text ? "␣" : "◌"),
          "aria-label":"Token " + (row.output_token + 1) + " · " + JSON.stringify(fragment(text))});
        button.addEventListener("click", () => select(row));
        button.addEventListener("keydown", event => {
          if (!["ArrowLeft","ArrowRight"].includes(event.key)) return;
          event.preventDefault();
          const next = row.output_token + (event.key === "ArrowRight" ? 1 : -1);
          if (next >= 0 && next < page.total_output_tokens) goTo(next);
        });
        return button;
      });
      const map = tokenEditMap(page,buttons,select);
      function goTo(token) {
        const local = page.tokens.find(row => row.output_token === token);
        if (!local) return load(token, true);
        select(local);
        buttons[page.tokens.indexOf(local)].focus();
      }
      function select(row, sourceId) {
        const index = page.tokens.indexOf(row);
        buttons.forEach((button,i) => button.setAttribute("aria-pressed", String(i === index)));
        for (const band of map.querySelectorAll("tbody tr")) {
          for (const cell of Array.from(band.cells).slice(1)) {
            const selected = cell.cellIndex === index+1;
            cell.toggleAttribute("data-selected",selected);
            if (cell.firstElementChild) cell.firstElementChild.tabIndex = selected ? 0 : -1;
          }
        }
        const selected = buttons[index].getBoundingClientRect();
        const visible = map.getBoundingClientRect();
        const labelWidth = map.querySelector("thead th").getBoundingClientRect().width;
        if (selected.left < visible.left+labelWidth) map.scrollLeft += selected.left-visible.left-labelWidth;
        else if (selected.right > visible.right) map.scrollLeft += selected.right-visible.right;
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
            element("details", {"data-generation":edit.context_id}, element("summary", {text:"Inspect edit to " + name}),
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
        if (inline) headline.textContent = finding;
        readout.replaceChildren(element("h3", {text:tokenLabel}), ...(!inline ? [element("p", {className:"token-finding", text:finding})] : []),
          element("p", {text:"Edits are ordered by the absolute change in log probability. Earlier output tokens stay fixed."}),
          element("div", {className:"edit-result label", "aria-hidden":"true"},
            element("span", {text:"Source edited"}), element("span", {text:"Edited probability"}), element("span", {text:"Change"})), ranks);
        if (!inline) readout.append(link("Inspect decoder-block measurements for this token", "/details?" + new URLSearchParams({run:measured.run_id}) + row.inspector_path.slice(row.inspector_path.indexOf("#"))));
        if (compare) {
          const comparison = element("div", {className:"repeat-token"});
          if (ties === 1) comparison.append(element("p", {text:"For the " + sourceName(strongest.edit.source) +
            " edit, the shift was " + signed(strongest.delta) + " here and " + signed(strongest.other) +
            " at position " + (other + 1) + ". Each position uses its own recorded prefix."}));
          comparison.append(compare);
          readout.append(comparison);
        }
        const id = sourceId || linkedGeneration;
        const generation = id && [...ranks.querySelectorAll("details")].find(node => node.dataset.generation === id);
        if (generation) generation.open = true;
        if (linkedGeneration && generation) generation.querySelector("summary").focus();
        linkedGeneration = null;
        if (!sharePosition) return;
        const share = new URL(window.location.href);
        share.hash = generation ? new URLSearchParams({"output-token":row.output_token+1,generation:id}) :
          "output-token-" + (row.output_token + 1);
        history.replaceState(null, "", share);
      }
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
      result.replaceChildren(coverage, map,
        element("p",{className:"token-map-key",text:"↑ Raised · ↓ Lowered · = No change. Stronger color shows a larger absolute change in log probability. Select a cell to inspect the edit."}),
        nav, readout);
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
  const linked = sharePosition && /^#output-token-([1-9]\d*)$/.exec(window.location.hash);
  const position = linked ? linked[1] : linkedSelection.get("output-token");
  const requested = Number(position)-1;
  load(Number.isSafeInteger(requested) && requested >= 0 && requested < landmark.total_tokens ? requested : landmark.token);
  return region;
}

function tokenEditMap(page,buttons,choose) {
  const maximum = Math.max(0,...page.edits.map(edit => Math.abs(edit.peak?.delta || 0)));
  const columns = page.tokens.map(row => new Map(row.sources.map(s =>
    [s.context_id,s.edited_prompt_log_probability-row.baseline_log_probability])));
  const header = element("tr",{},element("th",{scope:"col",text:"Source edited"}),
    ...buttons.map((button,i) => element("th",{scope:"col"},element("small",{text:page.tokens[i].output_token+1}),button)));
  const rows = page.edits.map(edit => element("tr",{},element("th",{scope:"row",text:sourceName(edit.source)}),
    ...columns.map((column,i) => {
      const delta = column.get(edit.context_id);
      const description = "Position "+(page.tokens[i].output_token+1)+" · "+(delta == null ? "No measurement" : signed(delta)+" log p");
      if (delta == null) return element("td",{text:"—",title:description,"aria-label":description});
      const button = element("button",{type:"button",title:description,text:delta > 0 ? "↑" : delta < 0 ? "↓" : "=",
        "aria-label":sourceName(edit.source)+" · "+description});
      button.onclick = () => choose(page.tokens[i],edit.context_id);
      button.onkeydown = event => {
        const step = {ArrowLeft:-1,ArrowRight:1}[event.key];
        if (!step) return;
        const target = button.closest("tr").cells[i+1+step]?.querySelector("button");
        if (target) {event.preventDefault();target.focus();target.click();}
      };
      const cell = element("td",{},button);
      if (delta) {
        cell.dataset.direction = delta > 0 ? "up" : "down";
        cell.style.setProperty("--strength",String(Math.abs(delta)/maximum));
      }
      return cell;
    })));
  return element("div",{className:"token-edit-map",tabindex:"0","aria-label":"Scroll through output positions"},
    element("table",{"aria-label":"Measured source edits by output token"},element("thead",{},header),element("tbody",{},...rows)));
}
