"""Run one question through the public workflow with its default prompts."""

from pathlib import Path
import sys


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from history_db import readonly_engine


def query_history(directory, endpoint, question):
    from langchain_community.agent_toolkits import create_sql_agent
    from langchain_community.utilities import SQLDatabase
    from langchain_openai import ChatOpenAI

    db = SQLDatabase(readonly_engine(directory))
    llm = ChatOpenAI(base_url=endpoint, api_key="loopback", model="default_model",
                     max_tokens=512, disable_streaming=True, max_retries=0)
    agent = create_sql_agent(llm, db=db, agent_type="tool-calling", max_iterations=8)
    print(agent.invoke({"input":question})["output"])


def ask_documents(directory, endpoint, question):
    from llama_index.core import Document
    from llama_index.core.node_parser import SentenceSplitter
    from llama_index.core.query_engine import CitationQueryEngine
    from llama_index.llms.openai_like import OpenAILike
    from llama_index.retrievers.bm25 import BM25Retriever

    docs = [Document(text=p.read_text(), metadata={"file": p.stem}) for p in sorted(directory.glob("*.txt"))]
    nodes = SentenceSplitter(chunk_size=512, chunk_overlap=64).get_nodes_from_documents(docs)
    retriever = BM25Retriever.from_defaults(nodes=nodes, similarity_top_k=3)
    llm = OpenAILike(model="default_model", api_base=endpoint, api_key="loopback",
                     is_chat_model=True, context_window=16384, max_tokens=512, max_retries=0)
    result = CitationQueryEngine(retriever=retriever, llm=llm).query(question)
    print(result)


if __name__ == "__main__":
    workflow, directory, endpoint, question = sys.argv[1:]
    {"sql":query_history, "citations":ask_documents}[workflow](Path(directory), endpoint, question)
