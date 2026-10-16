#!/usr/bin/env python3
"""Prepare a verified small PCP database; never replace or mutate the source.

Stop the managed Runtime before running this tool. Install its output only after
the report confirms identical non-audit tables and a stable source snapshot.
"""

import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import time


SERVICE_READ_OPERATIONS = (
    "access_log", "active_revision_retention_leases", "browse_content_pages",
    "browse_index", "content_char_count", "content_library_summary", "current_revision",
    "describe", "durable_inventory", "expand_graph", "health_snapshot", "list_scopes",
    "page_count", "pending_feedback", "plan_revision_retention", "query_audit_summary",
    "read_pages", "revision_retention_leases", "search_pages",
)


def quoted(name):
    return '"' + name.replace('"', '""') + '"'


def source_stamp(path):
    result = {}
    for suffix in ("", "-wal"):
        item = Path(str(path) + suffix)
        if item.exists():
            stat = item.stat()
            if suffix == "-wal" and stat.st_size == 0:
                continue
            result[suffix] = (stat.st_ino, stat.st_size, stat.st_mtime_ns)
    return result


def projection(connection, schema, name, sql):
    columns = connection.execute(
        f"PRAGMA {schema}.table_xinfo({quoted(name)})"
    ).fetchall()
    names = [row[1] for row in columns if row[6] == 0]
    primary = sorted((row[5], row[1]) for row in columns if row[5])
    integer_primary = len(primary) == 1 and any(
        row[5] and row[2].upper() == "INTEGER" for row in columns
    )
    has_rowid = "WITHOUT ROWID" not in sql.upper()
    if has_rowid and not integer_primary:
        names.insert(0, "rowid")
    order = ",".join(quoted(row[1]) for row in primary) if primary else '"rowid"'
    return ",".join(map(quoted, names)), order


def table_digest(connection, schema, name, sql):
    columns, order = projection(connection, schema, name, sql)
    digest = hashlib.sha256()
    count = 0
    for row in connection.execute(
        f"SELECT {columns} FROM {schema}.{quoted(name)} ORDER BY {order}"
    ):
        # Length framing and type tags preserve BLOB/text/null distinctions.
        for value in row:
            if value is None:
                encoded = b"N"
            elif isinstance(value, bytes):
                encoded = b"B" + value
            else:
                encoded = type(value).__name__.encode() + b":" + str(value).encode("utf-8")
            digest.update(len(encoded).to_bytes(8, "big"))
            digest.update(encoded)
        count += 1
    return {"rows": count, "sha256": digest.hexdigest()}


def prepare(source, output, confirm_identity, days=7, max_allowed=10_000):
    source = Path(source).resolve(strict=True)
    output = Path(output).resolve()
    if source == output or output.exists():
        raise ValueError("output must be a new file, distinct from the source")
    if days <= 0 or max_allowed <= 0:
        raise ValueError("retention days and successful-event cap must be positive")
    stamp = source_stamp(source)
    output.parent.mkdir(parents=True, exist_ok=True)
    descriptor = os.open(output, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    os.close(descriptor)
    started = time.monotonic()
    connection = sqlite3.connect(output, uri=True)
    report = {"source": str(source), "output": str(output), "readyToInstall": False}
    try:
        connection.execute("ATTACH DATABASE ? AS snapshot", (source.as_uri() + "?mode=ro",))
        identity = connection.execute(
            "SELECT value FROM snapshot.pcp_metadata WHERE key='identity_id'"
        ).fetchone()
        if identity != (confirm_identity,):
            raise ValueError("source identity does not match --confirm-identity")
        connection.execute("PRAGMA foreign_keys=OFF")
        connection.execute("PRAGMA main.journal_mode=DELETE")
        connection.execute("BEGIN")
        objects = connection.execute(
            "SELECT type,name,tbl_name,sql FROM snapshot.sqlite_schema "
            "WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' ORDER BY rowid"
        ).fetchall()
        tables = [(name, sql) for kind, name, _, sql in objects if kind == "table"]
        virtual = {name for name, sql in tables if sql.upper().startswith("CREATE VIRTUAL TABLE")}
        shadows = {name + suffix for name in virtual for suffix in (
            "_data", "_idx", "_content", "_docsize", "_config"
        )}
        for name, sql in tables:
            if name not in shadows:
                connection.execute(sql)
        verified = {}
        for name, sql in tables:
            if name in virtual or name == "pcp_access_log":
                continue
            if name in shadows:
                connection.execute(f"DELETE FROM main.{quoted(name)}")
            columns, _ = projection(connection, "snapshot", name, sql)
            connection.execute(
                f"INSERT INTO main.{quoted(name)} ({columns}) "
                f"SELECT {columns} FROM snapshot.{quoted(name)}"
            )
            before = table_digest(connection, "snapshot", name, sql)
            after = table_digest(connection, "main", name, sql)
            if before != after:
                raise ValueError(f"non-audit table changed: {name}")
            verified[name] = after

        audit_sql = dict(tables)["pcp_access_log"]
        columns, _ = projection(connection, "snapshot", "pcp_access_log", audit_sql)
        # All denied/failed rows survive, including old internal access failures.
        connection.execute(
            f"INSERT INTO main.pcp_access_log ({columns}) "
            f"SELECT {columns} FROM snapshot.pcp_access_log WHERE decision <> 'allowed'"
        )
        cutoff = (dt.datetime.now(dt.timezone.utc) - dt.timedelta(days=days)).isoformat(
            timespec="milliseconds"
        ).replace("+00:00", "Z")
        placeholders = ",".join("?" for _ in SERVICE_READ_OPERATIONS)
        connection.execute(
            f"INSERT INTO main.pcp_access_log ({columns}) SELECT {columns} "
            "FROM snapshot.pcp_access_log WHERE decision='allowed' AND occurred_at >= ? "
            "AND NOT (COALESCE(json_extract(principal_json,'$.principalType'),'')='service' "
            f"AND operation IN ({placeholders})) "
            "ORDER BY occurred_at DESC,event_id DESC LIMIT ?",
            (cutoff, *SERVICE_READ_OPERATIONS, max_allowed),
        )
        for kind, _, _, sql in objects:
            if kind != "table":
                connection.execute(sql)
        for pragma in ("user_version", "application_id"):
            value = connection.execute(f"PRAGMA snapshot.{pragma}").fetchone()[0]
            connection.execute(f"PRAGMA main.{pragma}={value}")
        source_audit = connection.execute("SELECT count(*) FROM snapshot.pcp_access_log").fetchone()[0]
        retained_audit = connection.execute("SELECT count(*) FROM main.pcp_access_log").fetchone()[0]
        source_security = connection.execute(
            "SELECT count(*) FROM snapshot.pcp_access_log WHERE decision <> 'allowed'"
        ).fetchone()[0]
        retained_security = connection.execute(
            "SELECT count(*) FROM main.pcp_access_log WHERE decision <> 'allowed'"
        ).fetchone()[0]
        if source_security != retained_security:
            raise ValueError("security audit count changed")
        # Compare the actual records too, not only the counts.
        missing_security = connection.execute(
            f"SELECT {columns} FROM snapshot.pcp_access_log WHERE decision <> 'allowed' "
            f"EXCEPT SELECT {columns} FROM main.pcp_access_log WHERE decision <> 'allowed' LIMIT 1"
        ).fetchone()
        if missing_security is not None:
            raise ValueError("security audit content changed")
        connection.commit()
        integrity = connection.execute("PRAGMA main.integrity_check").fetchall()
        if integrity != [("ok",)]:
            raise ValueError(f"compacted database integrity failed: {integrity[:3]}")
        if connection.execute("PRAGMA main.foreign_key_check").fetchone() is not None:
            raise ValueError("compacted database foreign-key check failed")
        for name in virtual:
            connection.execute(f"INSERT INTO {quoted(name)} ({quoted(name)}) VALUES ('integrity-check')")
        connection.commit()
        if source_stamp(source) != stamp:
            raise ValueError("source changed during preparation; stop Runtime and prepare again")
        report.update({
            "identityId": confirm_identity,
            "retentionDays": days,
            "allowedEventCap": max_allowed,
            "sourceBytes": source.stat().st_size,
            "outputBytes": output.stat().st_size,
            "sourceAuditRows": source_audit,
            "retainedAuditRows": retained_audit,
            "retainedSecurityRows": retained_security,
            "nonAuditTables": verified,
            "integrity": "ok",
            "foreignKeys": "ok",
            "sourceStable": True,
            "readyToInstall": True,
            "seconds": round(time.monotonic() - started, 3),
        })
        return report
    except BaseException:
        connection.rollback()
        raise
    finally:
        connection.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--report", required=True)
    parser.add_argument("--confirm-identity", required=True)
    parser.add_argument("--days", type=int, default=7)
    parser.add_argument("--max-allowed", type=int, default=10_000)
    args = parser.parse_args()
    report = prepare(args.source, args.output, args.confirm_identity, args.days, args.max_allowed)
    report_path = Path(args.report)
    with report_path.open("x", encoding="utf-8") as stream:
        os.chmod(report_path, 0o600)
        json.dump(report, stream, ensure_ascii=False, indent=2)
        stream.write("\n")
    print(json.dumps({key: value for key, value in report.items() if key != "nonAuditTables"}, ensure_ascii=False))


if __name__ == "__main__":
    main()
