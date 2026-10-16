import datetime as dt
import importlib.util
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest


spec = importlib.util.spec_from_file_location("compact_access_audit", Path(__file__).with_name("compact-access-audit.py"))
compact = importlib.util.module_from_spec(spec)
spec.loader.exec_module(compact)


class CompactionTests(unittest.TestCase):
    def test_preserves_memory_history_blobs_search_and_security_while_bounding_derived_logs(self):
        with tempfile.TemporaryDirectory() as directory:
            source, output = Path(directory) / "source.sqlite3", Path(directory) / "small.sqlite3"
            connection = sqlite3.connect(source)
            connection.execute("PRAGMA journal_mode=WAL")
            connection.executescript("""
                CREATE TABLE pcp_metadata(key TEXT PRIMARY KEY,value TEXT);
                INSERT INTO pcp_metadata VALUES('identity_id','idn_test');
                CREATE TABLE pcp_pages(page_id TEXT PRIMARY KEY,head TEXT);
                INSERT INTO pcp_pages(rowid,page_id,head) VALUES(41,'page_a','rev_two');
                CREATE TABLE pcp_revisions(id TEXT PRIMARY KEY,page_id TEXT REFERENCES pcp_pages(page_id),content BLOB);
                INSERT INTO pcp_revisions VALUES('rev_one','page_a',x'0001ff');
                INSERT INTO pcp_revisions VALUES('rev_two','page_a','旧内容与新证据');
                CREATE TABLE keyed(k TEXT PRIMARY KEY,v TEXT) WITHOUT ROWID;
                INSERT INTO keyed VALUES('key','value');
                CREATE VIRTUAL TABLE pcp_revision_fts USING fts5(revision_id UNINDEXED,content);
                INSERT INTO pcp_revision_fts(rowid,revision_id,content) VALUES(17,'rev_two','memory searchable');
                CREATE INDEX memory_page ON pcp_revisions(page_id);
                CREATE TABLE pcp_access_log(event_id TEXT PRIMARY KEY,occurred_at TEXT,principal_json TEXT,
                    session_id TEXT,operation TEXT,scopes_json TEXT,decision TEXT,detail TEXT,telemetry_json TEXT);
                CREATE INDEX audit_time ON pcp_access_log(occurred_at DESC,event_id DESC);
            """)
            now = dt.datetime.now(dt.timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")
            service = json.dumps({"principalType": "service", "principalId": "internal"})
            client = json.dumps({"principalType": "model_client", "principalId": "client"})
            for index in range(12):
                connection.execute("INSERT INTO pcp_access_log VALUES(?,?,?,?,?,?,?,?,?)", (
                    f"external_{index:02}", now, client, "s", "read_pages", '["a"]', "allowed", None, None
                ))
            for event, decision, principal, operation, timestamp in [
                ("internal", "allowed", service, "read_pages", now),
                ("internal_write", "allowed", service, "write_page", now),
                ("old_success", "allowed", client, "read_pages", "2020-01-01T00:00:00Z"),
                ("denied", "denied", service, "read_pages", "2020-01-01T00:00:00Z"),
                ("failed", "failed", client, "read_pages", "2020-01-01T00:00:00Z"),
            ]:
                connection.execute("INSERT INTO pcp_access_log VALUES(?,?,?,?,?,?,?,?,?)", (
                    event, timestamp, principal, "s", operation, '["a"]', decision, None, None
                ))
            connection.commit()
            connection.close()
            original = source.read_bytes()
            report = compact.prepare(source, output, "idn_test", max_allowed=3)
            self.assertTrue(report["readyToInstall"])
            self.assertEqual(source.read_bytes(), original)
            self.assertEqual(report["retainedAuditRows"], 5)
            self.assertEqual(report["retainedSecurityRows"], 2)
            connection = sqlite3.connect(output)
            events = {row[0] for row in connection.execute("SELECT event_id FROM pcp_access_log")}
            self.assertIn("denied", events)
            self.assertIn("failed", events)
            self.assertIn("internal_write", events)
            self.assertNotIn("internal", events)
            self.assertNotIn("old_success", events)
            self.assertEqual(connection.execute("SELECT rowid FROM pcp_pages").fetchone()[0], 41)
            self.assertEqual(connection.execute("SELECT revision_id FROM pcp_revision_fts WHERE pcp_revision_fts MATCH 'searchable'").fetchone()[0], "rev_two")
            self.assertEqual(connection.execute("SELECT content FROM pcp_revisions WHERE id='rev_one'").fetchone()[0], b"\x00\x01\xff")
            connection.close()
            self.assertEqual(output.stat().st_mode & 0o777, 0o600)

    def test_refuses_an_existing_output(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source.sqlite3"
            source.write_bytes(b"preserve")
            with self.assertRaises(ValueError):
                compact.prepare(source, source, "idn_test")
            self.assertEqual(source.read_bytes(), b"preserve")


if __name__ == "__main__":
    unittest.main()
