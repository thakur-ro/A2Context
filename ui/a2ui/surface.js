import { evidenceCatalog } from "./theme-frequency.js";
import { A2uiSurface } from "@a2ui/lit";
import { MessageProcessor, basicCatalog, setMarkdownRenderer } from "@a2ui/web_core/v1_0";

// The catalog uses Markdown for headings. Escape captured text before rendering it.
setMarkdownRenderer(markdown => {
  const match = /^(#{2,3}) (.+)$/s.exec(markdown);
  const text = (match ? match[2] : markdown.replace(/^\\([#\\])/, "$1"))
    .replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;");
  if (!match) return text;
  const tag = `h${match[1].length}`;
  return `<${tag}>${text}</${tag}>`;
});

const literal = text => text.replace(/^[#\\]/, "\\$&");

export function recommendation(surfaceId, { title, heading = "h3", label, description, detail, evidence = [] }) {
  const children = [];
  const components = [{ id: "root", component: "Column", children }];
  const destinations = new Map();
  function addText(id, text, variant = "body") {
    if (!text) return;
    children.push(id);
    components.push({ id, component: "Text", text, variant });
  }
  addText("label", label, "caption");
  addText("title", (heading === "h2" ? "## " : "### ") + title);
  addText("description", description && literal(description));
  addText("detail", detail, "caption");
  for (const [index, row] of evidence.entries()) {
    const button = `evidence_${index}`;
    const label = `${button}_label`;
    components.push({ id: label, component: "Text", text: literal(row.title) });
    addText(`${button}_summary`, literal(row.summary));
    components.push({ id: button, component: "Button", child: label, action: { event: { name: "openEvidence" } } });
    destinations.set(button, row.href);
    children.push(button);
  }
  return surface(surfaceId, components, destinations);
}

// Publish the complete comparison together, including both measured arms.
export function answerSurface({ title, description, detail, map, followup }, onFollowup) {
  const { view, update } = create("answer", ({name, sourceComponentId}) => {
    if (name === "followup") onFollowup();
    if (name === "inspectArm") {
      const arm = map.arms.find((_, i) => sourceComponentId === "inspect" + i);
      if (arm?.detail) update([{id:"readout", component:"Text", text:literal(arm.detail)}]);
    }
  });
  const children = [], components = [{id:"root",component:"Column",children}];
  function append(id, ...nodes) {
    children.push(id);
    components.push(...nodes);
  }
  append("title", { id: "title", component: "Text", text: "## " + title });
  if (description) append("description", { id: "description", component: "Text", text: literal(description) });
  if (map) {
    append("origin", { id: "origin", component: "Text", variant: "caption", text: literal(map.root) });
    // Keep comparisons together so a partially drawn result never implies a missing arm.
    append("comparison",
      { id:"comparison", component:"Column", children:["fork","readout"] },
      { id:"readout", component:"Text", text:"" },
      { id: "fork", component: "Row", children: map.arms.map((_, i) => "arm" + i) },
      ...map.arms.flatMap((arm, i) => [
        { id: "arm" + i, component: "Card", child: (arm.detail ? "inspect" : "node") + i },
        ...(arm.detail ? [{id:"inspect" + i, component:"Button", child:"node" + i, action:{event:{name:"inspectArm"}}}] : []),
        { id: "node" + i, component: "Column", children: ["name" + i, "value" + i] },
        { id: "name" + i, component: "Text", variant: "caption", text: literal(arm.label) },
        { id: "value" + i, component: "Text", text: literal(arm.value) },
      ]));
  }
  if (detail) append("detail", { id: "detail", component: "Text", variant: "caption", text: literal(detail) });
  if (followup) append("next",
    {id: "next", component: "Button", child: "nextLabel", action: {event: {name: "followup"}}},
    {id: "nextLabel", component: "Text", text: literal(followup)});
  update(components);
  return view;
}

export function questions(labels, ask) {
  const { view, update } = create("questions", ({name, sourceComponentId}) => {
    const index = labels.findIndex((_, i) => sourceComponentId === "ask" + i);
    if (name === "ask" && index >= 0) ask(labels[index]);
  });
  update([{ id: "root", component: "Row", children: labels.map((_, i) => "ask" + i) },
    ...labels.flatMap((label, i) => [
      {id: "ask" + i, component: "Button", child: "label" + i, action: {event: {name: "ask"}}},
      {id: "label" + i, component: "Text", text: literal(label)},
    ])]);
  return view;
}

function metricComponents(metrics) {
  return [{id:"metrics", component:"Row", children:metrics.map((_,i) => "metric" + i)},
    ...metrics.flatMap(({value,label},i) => [
      {id:"metric" + i,component:"Column",children:["value" + i,"label" + i]},
      {id:"value" + i,component:"Text",text:"### " + value},
      {id:"label" + i,component:"Text",text:literal(label),variant:"caption"},
    ])];
}

export function briefing({metrics}) {
  return surface("briefing",[{id:"root",component:"Column",children:["metrics"]},...metricComponents(metrics)],new Map());
}

export function themeFrequency(total,themes) {
  return surface("theme-frequency",[{id:"root",catalogId:evidenceCatalog.id,component:"ThemeFrequency",total,themes}],new Map());
}

export function workflowBriefing({metrics,recommendation}, investigate) {
  const children = ["metrics"];
  const components = [{id:"root",component:"Column",children},...metricComponents(metrics)];
  const actions = new Map();
  if (recommendation) {
    children.push("recommendation");
    components.push({id:"recommendation",component:"Card",child:"recommendationBody"},
      {id:"recommendationBody",component:"Column",children:["recommendationLabel","recommendationTitle","recommendationSummary","investigate"]},
      {id:"recommendationLabel",component:"Text",text:"Recommended investigation",variant:"caption"},
      {id:"recommendationTitle",component:"Text",text:"## " + recommendation.title},
      {id:"recommendationSummary",component:"Text",text:literal(recommendation.summary)},
      {id:"investigate",component:"Button",child:"investigateLabel",action:{event:{name:"openEvidence"}}},
      {id:"investigateLabel",component:"Text",text:"Investigate"});
    actions.set("investigate", investigate);
  }
  return surface("workflow-briefing",components,actions);
}

export function evidenceSections(surfaceId, {title, description, sections}) {
  const children = ["heading"];
  const components = [{id:"root",component:"Column",children},
    {id:"heading",component:"Text",text:"## " + title}];
  const destinations = new Map();
  if (description) {
    children.push("description");
    components.push({id:"description",component:"Text",text:literal(description)});
  }
  for (const [i, section] of sections.entries()) {
    const prefix = `section${i}`;
    const content = [`${prefix}Title`];
    children.push(prefix);
    components.push({id:prefix,component:"Card",child:`${prefix}Content`},
      {id:`${prefix}Content`,component:"Column",children:content},
      {id:`${prefix}Title`,component:"Text",text:"### " + section.title});
    for (const [index, text] of section.lines.entries()) {
      const id = `${prefix}Line${index}`;
      content.push(id);
      components.push({id,component:"Text",text:literal(text)});
    }
    if (section.onAction) {
      const id = `${prefix}Action`;
      content.push(id);
      components.push({id,component:"Button",child:`${id}Label`,action:{event:{name:"openEvidence"}}},
        {id:`${id}Label`,component:"Text",text:literal(section.action)});
      destinations.set(id,section.onAction);
    }
  }
  return surface(surfaceId,components,destinations);
}

function create(surfaceId, handleAction) {
  const processor = new MessageProcessor([basicCatalog,evidenceCatalog], handleAction);
  processor.processMessages([{ version: "1.0", createSurface: { surfaceId, catalogId: basicCatalog.id } }]);
  const view = new A2uiSurface();
  view.surface = processor.getSurface(surfaceId);
  return { view, update: components => processor.processMessages([
    { version: "1.0", updateComponents: { surfaceId, components } },
  ]) };
}

function surface(surfaceId, components, destinations) {
  const { view, update } = create(surfaceId, ({ name, sourceComponentId }) => {
    const destination = destinations.get(sourceComponentId);
    if (name !== "openEvidence") return;
    if (typeof destination === "function") destination();
    else if (destination?.startsWith("?")) location.assign(destination);
  });
  update(components);
  return view;
}
