"""Use PaperQA's document retrieval and answer path with a local model."""

import asyncio
from pathlib import Path
import sys

from paperqa import Docs, Settings


async def ask(directory, endpoint, question):
    model = "openai/default_model"
    config = {"model_list": [{"model_name": model, "litellm_params": {
        "model": model, "api_base": endpoint, "api_key": "loopback", "max_tokens": 512}}]}
    settings = Settings(llm=model, llm_config=config, summary_llm=model, summary_llm_config=config,
                        embedding="sparse", parsing={"use_doc_details": False}, answer={"evidence_k": 3})
    docs = Docs()
    for path in sorted(directory.glob("*.txt")):
        await docs.aadd(path, citation=f"A2Context, {path.stem}, 2026", docname=path.stem, settings=settings)
    session = await docs.aquery(question, settings=settings)
    print(session.answer)


if __name__ == "__main__":
    asyncio.run(ask(Path(sys.argv[1]), *sys.argv[2:]))
