# Does a previously observed resource description help the next SQLite session?

Use the five ordinary SQLite sessions in arrival order. Admit a session only if a schema inspection result was already captured in an earlier session using the same catalog and the same public history database. The opening request must omit the returned schema. Check the schema text against the two table declarations in the frozen history database before use. A shared tool name or catalog alone cannot establish that two users have access to the same resource.

Four opening requests predate the first observed schema. Only the fifth question qualifies. Preserve all five in the coverage denominator. This is a retrospective development test with one eligible question. The engineer did not write a task test or a target answer for the application.

Keep the original first request and the tool definitions. Append the verified schema text to the existing schema tool’s description for the candidate arm. For a control edit, append the same number of characters of neutral prose about inspecting a database, without table or column names. Include the unchanged original and a second unchanged request. Use temperature 0.5, a 512-token limit, and the 15 seeds 1 through 15. Shuffle arm order within seed. Record whether the first response proposes an advertised tool. Execute no proposed queries. Report gains and losses against the original, unchanged-repeat agreement and the control. Seeds are repeated draws on this one question.

All 60 generations completed. None of the four arms proposed a tool at any seed. The candidate and neutral edit produced zero gains and zero losses against the original, and the unchanged repeats agreed on all 15 seeds. The earlier schema was verified against both table definitions. Each edited tool description was 162 characters. We found no support for recommending this metadata insertion to increase first-step tool proposals on this question. Whether the answer text improved was outside the registered outcome. [Results for all five sessions](earlier-schema-results.jsonl)

```sh
python3 examples/ordinary-usage/earlier_schema.py data/ordinary-usage/tools --model-port 18742
```
