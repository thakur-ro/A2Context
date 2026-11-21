"""Run the Collector developer's question through one public application pipeline."""

import asyncio
from pathlib import Path
import sys


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from history_db import readonly_engine


async def ask_agent(directory, endpoint, question):
    from llama_index.core.agent.workflow import FunctionAgent
    from llama_index.llms.openai_like import OpenAILike
    from llama_index.tools.database import DatabaseToolSpec

    tools = DatabaseToolSpec(engine=readonly_engine(directory)).to_tool_list()
    llm = OpenAILike(
        model="default_model",
        api_base=endpoint,
        api_key="loopback",
        is_chat_model=True,
        is_function_calling_model=True,
        context_window=16384,
        max_tokens=512,
        max_retries=0,
    )
    agent = FunctionAgent(tools=tools, llm=llm, streaming=False)
    print(await agent.run(user_msg=question, max_iterations=8))


def ask_docs(directory, endpoint, question):
    from langchain_classic.chains import RetrievalQAWithSourcesChain
    from langchain_community.retrievers import BM25Retriever
    from langchain_core.documents import Document
    from langchain_openai import ChatOpenAI
    from langchain_text_splitters import RecursiveCharacterTextSplitter

    docs = [
        Document(page_content=p.read_text(), metadata={"source": str(p.relative_to(directory))})
        for p in sorted(directory.rglob("README.md"))
    ]
    chunks = RecursiveCharacterTextSplitter(chunk_size=1500, chunk_overlap=150).split_documents(docs)
    retriever = BM25Retriever.from_documents(chunks, k=3)
    llm = ChatOpenAI(
        base_url=endpoint,
        api_key="loopback",
        model="default_model",
        max_tokens=512,
        disable_streaming=True,
        max_retries=0,
    )
    chain = RetrievalQAWithSourcesChain.from_chain_type(llm, chain_type="stuff", retriever=retriever)
    reply = chain.invoke({"question": question})
    print(reply["answer"])
    print("Sources:", reply["sources"])


if __name__ == "__main__":
    workflow, path, endpoint, question = sys.argv[1:]
    if workflow == "sql":
        asyncio.run(ask_agent(Path(path), endpoint, question))
    else:
        ask_docs(Path(path), endpoint, question)
