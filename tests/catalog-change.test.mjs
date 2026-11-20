import assert from "node:assert/strict";
import {readFile} from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";

const source = await readFile(new URL("../ui/tool-catalog.js",import.meta.url),"utf8");
const compare = vm.runInNewContext(source + "\ncatalogProposalChange");
const identify = (conditions, outside = []) => compare({conditions}, new Set(outside))?.name;
const arm = (names, completed, attempted = completed) => ({names, completed, attempted});

test("compares proposal rates in complete arms and skips missing evidence", () => {
  assert.equal(identify([arm({lookup:1},2),arm({lookup:2},4)]),undefined);
  assert.equal(identify([arm({},0),arm({lookup:1},1)]),undefined);
  assert.equal(identify([arm({},1),arm({lookup:1},1,2)]),undefined);
  assert.equal(identify([arm({lookup:1},1),arm({},1)]),"lookup");
});

test("selects a changed out-of-scope function first, including names inherited by JS objects", () => {
  assert.equal(identify([arm({lookup:2},2),arm({constructor:1},2)],["constructor"]),"constructor");
  assert.equal(identify([arm({lookup:2},2),arm({constructor:1},2)]),"lookup");
});
