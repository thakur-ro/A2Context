# Does the revised feed work on two other workflows?

Freeze A2Context at `2e6159e` before connecting either workflow. Use LangChain’s SQL agent with its shipped toolkit and LlamaIndex’s CitationQueryEngine with its shipped BM25 retriever. Use the same local LFM model, the same public commit database and the same four documents as the [first pilot](../protocol.md). Holding the materials fixed lets us inspect answer quality across different application implementations. It limits what we can say about new domains. Neither workflow receives expected answers, prompts written for diagnosis, or test-specific detection rules. [LangChain SQL agent](https://reference.langchain.com/python/langchain-community/agent_toolkits/sql/base/create_sql_agent) · [LlamaIndex citation engine](https://developers.llamaindex.ai/python/examples/query_engine/citation_query_engine/)

Start each workflow with an empty sidecar database. Run the following questions once in order. Allow five minutes per question and eight agent iterations. Keep all failures. Record application stdout, final status, returned provider messages, first appearances of distinct findings, evidence links and quiet cases. The local capture adapter records provider traffic. Execution inside either framework has no separate telemetry in this setup.

| Order | SQL agent | Citation engine |
| --- | --- | --- |
| 1 | Which three files under deploy have changed most frequently, and how many commits touched each? | What is the difference between a source conflict and evidence that a source influenced an answer? |
| 2 | Which commit first added a Dockerfile, and on what date? | How was the sidecar’s memory use measured and under what resource limits? |
| 3 | Which five commits changed both the README and a Python test file? Give their subjects. | What can we infer when repeated requests produce different tool proposals before any source edit? |
| 4 | In the recorded history, how many distinct days had changes under src/sidecar? | Which source failures can the capture coverage check establish? |
| 5 | What are the most recent three commit subjects that mention a tool? | Which parts of the context attribution work have been tested on instrumented models? |

Use the original first-five rule and hash-based quiet sample of up to three runs per workflow. Codex reviews each finding against application records and checks the quiet sample against the public database and documents. A repeated observation occupies one place. If no findings appear, report the empty slots, the missed issues and that first-useful time was unobserved. Never add an incident or change the detector to improve the recorded result.
