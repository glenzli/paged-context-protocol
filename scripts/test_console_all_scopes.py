#!/usr/bin/env python3
"""Verify persistent all-scope grants through real isolated Console/MCP processes.

Uses the same binary, timeout, report and temporary-home arguments as
test_console_local_clients.py. No real PCP home or existing authorization is used.
"""
from __future__ import annotations

from contextlib import ExitStack
import json
import subprocess
import sys

from test_console_local_clients import (
    ContractError, LocalClientHarness, READ_PERMISSIONS, digest_file, parse_arguments, require,
)


def denied_tool(session, name, arguments):
    reply = session.tool(name, arguments)
    require("error" in reply or reply.get("result", {}).get("isError") is True,
            "an unauthorized MCP action succeeded: " + name)


class AllScopesHarness(LocalClientHarness):
    def register(self, name, mode, all_scopes):
        body = self.payload(name, mode)
        if all_scopes:
            body.update(allScopes=True, scopes=[])
        pending = self.view(self.ok("POST", "/api/local-clients", body), body, "pending")
        require(pending.get("allScopes", False) == all_scopes, "scope policy changed before approval")
        self.denied("GET", "/api/local-clients/" + pending["id"] + "/config")
        ready = self.view(self.ok("POST", "/api/local-clients/" + pending["id"] + "/approve"), body, "ready")
        self.state(ready)
        _, config, _ = self.configuration(ready)
        return body, ready, config

    @staticmethod
    def all_grants(session, view):
        access = session.data("pcp_whoami", {})["access"]
        require(access["principal"]["principalId"] == view["principalId"], "all-scope identity changed")
        require(access["principal"]["principalType"] == "model_client", "all-scope client became an operator")
        expected = READ_PERMISSIONS | ({"ingest"} if view["mode"] == "contribute" else set())
        require(set(access.get("storePermissions", [])) == expected, "all-scope mode gained extra permissions")
        require(not access.get("grants"), "all-scope policy was materialized as a fixed scope list")

    def run(self):
        self.start_console()
        legacy_body, legacy, legacy_config = self.register("Selected legacy contributor", "contribute", False)
        reader_body, reader, reader_config = self.register("All scopes reader", "read", True)
        writer_body, writer, writer_config = self.register("All scopes contributor", "contribute", True)
        changed = dict(legacy_body, allScopes=True, scopes=[])
        self.denied("POST", "/api/local-clients", changed)
        self.denied("POST", "/api/local-clients", dict(self.payload("Conflicting policy"), allScopes=True))
        self.checkpoint("all and selected policies are immutable and need explicit approval")

        # New-project registration is deliberately a separate operator setting.
        self.ok("POST", "/api/enrollment/registrations/" + writer["registrationId"] + "/project-policy", {"enabled": True})
        with ExitStack() as stack:
            selected = stack.enter_context(self.session(legacy_config, "selected"))
            read = stack.enter_context(self.session(reader_config, "all-read"))
            contribute = stack.enter_context(self.session(writer_config, "all-contribute"))
            for session in (selected, read, contribute):
                session.initialize()
            personal = self.grants(selected, legacy)
            self.all_grants(read, reader)
            self.all_grants(contribute, writer)
            self.stage = "legacy contributor captures personal content"
            initial = selected.data("pcp_capture", self.capture(personal))
            require(initial.get("pageId"), "legacy contributor lost its original ingest grant")
            require("temporary integration Store" in json.dumps(read.data("pcp_read_pages", {"pageIds": [initial["pageId"]]})), "all-reader could not read an existing Page")
            denied_tool(read, "pcp_capture", self.capture(personal))

            for index in range(2):
                if index:
                    old_pid = self.ok("GET", "/api/runtime")["lifecycle"]["pid"]
                    restart = self.ok("POST", "/api/runtime/restart")
                    require(restart.get("restarted") and restart.get("pid") != old_pid, "isolated Runtime did not restart")
                key = "example.test/all-scope-future-" + str(index)
                project = contribute.data("pcp_ensure_project_scope", {"projectKey": key, "displayName": "Future scope " + str(index)})
                scope = project["scope"]
                # Sessions already existed before this Scope, and survive the restart.
                listing = read.data("pcp_list_scopes", {})
                require(scope in json.dumps(listing), "running all-reader did not see a future Scope")
                capture = dict(self.capture(scope), projectKey=key)
                self.stage = "all-scope contributor captures in a future Scope"
                written = contribute.data("pcp_capture", capture)
                content = read.data("pcp_read_pages", {"pageIds": [written["pageId"]]})
                require("temporary integration Store" in json.dumps(content), "all-reader did not read a future-scope Page")
                denied_tool(read, "pcp_capture", capture)
                denied_tool(selected, "pcp_read_pages", {"pageIds": [written["pageId"]]})
                denied_tool(selected, "pcp_capture", capture)
                self.all_grants(read, reader)
                self.all_grants(contribute, writer)
                require(self.grants(selected, legacy) == personal, "selected policy expanded after a future Scope or restart")
            self.checkpoint("running MCP clients follow future scopes and retain exact modes across Runtime restart")

            saved = self.ok("GET", "/api/local-clients/" + legacy["id"])
            require(not saved["allScopes"] and saved["scopes"] == ["user:self"], "legacy local profile was upgraded")
            _, recovered_config, _ = self.configuration(saved)
            require(recovered_config == legacy_config, "legacy configuration changed")
            state = json.loads((self.home / "data/pcp-enrollments.json").read_text())
            approved = next(item["approved_access"] for item in state["registrations"] if item["registration_id"] == legacy["registrationId"])
            require("all_scopes" not in approved and not approved["read_all_scopes"], "legacy persisted authorization gained a new flag")
            self.checkpoint("legacy profiles and persisted authorizations remain scoped")

            self.ok("POST", "/api/local-clients/" + reader["id"] + "/revoke")
            denied_tool(read, "pcp_read_pages", {"pageIds": [initial["pageId"]]})
            self.denied("GET", "/api/local-clients/" + reader["id"] + "/config")
            self.all_grants(contribute, writer)
            self.grants(selected, legacy)
            self.ok("POST", "/api/local-clients/" + writer["id"] + "/revoke")
            denied_tool(contribute, "pcp_capture", self.capture(personal))
            self.checkpoint("revocation invalidates running all-scope clients without affecting independent grants")
        self.no_secrets_exposed()


def main():
    args = parse_arguments()
    require(not args.self_test, "use test_console_local_clients.py --self-test for shared harness checks")
    report = {"schema": "pcp.console.all-scopes.validation.v1", "passed": False,
              "productionStateUsed": False,
              "binaries": {key: {"path": str(getattr(args, key)), "sha256": digest_file(getattr(args, key))}
                           for key in ("console_binary", "runtime_binary", "mcp_binary")}}
    harness = AllScopesHarness(args)
    try:
        harness.run()
        report["passed"] = True
    except (ContractError, OSError, subprocess.TimeoutExpired) as error:
        report.update(failureStage=harness.stage, error=str(error) if isinstance(error, ContractError) else type(error).__name__)
    finally:
        report["checks"] = harness.checks
        if args.keep_temp:
            report["privateTestDirectory"] = str(harness.directory)
        harness.close()
    if args.report:
        args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
