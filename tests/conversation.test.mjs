import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";

const source = readFileSync(new URL("../ui/conversation.js", import.meta.url), "utf8");
const { conversationTurns } = runInNewContext(source + "\n({ conversationTurns });", {
  matchMedia: () => ({ addEventListener() {} }),
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
