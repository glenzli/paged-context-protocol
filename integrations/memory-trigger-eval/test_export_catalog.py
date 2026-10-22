import json
from pathlib import Path
import sys
import tempfile
import unittest

from export_catalog import export_catalog


class CatalogExportTests(unittest.TestCase):
    def test_real_stdio_initialize_notifications_and_paginated_tools(self):
        with tempfile.TemporaryDirectory(prefix="pcp-catalog-export-") as directory:
            trace = Path(directory) / "requests.jsonl"
            server = r'''
import json, sys
trace = open(sys.argv[1], "w")
for line in sys.stdin:
    request = json.loads(line)
    trace.write(json.dumps(request) + "\n"); trace.flush()
    method = request["method"]
    if method == "notifications/initialized":
        continue
    if method == "initialize":
        result = {"instructions":"real wire guidance", "protocolVersion":"2025-06-18",
                  "serverInfo":{"name":"synthetic-server", "version":"1"}}
        print(json.dumps({"jsonrpc":"2.0", "method":"notifications/message", "params":{}}), flush=True)
    elif method == "tools/list":
        continued = request["params"].get("cursor") == "second"
        result = {"tools":[{"name":"second" if continued else "first", "inputSchema":{"type":"object"}}]}
        if not continued:
            result["nextCursor"] = "second"
    else:
        raise RuntimeError("unexpected operation")
    print(json.dumps({"jsonrpc":"2.0", "id":request["id"], "result":result}), flush=True)
'''
            catalog = export_catalog([sys.executable, "-u", "-c", server, str(trace)])
            self.assertEqual(catalog["instructions"], "real wire guidance")
            self.assertEqual([t["name"] for t in catalog["tools"]], ["first", "second"])
            requests = [json.loads(line) for line in trace.read_text().splitlines()]
            self.assertEqual([r["method"] for r in requests], [
                "initialize", "notifications/initialized", "tools/list", "tools/list"])
            self.assertEqual(requests[-1]["params"], {"cursor":"second"})

    def test_early_exit_retains_actionable_connection_error(self):
        with self.assertRaisesRegex(ValueError, "connection unavailable"):
            export_catalog([sys.executable, "-c", "import sys; sys.exit('connection unavailable')"])

    def test_unresponsive_server_is_bounded(self):
        with self.assertRaisesRegex(ValueError, "timed out"):
            export_catalog([sys.executable, "-u", "-c", "import time; time.sleep(20)"], timeout=0.05)


if __name__ == "__main__":
    unittest.main()
