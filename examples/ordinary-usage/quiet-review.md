# Review of the six quiet sessions

The project owner delegated the review to Codex on November 21, 2026. Codex also implemented the pilot. These are source-checked developer judgments. No application maintainer or independent human signed off. The frozen feed showed zero recommendations on each workflow. All ten recommendation slots remained empty, so time to a confirmed recommendation was unobserved.

I reviewed all six sessions selected by the [predeclared sampling rule](protocol.md), including the application logs, the recorded requests and the data available to each application. Five warranted investigation. One had no material issue I could establish. These counts describe the sample, with related misses across questions. They do not measure production recall.

| Session and recorded call | Disposition | What I checked |
| --- | --- | --- |
| SQLite 2 · `db782ca291a14096` | Investigate a missed schema lookup. | The agent said it could not retrieve project timelines or file change logs. It had both schema inspection and SQL query tools and proposed neither. The connected database contained dates, subjects and changed paths for 210 commits. A search for “redact” or “retention” in the subjects returned no matches. The schema gave the agent a way to investigate. Establishing the exact introduction dates may require the source diffs. |
| SQLite 5 · `be6b475eaf0f4629` | Investigate an unsupported capability claim. | The agent said it could not analyze file changes over time and proposed no tool. A read-only query counted affected commits for the README (90), onboarding browser test (12), deployment guide (5) and Dockerfile (3). Which files qualify as onboarding depends on the user’s scope. |
| SQLite 3 · `e730b121e2c448a9` | Investigate a missed answer. | The agent said its tools could not retrieve UI changes alongside browser tests. A query against the attached database returned three concrete commits touching both `ui/` and browser tests, including `60b839d3`, `b1b8abee` and `baf541cc`. The agent proposed no tool. |
| PaperQA 2 · `c3b9ef10fb7d4f26` | Investigate retrieval for the operating-budget question. | The answer discussed decoder blocks and edit substitutions. All three excerpt requests used the methods document. The indexed validation document contained a one-CPU, 512 MiB benchmark and measured latency and memory. None of the retrieved passages or the final model input contained “512 MiB” or “39.7 MiB.” |
| PaperQA 4 · `80bdf478b6ea4faf` | Investigate an incorrect score claim and the omitted source. | The answer said incomplete capture decreases the workflow score. The source defines the score as passes divided by assessed runs. Adding an unresolved eligible run lowers coverage and can leave the score unchanged. The formula appeared in a chunk captured for an earlier question. It was absent from this question’s three retrieved excerpts and final input. |
| PaperQA 1 · `08f7ffa7726a4386` | No material miss established. | The answer identified recorded inputs, source order, tool definitions and request provenance. The indexed source supports those points. An engineer who needs token-exact inputs would also need runtime instrumentation. |

## Reproduce the checks

The [pilot record](first-pass.json) identifies all calls and includes a digest for each application log. The quiet sample’s local review files link to the full request bodies. The SQLite checks used the attached history database in read-only mode.

```sql
SELECT count(*) FROM commits;
SELECT count(*) FROM commits
WHERE lower(subject) LIKE '%redact%' OR lower(subject) LIKE '%retention%';

SELECT path, count(DISTINCT sha) AS commits FROM changes
WHERE lower(path) IN ('readme.md', 'deployment.md', 'dockerfile') OR lower(path) LIKE '%onboard%'
GROUP BY path ORDER BY commits DESC;

SELECT c.sha, c.date, c.subject FROM commits AS c
WHERE EXISTS (SELECT 1 FROM changes AS f WHERE f.sha=c.sha AND f.path LIKE 'ui/%')
  AND EXISTS (SELECT 1 FROM changes AS t WHERE t.sha=c.sha AND t.path LIKE 'tests/%browser%')
ORDER BY c.date DESC, c.sha LIMIT 3;
```

I compared each indexed document byte-for-byte with the frozen Git revision before checking the retrieved passages and final replies. [Original budget measurement](https://github.com/thakur-ro/A2Context/blob/efc533203cfae310dc4fde57fdcf1ac60295dd98/docs/validation.md#L42) · [Original score definition](https://github.com/thakur-ro/A2Context/blob/efc533203cfae310dc4fde57fdcf1ac60295dd98/docs/foundational-algorithms.md#L46)
