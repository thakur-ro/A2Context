# Five fresh questions for the Collector history agent

Run the LlamaIndex FunctionAgent with its original prompts and unmodified database tools on the same frozen public history database. Start an empty deployment using the signature-aware detector. Keep the model, token limit and provider settings from the Collector pilot. Send these questions once, in order.

1. How many distinct commits in the available history changed a README file?
2. Which three paths under processor were touched in the largest number of distinct commits?
3. What was the subject and date of the latest commit that changed a go.mod file under exporter?
4. Which dates in this history saw changes under both receiver and exporter, even if they were in separate commits?
5. How many commits touched both a Go source file and a Markdown document?

Review the first five distinct recommendations in order of appearance. Verify the catalog and exact reply and independently check whether the connected database could answer the question. Keep all five sessions, including model or application failures. Review up to three silent sessions by lowest SHA-256 of trace ID. Record elapsed time from first connection to first visible finding and to developer confirmation. A grouped repeat counts once. These are new questions on a familiar application. The pilot for two unfamiliar workflows retains its original acceptance criterion.
