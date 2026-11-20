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
  const processor = new MessageProcessor([basicCatalog], ({ name, sourceComponentId }) => {
    const destination = destinations.get(sourceComponentId);
    if (name === "openEvidence" && destination?.startsWith("?")) location.assign(destination);
  });
  processor.processMessages([
    { version: "1.0", createSurface: { surfaceId, catalogId: basicCatalog.id } },
    { version: "1.0", updateComponents: { surfaceId, components } },
  ]);
  const view = new A2uiSurface();
  view.surface = processor.getSurface(surfaceId);
  return view;
}
