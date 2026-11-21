// Counts come from the scoped sidecar summary. A conversation can have more than one theme.
import {html, nothing} from "lit";
import {z} from "zod";
import {Catalog} from "@a2ui/web_core/v1_0";
import {A2uiLitElement, registerUniversalElement} from "@a2ui/web_core/universal";

const count = z.number().int().nonnegative();
const api = {name:"ThemeFrequency",schema:z.object({
  total:count, themes:z.array(z.object({label:z.string(), conversations:count, groups:count})).max(3),
}).strict()};

class ThemeFrequency extends A2uiLitElement {
  api = api;
  createRenderRoot() { return this; }
  render() {
    if (!this.controller?.props) return nothing;
    const {themes,total} = this.controller.props;
    return html`<section class="frequency" aria-label="Recurring themes">
      <header><h2>Recurring themes</h2><span>Conversations</span></header>
      <ul>${themes.map(({label,conversations,groups},index) => html`<li><button type="button"
        title=${"Show an example of " + label}
        @click=${() => this.context.dispatchAction({event:{name:"inspectTheme",context:{index}}})}>
        <span class="frequency-name">${label}</span>
        ${total && conversations ? html`<progress class="frequency-track" aria-hidden="true" value=${conversations} max=${total}></progress>` : html`<span></span>`}
        <span class="frequency-count">${conversations || !groups ? conversations + " of " + total : groups + (groups === 1 ? " group" : " groups") + " · conversation unknown"}</span>
        <span class="frequency-open" aria-hidden="true">›</span>
      </button></li>`)}</ul>
    </section>`;
  }
}

const component = {...api,tagName:"a2context-theme-frequency",element:ThemeFrequency};
registerUniversalElement(component);
export const evidenceCatalog = new Catalog("https://github.com/thakur-ro/A2Context#evidence-v1","1.0",[component],[]);
