# Does another advertised tool influence the proposed argument name?

Three failed SQL proposals supplied a field called `table_names`. The selected query tool requires `query`. Another tool in the same request declares `table_names`. A shared name is an observation. To test influence, use each original rejected proposal where the sidecar locates a supplied field on another tool, provided the history contains no prior invocation of that other tool. Freeze these criteria before drawing any new response.

Run each original request twice and one edited request at seeds 11, 29, 47, 71 and 89 with the same local model, temperature 0.5 and a 512-token limit. In the edit rename only the other tool’s property and its corresponding required entry from `table_names` to the same-length alias `entry_names`. Keep the selected tool, all descriptions, prior conversation, question and tool order unchanged. Check that the alias is absent from the entire original and the old literal occurs nowhere else in the edited request. If either check fails, retain and explain the exclusion.

Shuffle arm order within each question and seed. Count whether a proposal for the originally selected tool uses the old or new argument name. A strict switch requires both unchanged replies to propose the old name for that tool and the edited reply to propose the alias for that same tool without the original. Count tool choice changes, other fields and failures separately. The intervention changes a hypothetical catalog only. No tool runs.

## Results

Three requests qualified. One rejected proposal had no field declared on a sibling tool. All 45 generations completed. Both unchanged arms reproduced the original extraneous field in only one of the 15 pairs. No edited proposal for the target tool used the original name or the alias. **There were zero strict switches.** The unchanged arms agreed on the proposed tools and field names in all 15 pairs. One edit changed the sequence of tool names by adding a query-checker proposal. The other 14 edits preserved the tool-name sequence. [All tool names and field lists](sibling-field-results.jsonl)

The field genuinely appeared on the other tool. This test gives us little evidence that the other declaration caused the original mistake. An engineer can see the shared name in the UI and judge whether further sampling is warranted. Reproduce using `python examples/ordinary-usage/transfer/sibling_field.py data/ordinary-usage/transfer-frozen/sql --sidecar-port 18474 --model-port 18742`.
