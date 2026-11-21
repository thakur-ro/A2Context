# Follow-up on the SQLite tool description

The five first requests in the SQLite pilot each advertised `SQLite_query` and `SQLite_schema`. The description at `/tools/0/function/description` told the model to use `list_databases`. That name was absent from the catalog. The original pilot has ended. This follow-up uses the same five sessions as development data.

**Prediction.** Replacing “Use list_databases to see available database names.” with “Use SQLite_schema to inspect the attached database schema.” will increase the number of first responses proposing an advertised tool. This outcome is observable without executing a proposal. The edit also changes what the sentence tells the model about the database.

Replay each session’s first recorded body on the same local model. Hold temperature at 0.5 and maximum generation length at 512 in all arms. Use seeds 11, 29 and 47 on each of the five questions. Within each question and seed, send the original twice and the edited request once in a fixed shuffled order. Keep failed responses and refusals. Report all 15 triples, original versus unchanged-repeat agreement, and gains and losses with the edit. Separate the number of questions from the number of generations. A change in tool selection cannot establish whether the proposed query ran or helped the user.
