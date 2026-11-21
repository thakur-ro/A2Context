# Follow-up on the SQLite tool description

The five first requests in the SQLite pilot each advertised `SQLite_query` and `SQLite_schema`. The description at `/tools/0/function/description` told the model to use `list_databases`. That name was absent from the catalog. The original pilot has ended. This follow-up uses the same five sessions as development data.

**Prediction.** Replacing “Use list_databases to see available database names.” with “Use SQLite_schema to inspect the attached database schema.” will increase the number of first responses proposing an advertised tool. This outcome is observable without executing a proposal. The edit also changes what the sentence tells the model about the database.

Replay each session’s first recorded body on the same local model. Hold temperature at 0.5 and maximum generation length at 512 in all arms. Use seeds 11, 29 and 47 on each of the five questions. Within each question and seed, send the original twice and the edited request once in a fixed shuffled order. Keep failed responses and refusals. Report all 15 triples, original versus unchanged-repeat agreement, and gains and losses with the edit. Separate the number of questions from the number of generations. A change in tool selection cannot establish whether the proposed query ran or helped the user.

## Result

All 45 generations returned a response. The model proposed an advertised tool in **4 of 15** original trials, **4 of 15** unchanged repeats and **2 of 15** edited trials. All 15 unchanged pairs agreed on whether there was a proposal. The edit produced **zero gains and two losses**. Both losses occurred on questions for which the original proposed a schema lookup. In one other trial the edit added a query alongside the proposed schema lookup. These are five questions with three seeds each.

The request establishes that a tool description referred to an absent name. The intervention did not support recommending this edit to increase first-step tool use. A card claiming it would fix the agent’s refusal would overstate the evidence. The trial used MLX LM 0.32.2 and Liquid LFM2.5-1.2B-Instruct MLX 8-bit with weights SHA-256 `8c3f09f8d1d5739464d8c6b8a321f68f7fa7fe51f8fb11adf104ff637b50a58f`. No tool was executed and no paid model was called. [All 15 paired results](catalog-edit-results.jsonl)

```sh
python3 examples/ordinary-usage/catalog_edit.py data/ordinary-usage/tools/sidecar.sqlite --model-port 18742
```
