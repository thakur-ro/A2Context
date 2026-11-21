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

// Each batch attaches a complete subtree to the existing surface.
export function answerSurface({ title, description, detail, map, followup }, onFollowup) {
  const { view, update } = create("answer", ({name, sourceComponentId}) => {
    if (name === "followup") onFollowup();
    if (name === "inspectArm") {
      const arm = map.arms.find((_, i) => sourceComponentId === "inspect" + i);
      if (arm?.detail) update([{id:"readout", component:"Text", text:literal(arm.detail)}]);
    }
  });
  const children = [];
  function append(id, ...components) {
    children.push(id);
    update([{ id: "root", component: "Column", children: [...children] }, ...components]);
  }
  append("title", { id: "title", component: "Text", text: "## " + title });
  const steps = [];
  if (description) steps.push(() => append("description", { id: "description", component: "Text", text: literal(description) }));
  if (map) {
    steps.push(() => append("origin", { id: "origin", component: "Text", variant: "caption", text: literal(map.root) }));
    // Keep comparisons together so a partially drawn result never implies a missing arm.
    steps.push(() => append("comparison",
      { id:"comparison", component:"Column", children:["fork","readout"] },
      { id:"readout", component:"Text", text:"" },
      { id: "fork", component: "Row", children: map.arms.map((_, i) => "arm" + i) },
      ...map.arms.flatMap((arm, i) => [
        { id: "arm" + i, component: "Card", child: (arm.detail ? "inspect" : "node") + i },
        ...(arm.detail ? [{id:"inspect" + i, component:"Button", child:"node" + i, action:{event:{name:"inspectArm"}}}] : []),
        { id: "node" + i, component: "Column", children: ["name" + i, "value" + i] },
        { id: "name" + i, component: "Text", variant: "caption", text: literal(arm.label) },
        { id: "value" + i, component: "Text", text: literal(arm.value) },
      ])));
  }
  if (detail) steps.push(() => append("detail", { id: "detail", component: "Text", variant: "caption", text: literal(detail) }));
  if (followup) steps.push(() => append("next",
    {id: "next", component: "Button", child: "nextLabel", action: {event: {name: "followup"}}},
    {id: "nextLabel", component: "Text", text: literal(followup)}));
  return { view, steps };
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

export function briefing({ metrics, themes }) {
  const children = ["metrics"];
  const components = [
    {id:"root", component:"Column", children},
    {id:"metrics", component:"Row", children:metrics.map((_, i) => "metric" + i)},
    ...metrics.flatMap(({value,label}, i) => [
      {id:"metric" + i, component:"Column", children:["number" + i,"metricLabel" + i]},
      {id:"number" + i, component:"Text", text:"### " + value},
      {id:"metricLabel" + i, component:"Text", text:literal(label), variant:"caption"},
    ]),
  ];
  if (themes.length) {
    children.push("themes", ...themes.map((_, i) => "theme" + i));
    components.push({id:"themes", component:"Text", text:"### Recurring themes"}, ...themes.map((text, i) => (
      {id:"theme" + i, component:"Text", text:literal(text)})));
  }
  children.push("question");
  components.push({id:"question", component:"Text", text:"## What would you like to investigate?"});
  return surface("briefing", components, new Map());
}

function create(surfaceId, handleAction) {
  const processor = new MessageProcessor([basicCatalog], handleAction);
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
    if (name === "openEvidence" && destination?.startsWith("?")) location.assign(destination);
  });
  update(components);
  return view;
}
