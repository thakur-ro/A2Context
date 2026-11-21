# Can the frozen feed help someone working on the OpenTelemetry Collector?

Freeze the sidecar at `29b0801`. Start two empty deployments, use each application for five engineering questions in the order below, and retain every outcome. Use public Collector revision `8e266ba207ab3ad93e3606578e3b56a5fbd47adf`. This changes the corpus and application implementations. The LangChain and LlamaIndex projects also supplied different components in the previous pilot. Use the same local LFM2.5-1.2B-Instruct model and 512-token limit to isolate the workflow change. No paid API calls.

For repository history, use LlamaIndex FunctionAgent with DatabaseToolSpec’s shipped functions and no custom system prompt. Give it read-only SQLite tables of commits and changed paths from the 100 most recent first-parent commits at the pinned revision. Allow eight agent iterations. For operational guidance, use LangChain RetrievalQAWithSourcesChain, its shipped stuff prompt, BM25 retrieval of three chunks, and a source path on each document. Index the README files for exporterhelper, otlpexporter, batchprocessor, memorylimiterprocessor and otlpreceiver. Use 1,500-character chunks with 150 characters of overlap. Preserve the actual answer and source field. Use nonstreaming transport for capture.

| Order | Repository history | Operational guidance |
| --- | --- | --- |
| 1 | In the available history, which three receiver paths changed in the most commits? Give counts. | If the backend stops accepting exports, what happens to data once the sending queue fills? |
| 2 | What is the most recent commit touching exporterhelper, and what did its subject say? | Which batch setting actually limits the size of a batch, and how does it differ from the setting that triggers a send? |
| 3 | Which changes touched both a production Go file and its test under processor? Give me examples to inspect. | How should an operator choose the soft and hard memory limits, and what should the sending component do when data is refused? |
| 4 | On how many distinct commit dates were files under receiver/otlpreceiver changed? | Which configuration enables HTTP alongside gRPC on the OTLP receiver? Where is it documented? |
| 5 | Which five files under exporter changed most often in this history? | What survives a Collector restart when a sending queue uses persistent storage? |

Use the original first-five distinct recommendations per workflow, the published ordering and the three lowest hash-ranked quiet sessions per workflow. Codex reviews findings against exact model and application records and the pinned repository. Count repetitions once. Report the elapsed time from first connection to first displayed and first reviewer-confirmed useful recommendation separately. Record missed issues, dismissals, capture errors, framework errors, and unsupported inputs. Stop after the predefined questions even if all sessions are quiet. No detector changes count toward this prospective result.
