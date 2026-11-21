# Can a recommendation help the model repair a rejected proposal?

Use all four LangChain requests that ended in a missing-argument error during the frozen pilot. This is a follow-up on developer-inspected data. Freeze the design before generating more proposals.

For each request and each seed (11, 29, 47, 71, 89), generate three replies from the same local model at temperature 0.5 and a 512-token limit. Shuffle arm order within the pair. The first arm repeats the captured request. The other two append the actual assistant proposal and one simulated tool response. Both responses say the proposal has not run, ask for a corrected proposal and leave tool choice free. The generic response reports invalid arguments. The specific response also names the missing argument and the fields present, using the sidecar’s pre-dispatch result. All tools, earlier messages and application instructions stay identical. No tool is dispatched.

The primary outcome is a single new proposal for the originally rejected tool that passes its shipped argument validator. Report other tools, text answers, invalid arguments, multiple proposals and provider failures separately. Also ask SQLite to compile each qualifying read-only SELECT against the original schema with EXPLAIN QUERY PLAN. This measures whether the statement compiles, which is narrower than whether it answers the user. Use a read-only connection, deny writes and attachments, and never execute generated query results. Compare the counts with and without the detailed diagnostic on the same request and seed. Report every trial without extending the sample.

## Results

All 60 generations completed. Naming the missing field produced a valid proposal in six pairs where the generic response did not. Eight pairs passed with both messages and six passed with neither. No pair passed with the generic response alone.

| Outcome, 20 generations per arm | Original | Generic response | Specific response |
| --- | ---: | ---: | ---: |
| Single proposal accepted by the argument validator | 2 | 8 | 14 |
| Single proposal with invalid arguments | 17 | 0 | 0 |
| No tool proposed | 0 | 12 | 6 |
| Multiple proposals | 1 | 0 | 0 |
| Single proposal for another tool | 0 | 0 | 0 |
| Qualifying query compiled | 0 | 0 | 0 |

The accepted proposals still referenced nonexistent database objects. SQLite reported a missing column for 19 of the 24 statements and a missing table for five. An engineer who repairs the tool interface still has a second problem to investigate. These counts come from four questions on one model. No generated SQL ran. [Per-trial proposals and compilation results](repair-results.jsonl)

Reproduce with the transfer pilot’s Python environment and saved capture directory. Start the loopback sidecar and the original local model, then run `python examples/ordinary-usage/transfer/repair.py data/ordinary-usage/transfer-frozen/sql --sidecar-port 18474 --model-port 18742`.
