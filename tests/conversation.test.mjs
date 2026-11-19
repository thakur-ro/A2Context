import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";

const source = readFileSync(new URL("../ui/conversation.js", import.meta.url), "utf8");
const { capturedUserInput, conversationTurns } = runInNewContext(source + "\n({ capturedUserInput, conversationTurns });", {
  matchMedia: () => ({ addEventListener() {} }),
});

function message(index, text, extra = {}) {
  const { role = "user", part_index, field = "request.messages", span_id = "inference", availability = "included" } = extra;
  return { kind: "text", role, availability, text, evidence: { field, span_id, message_index: index, part_index } };
}

test("displays adjacent user messages in a declared turn and orders captured parts", () => {
  const context = [
    message(0, "Earlier user request."),
    message(1, "Earlier assistant reply.", { role: "assistant" }),
    message(4, "Please handle the request now."),
    message(2, "Request type: price."),
    message(3, "19.", { part_index: 1 }),
    message(3, "Product ID:", { part_index: 0 }),
  ];
  const turn = conversationTurns([{ runIndex: 0, run: { conversation_id: "thread" },
    call: { id: "price", turn_id: "price-check", context } }])[0];
  assert.equal(turn.input, "Request type: price.\n\nProduct ID:\n19.\n\nPlease handle the request now.");
});

test("leaves gaps, different capture streams, and unavailable messages outside the turn text", () => {
  const latest = message(2, "Latest user input.");
  for (const boundary of [
    null,
    message(1, "Other source.", { field: "other.messages" }),
    message(1, "Other span.", { span_id: "earlier" }),
    message(1, null, { availability: "redacted" }),
    message(1, "Tool result.", { role: "tool" }),
  ]) {
    const context = [message(0, "Older request."), boundary, latest].filter(Boolean);
    assert.equal(capturedUserInput({ context }, true).text, "Latest user input.");
  }
  const context = [message(0, "Earlier request."), message(1, null, { availability: "redacted" })];
  assert.equal(capturedUserInput({ context }, true), null);
  const imprecise = [message(0, "Older request."), message(Number.MAX_SAFE_INTEGER + 5, "Latest user input.")];
  assert.equal(capturedUserInput({ context: imprecise }, true).text, "Latest user input.");
});

test("keeps only the newest captured message when turn identity is unavailable", () => {
  const earlier = message(0, "Which product?");
  const latest = message(1, "Product 19.");
  const turns = conversationTurns([
    { runIndex: 0, run: { conversation_id: "thread" }, call: { id: "a", context: [earlier] } },
    { runIndex: 0, run: { conversation_id: "thread" }, call: { id: "b", context: [earlier, latest] } },
    { runIndex: 0, run: { conversation_id: "thread" }, call: { id: "c", context: [earlier, latest] } },
  ]);
  assert.equal(turns[0].input, "Which product?");
  assert.equal(turns[1].input, "Product 19.");
  assert.equal(turns[2].input, null);
});

test("groups flat and single-batch LangSmith messages from their exact content paths", () => {
  for (const [stream, source] of [["inputs.messages", "langsmith"], ["inputs.messages[0]", "langsmith:project"]]) {
    const context = [
      message(null, "Prior question.", { field: stream + "[0].content" }),
      message(null, "Prior reply.", { field: stream + "[1].kwargs.content", role: "assistant" }),
      message(null, "Check the price.", { field: stream + "[2].data.content" }),
      message(null, "19.", { field: stream + "[3].data.content[1]" }),
      message(null, "Product ID:", { field: stream + "[3].data.content[0]" }),
    ];
    const call = { id: "langsmith-call", turn_id: "price-check", context };
    const [turn] = conversationTurns([{ runIndex: 0, run: { source, conversation_id: "thread" }, call }]);
    assert.equal(turn.input, "Check the price.\n\nProduct ID:\n19.");
    assert.equal(capturedUserInput(call, false, source).text, "Product ID:\n19.");
    assert.equal(capturedUserInput(call, true, "different-exporter").text, "Product ID:");
    assert.equal(context[4].evidence.field, stream + "[3].data.content[0]");
    assert.equal(context[4].evidence.message_index, null);
  }
});

test("keeps distinct LangSmith message streams separate", () => {
  const context = [
    message(null, "From a different message stream.", { field: "inputs.messages[1].content" }),
    message(null, "Latest input.", { field: "inputs.messages[0][2].kwargs.content" }),
  ];
  assert.equal(capturedUserInput({ context }, true, "langsmith").text, "Latest input.");
});

test("groups OpenInference text parts in numeric order using their flattened paths", () => {
  const context = [
    message(null, "Prior question.", { field: "llm.input_messages.7.message.content" }),
    message(null, "Prior reply.", { field: "llm.input_messages.8.message.content", role: "assistant" }),
    message(null, "Check the price.", { field: "llm.input_messages.9.message.content" }),
    message(null, "19.", { field: "llm.input_messages.10.message.contents.10.message_content.text" }),
    message(null, "Product ID:", { field: "llm.input_messages.10.message.contents.2.message_content.text" }),
  ];
  const call = { id: "openinference-call", turn_id: "price-check", context };
  const [turn] = conversationTurns([{ runIndex: 0, run: { source: "otlp", conversation_id: "thread" }, call }]);
  assert.equal(turn.input, "Check the price.\n\nProduct ID:\n19.");
  context[2].availability = "redacted";
  context[2].text = null;
  assert.equal(capturedUserInput(call, true, "otlp").text, "Product ID:\n19.");
  assert.equal(capturedUserInput(call, true, "different-exporter").text, "Product ID:");
  assert.equal(context[4].evidence.field, "llm.input_messages.10.message.contents.2.message_content.text");
});

function entry(id, turn_id, runIndex, conversation_id, text, message_index, extras = []) {
  const input = {
    kind: "text", role: "user", availability: "included", text,
    evidence: { field: "gen_ai.input.messages", message_index },
  };
  return {
    runIndex, run: { conversation_id },
    call: { id, turn_id, context: [input, ...extras] },
  };
}

test("joins declared turns across traces of one conversation without creating new user messages", () => {
  const turns = conversationTurns([
    entry("a", "turn-1", 0, "thread", "Check my order.", 0, [{
      kind: "text", role: "user", availability: "included", text: "Order A-123.",
      evidence: { field: "gen_ai.input.messages", message_index: 0 },
    }]),
    entry("b", "turn-1", 1, "thread", "Check my order.", 0),
    entry("c", "turn-2", 1, "thread", "Check my order.", 0),
  ]);
  assert.equal(turns.length, 2);
  assert.equal(turns[0].calls.length, 2);
  assert.equal(turns[0].input, "Check my order.\nOrder A-123.");
  assert.equal(turns[1].input, "Check my order.");
});

test("keeps calls with missing turn IDs distinct and suppresses repeated request history", () => {
  const turns = conversationTurns([
    entry("a", null, 0, "thread", "Check my order.", 0),
    entry("b", null, 0, "thread", "Check my order.", 0),
    entry("c", null, 0, "thread", "Check my order.", 2),
  ]);
  assert.equal(turns.length, 3);
  assert.equal(turns[0].input, "Check my order.");
  assert.equal(turns[1].input, null);
  assert.equal(turns[2].input, "Check my order.");
});
