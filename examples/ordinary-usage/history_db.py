"""Open the pilot history database for read-only agent tools."""

import sqlite3


def readonly_engine(directory):
    from sqlalchemy import create_engine

    def connect():
        db = sqlite3.connect((directory / "history.db").as_uri() + "?mode=ro&immutable=1", uri=True)
        db.execute("PRAGMA query_only=ON")

        def authorize(op, name, argument, schema, caller):
            allowed = op in (
                sqlite3.SQLITE_READ,
                sqlite3.SQLITE_SELECT,
                sqlite3.SQLITE_FUNCTION,
                sqlite3.SQLITE_RECURSIVE,
                sqlite3.SQLITE_TRANSACTION,
            )
            if op == sqlite3.SQLITE_PRAGMA:
                allowed = name.lower() in {
                    "table_info",
                    "table_xinfo",
                    "index_list",
                    "index_info",
                    "index_xinfo",
                    "foreign_key_list",
                    "read_uncommitted",
                }
            return sqlite3.SQLITE_OK if allowed else sqlite3.SQLITE_DENY

        db.set_authorizer(authorize)
        return db

    return create_engine("sqlite://", creator=connect)
