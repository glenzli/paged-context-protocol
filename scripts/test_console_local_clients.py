#!/usr/bin/env python3
"""Exercise Console local-client onboarding against real, isolated PCP processes.

No installed service, real PCP home, LaunchAgent, or existing Store is used. The
three binaries must already be built; this script never invokes Cargo. Example:

    python3 scripts/test_console_local_clients.py \
      --console-binary target/debug/pcp-console \
      --runtime-binary target/debug/pcp-runtime \
      --mcp-binary target/debug/pcp-mcp --report /tmp/pcp-local-clients.json

The report contains assertions and binary hashes, never credentials or raw RPC
transcripts. --keep-temp retains the private test home for local investigation.
Run --self-test to check the harness without starting PCP.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import secrets
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import urllib.error
import urllib.parse
import urllib.request
import uuid


READ_PERMISSIONS = {"list_scopes", "search", "read_summary", "read_detail"}
SAFE_ENV_KEYS = {
    "PATH", "HOME", "USER", "LOGNAME", "TMPDIR", "LANG", "LC_ALL", "LC_CTYPE",
    "SYSTEMROOT", "DYLD_LIBRARY_PATH",
}
SENSITIVE_KEY = re.compile(r"credential|password|secret|(^|[_-])token($|[_-])", re.I)


class ContractError(RuntimeError):
    """A redacted, actionable contract failure; never embeds response bodies."""


def require(condition, message):
    if not condition:
        raise ContractError(message)


def clean_environment(source):
    # An allowlist also removes unrelated provider credentials from test children.
    return {key: value for key, value in source.items() if key in SAFE_ENV_KEYS}


def public_fields(value):
    if isinstance(value, dict):
        for key, item in value.items():
            require(not SENSITIVE_KEY.search(key), "public response contains a credential field")
            public_fields(item)
    elif isinstance(value, list):
        for item in value:
            public_fields(item)


def digest_file(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def private_path(path, root, *, directory=False):
    require(not path.is_symlink(), "private client state must not be a symlink")
    require(path.resolve().is_relative_to(root.resolve()), "private client state escaped the test home")
    info = path.stat()
    require(info.st_uid == os.getuid(), "private client state has a different owner")
    require(info.st_mode & 0o077 == 0, "private client state is accessible to another user")
    require(path.is_dir() if directory else path.is_file(), "private client state has the wrong file type")


def stop_owned(process):
    """Only signal the private session created for this exact Popen child."""
    if process is None:
        return
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=15)
        except subprocess.TimeoutExpired:
            pass
    # Also collect a child Runtime left behind by a failed Console startup.
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    deadline = time.monotonic() + 3
    while time.monotonic() < deadline:
        try:
            os.killpg(process.pid, 0)
        except ProcessLookupError:
            break
        process.poll()
        time.sleep(0.05)
    else:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
    process.wait(timeout=3)


class McpSession:
    """Minimal newline-delimited MCP client; generated command/args/env are intact."""

    def __init__(self, config, environment, log_directory, label, timeout):
        self.config = config
        self.timeout = timeout
        self.sequence = 0
        self.incoming = queue.Queue()
        self.stderr = (log_directory / (label + ".stderr")).open("wb")
        child_environment = dict(environment)
        child_environment.update(config["env"])
        self.process = subprocess.Popen(
            [config["command"], *config["args"]],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.stderr,
            cwd=log_directory, env=child_environment, start_new_session=True,
        )
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()

    def _read(self):
        try:
            for line in self.process.stdout:
                self.incoming.put(line)
        finally:
            self.incoming.put(None)

    def request(self, method, params=None):
        self.sequence += 1
        request = {"jsonrpc": "2.0", "id": self.sequence, "method": method}
        if params is not None:
            request["params"] = params
        self._send(request)
        deadline = time.monotonic() + self.timeout
        while True:
            try:
                line = self.incoming.get(timeout=max(0.01, deadline - time.monotonic()))
            except queue.Empty:
                raise ContractError("MCP response timed out for " + method) from None
            require(line is not None, "MCP process exited before replying to " + method)
            try:
                message = json.loads(line)
            except (ValueError, UnicodeDecodeError):
                raise ContractError("MCP stdout was not a JSON-RPC message") from None
            if message.get("id") == self.sequence:
                return message
            require(time.monotonic() < deadline, "MCP response timed out for " + method)

    def _send(self, message):
        try:
            self.process.stdin.write(json.dumps(message).encode() + b"\n")
            self.process.stdin.flush()
        except (BrokenPipeError, OSError):
            raise ContractError("MCP process closed its input") from None

    def initialize(self):
        response = self.request("initialize", {
            "protocolVersion": "2024-11-05", "capabilities": {},
            "clientInfo": {"name": "pcp-local-client-validation", "version": "1"},
        })
        require("result" in response, "generated MCP configuration could not initialize")
        self._send({"jsonrpc": "2.0", "method": "notifications/initialized"})
        response = self.request("tools/list", {})
        require("result" in response, "MCP tools/list failed")
        tools = {tool["name"] for tool in response["result"].get("tools", [])}
        require({"pcp_whoami", "pcp_capture", "pcp_read_pages"} <= tools,
                "generated MCP configuration lacks expected context tools")
        return tools

    def tool(self, name, arguments):
        return self.request("tools/call", {"name": name, "arguments": arguments})

    def data(self, name, arguments):
        response = self.tool(name, arguments)
        require("result" in response and not response["result"].get("isError", False),
                "MCP tool failed: " + name)
        result = response["result"]
        if isinstance(result.get("structuredContent"), dict):
            return result["structuredContent"]
        for content in result.get("content", []):
            if content.get("type") == "text":
                try:
                    return json.loads(content["text"])
                except (KeyError, ValueError):
                    continue
        raise ContractError("MCP tool returned no structured JSON: " + name)

    def close(self):
        stop_owned(self.process)
        for stream in (self.process.stdin, self.process.stdout):
            stream.close()
        self.stderr.close()
        self.reader.join(timeout=1)

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()


class LocalClientHarness:
    def __init__(self, arguments):
        self.arguments = arguments
        temp_parent = "/private/tmp" if Path("/private/tmp").is_dir() else "/tmp"
        self.directory = Path(tempfile.mkdtemp(prefix="pcp-lc-", dir=temp_parent)).resolve()
        self.home = self.directory / "home"
        self.infra = self.directory / "infra"
        self.logs = self.directory / "logs"
        for directory in (self.home, self.infra, self.logs, self.home / "bin"):
            directory.mkdir(parents=True, exist_ok=True, mode=0o700)
        # Short temp paths keep macOS Unix socket paths within their kernel limit.
        (self.home / "bin" / "pcp-mcp").symlink_to(arguments.mcp_binary)
        self.environment = clean_environment(os.environ)
        self.environment.update({
            "PCP_HOME": str(self.home), "INFRA_PROTOCOL_RUNTIME_DIR": str(self.infra),
            "PCP_OBSERVER_ENABLED": "1", "PCP_ENROLLMENT_ENABLED": "1",
        })
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        self.console = None
        self.console_log = None
        self.console_generation = 0
        self.http_responses = []
        self.secrets = set()
        self.checks = []
        self.stage = "prepare isolated processes"

    def checkpoint(self, name):
        self.checks.append(name)
        self.stage = name

    def start_console(self):
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        self.url = "http://127.0.0.1:" + str(port)
        environment = dict(self.environment)
        environment.update({"PCP_CONSOLE_BIND": "127.0.0.1:" + str(port),
                            "PCP_OBSERVER_CONSOLE_URL": self.url})
        self.console_generation += 1
        self.console_log = (self.logs / ("console-" + str(self.console_generation) + ".log")).open("wb")
        self.console = subprocess.Popen(
            [str(self.arguments.console_binary), "--managed", "--home", str(self.home),
             "--runtime-binary", str(self.arguments.runtime_binary)],
            cwd=self.directory, env=environment, stdout=self.console_log,
            stderr=subprocess.STDOUT, start_new_session=True,
        )
        deadline = time.monotonic() + self.arguments.timeout
        while time.monotonic() < deadline:
            require(self.console.poll() is None, "isolated Console exited during startup; inspect its private log")
            try:
                status, _ = self.http("GET", "/api/health")
                if status == 204:
                    break
            except (OSError, urllib.error.URLError):
                pass
            time.sleep(0.1)
        else:
            raise ContractError("isolated Console did not become healthy")
        runtime = self.ok("GET", "/api/runtime")
        lifecycle = runtime.get("lifecycle", {})
        require(runtime.get("reachable") and lifecycle.get("managed") and lifecycle.get("ownsProcess"),
                "Console does not own a healthy isolated Runtime")
        require(Path(lifecycle.get("home", "")).resolve() == self.home,
                "Console Runtime home differs from the isolated home")
        source = (self.home / "config" / "runtime.toml").read_text()
        require(not re.search(r"^\s*\[(maintenance|semantic_search|intent_match)\]", source, re.M),
                "isolated Runtime unexpectedly enabled an external model facility")
        return lifecycle["pid"]

    def stop_console(self):
        stop_owned(self.console)
        self.console = None
        if self.console_log:
            self.console_log.close()
            self.console_log = None

    def http(self, method, path, body=None, *, mutation=True):
        headers = {}
        if method == "POST" and mutation:
            headers["x-pcp-console"] = "1"
        payload = None
        if body is not None:
            payload = json.dumps(body).encode()
            headers["Content-Type"] = "application/json"
        request = urllib.request.Request(self.url + path, payload, headers, method=method)
        try:
            response = self.opener.open(request, timeout=self.arguments.timeout)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            status, raw = response.status, response.read()
        self.http_responses.append(raw)
        if not raw:
            return status, None
        try:
            data = json.loads(raw)
        except (ValueError, UnicodeDecodeError):
            # Router rejections (for example a percent-encoded path separator)
            # may use Axum's plain-text extractor response. They must still be 4xx.
            if 400 <= status < 500:
                return status, None
            raise ContractError("Console returned a non-JSON response for " + path) from None
        public_fields(data)
        return status, data

    def ok(self, method, path, body=None, **options):
        status, data = self.http(method, path, body, **options)
        require(200 <= status < 300, "Console request failed: " + method + " " + path + " (HTTP " + str(status) + ")")
        return data

    def denied(self, method, path, body=None, **options):
        status, _ = self.http(method, path, body, **options)
        require(400 <= status < 500, "Console must reject " + method + " " + path + " with a client error")

    @staticmethod
    def payload(name, mode="read"):
        return {"operationId": str(uuid.uuid4()), "displayName": name,
                "mode": mode, "scopes": ["user:self"]}

    def view(self, value, payload, status):
        require(isinstance(value, dict) and value.get("status") == status,
                "local client has unexpected status; expected " + status)
        try:
            require(str(uuid.UUID(value["id"])) == value["id"], "local client id is not canonical")
        except (KeyError, ValueError, TypeError):
            raise ContractError("local client id is not a UUID") from None
        require(value.get("displayName") == payload["displayName"], "local client display name changed")
        require(value.get("mode") == payload["mode"] and value.get("scopes") == payload["scopes"],
                "local client changed the explicitly requested grant")
        require(value.get("principalId", "").startswith("client:local:"),
                "local client did not receive its own tenant Principal")
        require(value.get("canResumeRegistration") is False,
                "pending, active, or terminal client incorrectly offers registration resume")
        return value

    def enrollment_members(self):
        snapshot = self.ok("GET", "/api/enrollment").get("result", {})
        require(snapshot.get("status") == "snapshot", "operator enrollment snapshot is unavailable")
        return {kind: sorted((entry[id_field], entry["client"]["principal"]["principalId"])
                             for entry in snapshot[kind])
                for kind, id_field in (("pending", "request_id"), ("registrations", "registration_id"))}

    @staticmethod
    def fixture_file(path, value):
        # These synthetic files exist exclusively in the newly created test home.
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "w") as output:
            json.dump(value, output)
            output.flush()
            os.fsync(output.fileno())

    def interrupted_registration(self, template):
        for prepared in (False, True):
            payload = self.payload("Interrupted " + ("prepared credential" if prepared else "profile only"))
            directory = self.home / "local-clients" / payload["operationId"]
            directory.mkdir(mode=0o700)
            profile_path = directory / "profile.json"
            self.fixture_file(profile_path, {"version": 1, "input": payload})
            original_profile = profile_path.read_bytes()
            state_path = directory / "enrollment.json"
            prefix = "/api/local-clients/" + payload["operationId"]
            state = None
            if prepared:
                state = dict(template)
                state.pop("request_id", None)
                state.pop("registration_id", None)
                state.update({"principal_id": "client:local:" + secrets.token_hex(16),
                              "principal_name": payload["displayName"], "credential": secrets.token_hex(32)})
                self.secrets.add(state["credential"])
                self.fixture_file(state_path, state)
            original_state = state_path.read_bytes() if prepared else None
            before = self.enrollment_members()
            for _ in range(2):
                view = self.ok("GET", prefix)
                require(view.get("status") == "unavailable" and view.get("canResumeRegistration") is True,
                        "interrupted local setup does not offer explicit safe registration resume")
                if prepared:
                    require(view.get("principalId") == state["principal_id"], "GET replaced the prepared client identity")
                self.denied("GET", prefix + "/config")
            self.denied("POST", "/api/local-clients", dict(payload, displayName="Changed interrupted request"))
            require(profile_path.read_bytes() == original_profile, "GET or conflicting retry changed the interrupted profile")
            require((state_path.read_bytes() if state_path.exists() else None) == original_state,
                    "GET or conflicting retry prepared or changed a private credential")
            require(self.enrollment_members() == before, "GET submitted an interrupted local enrollment")

            if prepared:
                # Malformed private state is uncertain and must never advertise a safe retry.
                state_path.write_text("{invalid private fixture")
                uncertain = self.ok("GET", prefix)
                require(uncertain.get("status") == "unavailable" and uncertain.get("canResumeRegistration") is False,
                        "malformed private state incorrectly offers registration resume")
                require(self.enrollment_members() == before, "GET submitted enrollment from malformed private state")
                state_path.write_bytes(original_state)

            resumed = self.view(self.ok("POST", "/api/local-clients", payload), payload, "pending")
            repeated = self.view(self.ok("POST", "/api/local-clients", payload), payload, "pending")
            require(resumed["id"] == repeated["id"] == payload["operationId"]
                    and resumed.get("requestId") == repeated.get("requestId"),
                    "explicit registration resume did not reuse the durable operation")
            resumed_state = self.state(resumed)
            if prepared:
                require(resumed["principalId"] == state["principal_id"]
                        and resumed_state["credential"] == state["credential"],
                        "explicit registration resume replaced the prepared identity or credential")
            after = self.enrollment_members()
            require(after["registrations"] == before["registrations"]
                    and len(after["pending"]) == len(before["pending"]) + 1,
                    "explicit registration resume did not create exactly one pending request")
            self.denied("GET", prefix + "/config")
        self.checkpoint("interrupted local setup stays read-only until explicit idempotent registration resume")

    def state(self, view):
        directory = self.home / "local-clients" / view["id"]
        for parent in (self.home / "local-clients", directory):
            private_path(parent, self.home, directory=True)
        path = directory / "enrollment.json"
        private_path(path, self.home)
        for child in directory.iterdir():
            private_path(child, self.home, directory=child.is_dir())
        data = json.loads(path.read_text())
        credential = data.get("credential")
        require(isinstance(credential, str) and len(credential) >= 16,
                "private enrollment state lacks a random credential")
        self.secrets.add(credential)
        require(data.get("principal_id") == view["principalId"], "private enrollment Principal mismatch")
        return data

    def configuration(self, view):
        result = self.ok("GET", "/api/local-clients/" + view["id"] + "/config")
        require(result.get("client", {}).get("status") == "ready", "config did not attest a ready client")
        servers = result.get("config", {}).get("mcpServers", {})
        require(len(servers) == 1, "generated config must name one MCP server")
        key, config = next(iter(servers.items()))
        require(key != "pcp" and key.startswith("pcp-"), "generated config uses a colliding generic server key")
        require(set(config) == {"command", "args", "env"} and config["args"] == [],
                "generated MCP command has unexpected arguments")
        command = Path(config["command"])
        require(command.is_absolute() and command.resolve() == self.arguments.mcp_binary,
                "generated command did not select the explicit MCP binary")
        expected_env = {"PCP_CLIENT_ID": view["principalId"],
                        "PCP_ENROLLMENT_FILE": str(self.home / "local-clients" / view["id"] / "enrollment.json"),
                        "PCP_MCP_SURFACE": "generic", "PCP_MCP_TOOLSET": "context"}
        require(config["env"] == expected_env, "generated config contains unexpected environment or credential material")
        return result, config, key

    def session(self, config, label):
        # The generated config must carry everything the MCP client needs.
        # Fixture-only PCP_HOME/Discovery overrides belong to Console/Runtime.
        return McpSession(config, clean_environment(os.environ), self.logs, label, self.arguments.timeout)

    @staticmethod
    def grants(session, view):
        data = session.data("pcp_whoami", {})
        access = data.get("access", {})
        require(access.get("principal", {}).get("principalId") == view["principalId"],
                "live MCP session uses a different Principal")
        require(access.get("principal", {}).get("principalType") == "model_client",
                "live MCP session is not a model client")
        require(not access.get("storePermissions"), "tenant MCP unexpectedly has Store-wide authority")
        grants = access.get("grants", [])
        require(len(grants) == 1, "live MCP grants extend beyond the one requested Scope")
        namespace = grants[0].get("namespace", "")
        require(namespace.startswith("user:idn_"), "user:self did not resolve to the Store identity Scope")
        expected = READ_PERMISSIONS | ({"ingest"} if view["mode"] == "contribute" else set())
        require(set(grants[0].get("permissions", [])) == expected, "live MCP permissions differ from the requested mode")
        return namespace

    @staticmethod
    def capture(scope):
        return {"scope": scope, "category": "verified_finding", "title": "Isolated onboarding test",
                "content": "This synthetic finding exists only in the temporary integration Store.",
                "retentionRationale": "Verify the approved local client's exact ingest grant."}

    def no_secrets_exposed(self):
        for secret in self.secrets:
            require(all(secret.encode() not in body for body in self.http_responses),
                    "Console API exposed a private enrollment credential")
            for path in self.logs.iterdir():
                if path.is_file():
                    require(secret.encode() not in path.read_bytes(),
                            "a test process logged a private enrollment credential")

    def run(self):
        first_runtime = self.start_console()
        initial = self.ok("GET", "/api/local-clients")
        require(initial.get("available") is True and initial.get("clients") == [],
                "fresh isolated Console does not offer an empty local-client list")
        self.checkpoint("isolated managed Console and fresh Store")

        self.denied("POST", "/api/local-clients", self.payload("No mutation header"), mutation=False)
        for changes in ({"mode": "admin"}, {"scopes": []}, {"scopes": ["*"]},
                        {"scopes": ["user:self", "user:self"]}, {"principalId": "operator:local"},
                        {"enrollmentFile": "/etc/passwd"}, {"operationId": "../outside"},
                        {"displayName": "测" * 41}):
            request = self.payload("Invalid request")
            request.update(changes)
            self.denied("POST", "/api/local-clients", request)
        self.denied("GET", "/api/local-clients/" + urllib.parse.quote("../outside", safe="") + "/config")
        require(self.ok("GET", "/api/local-clients")["clients"] == [], "invalid requests created a client")
        self.checkpoint("mutation guard and invalid grants, paths, identities, and names rejected")

        read_payload = self.payload("Independent reader")
        reader = self.view(self.ok("POST", "/api/local-clients", read_payload), read_payload, "pending")
        prefix = "/api/local-clients/" + reader["id"]
        pending = self.state(reader)
        require(reader.get("requestId"), "pending local client has no Runtime request")
        for _ in range(2):
            same = self.view(self.ok("POST", "/api/local-clients", read_payload), read_payload, "pending")
            require(same["id"] == reader["id"] and same.get("requestId") == reader["requestId"],
                    "operationId replay created a second identity or enrollment request")
            self.view(self.ok("GET", prefix), read_payload, "pending")
            self.denied("GET", prefix + "/config")
        changed = dict(read_payload, mode="contribute")
        self.denied("POST", "/api/local-clients", changed)
        self.denied("POST", prefix + "/approve", mutation=False)
        self.view(self.ok("GET", prefix), read_payload, "pending")
        require(self.state(reader)["credential"] == pending["credential"], "status polling replaced the private credential")
        require(len(self.ok("GET", "/api/local-clients")["clients"]) == 1, "idempotent begin created duplicate clients")
        self.checkpoint("pending recovery is read-only and duplicate operationId is idempotent")

        reader = self.view(self.ok("POST", prefix + "/approve"), read_payload, "ready")
        require(reader.get("registrationId"), "approved client has no Runtime registration")
        read_result, read_config, read_key = self.configuration(reader)
        same = self.ok("POST", "/api/local-clients", read_payload)
        require(same["id"] == reader["id"] and same.get("status") == "ready", "ready operationId replay changed identity")
        with self.session(read_config, "reader") as session:
            session.initialize()
            scope = self.grants(session, reader)
            denied = session.tool("pcp_capture", self.capture(scope))
            require("error" in denied or denied.get("result", {}).get("isError") is True,
                    "read-only local client unexpectedly ingested a Page")
        self.checkpoint("generated reader config initializes MCP with exact read grants and denies ingest")

        contribute_payload = self.payload("Independent contributor", "contribute")
        contributor = self.view(self.ok("POST", "/api/local-clients", contribute_payload), contribute_payload, "pending")
        require(contributor["principalId"] != reader["principalId"] and contributor["id"] != reader["id"],
                "two local clients share an identity")
        other_state = self.state(contributor)
        require(other_state["credential"] != pending["credential"], "two local clients share a credential")
        contribute_prefix = "/api/local-clients/" + contributor["id"]
        contributor = self.view(self.ok("POST", contribute_prefix + "/approve"), contribute_payload, "ready")
        _, contribute_config, contribute_key = self.configuration(contributor)
        require(read_key != contribute_key, "two generated MCP configurations share a server key")
        with self.session(contribute_config, "contributor") as session:
            session.initialize()
            contribute_scope = self.grants(session, contributor)
            require(contribute_scope == scope, "user:self resolved differently for two clients of one Store")
            written = session.data("pcp_capture", self.capture(scope))
            require(written.get("pageId") and written.get("revisionId"), "contribute grant did not ingest a real Page")
            wrong_scope = session.tool("pcp_capture", self.capture("project:not-granted"))
            require("error" in wrong_scope or wrong_scope.get("result", {}).get("isError") is True,
                    "contribute client wrote outside its approved Scope")
        with self.session(read_config, "reader-content") as session:
            session.initialize()
            content = session.data("pcp_read_pages", {"pageIds": [written["pageId"]]})
            require("temporary integration Store" in json.dumps(content),
                    "read grant could not retrieve the contributor's real Page")
        self.checkpoint("second client has independent identity and credential with exact contribute grants")

        self.interrupted_registration(pending)

        previous_pid = self.ok("GET", "/api/runtime")["lifecycle"]["pid"]
        with self.session(read_config, "reader-across-runtime-restart") as session:
            session.initialize()
            self.grants(session, reader)
            restarted = self.ok("POST", "/api/runtime/restart")
            require(restarted.get("restarted") and restarted.get("pid") != previous_pid,
                    "managed Runtime restart did not replace its process")
            content = session.data("pcp_read_pages", {"pageIds": [written["pageId"]]})
            require("temporary integration Store" in json.dumps(content),
                    "running MCP client did not reconnect after Runtime restart")
        with self.session(read_config, "reader-after-runtime-restart") as session:
            session.initialize()
            require(self.grants(session, reader) == scope, "same config lost its grants after Runtime restart")
        self.checkpoint("unchanged generated config reconnects after managed Runtime restart")

        with self.session(read_config, "reader-across-console-restart") as session:
            session.initialize()
            self.stop_console()
            next_runtime = self.start_console()
            require(next_runtime != first_runtime, "Console restart did not start a new owned Runtime")
            content = session.data("pcp_read_pages", {"pageIds": [written["pageId"]]})
            require("temporary integration Store" in json.dumps(content),
                    "running MCP client did not reconnect after Console restart")
        recovered = self.view(self.ok("GET", prefix), read_payload, "ready")
        require(recovered["principalId"] == reader["principalId"], "Console restart changed client identity")
        next_result, _, _ = self.configuration(recovered)
        require(next_result["config"] == read_result["config"], "Console restart changed the generated MCP config")
        with self.session(read_config, "reader-after-console-restart") as session:
            session.initialize()
            self.grants(session, reader)
        self.checkpoint("Console and Runtime restart preserve credentials and generated config")

        with self.session(read_config, "reader-across-revocation") as session:
            session.initialize()
            self.view(self.ok("POST", prefix + "/revoke"), read_payload, "revoked")
            denied = session.tool("pcp_read_pages", {"pageIds": [written["pageId"]]})
            require("error" in denied or denied.get("result", {}).get("isError") is True,
                    "revoked running MCP client retained access to Page content")
        self.denied("GET", prefix + "/config")
        self.view(self.ok("GET", prefix), read_payload, "revoked")
        revoked_connected = False
        with self.session(read_config, "revoked-reader") as session:
            try:
                session.initialize()
                self.grants(session, reader)
                revoked_connected = True
            except ContractError:
                require(session.process.wait(timeout=self.arguments.timeout) != 0,
                        "revoked MCP client closed without rejecting enrollment")
        require(not revoked_connected, "revoked configuration opened a new authorized MCP session")
        with self.session(contribute_config, "unaffected-contributor") as session:
            session.initialize()
            self.grants(session, contributor)
        self.checkpoint("revocation rejects saved config while independent client remains authorized")
        for view in (reader, contributor):
            self.state(view)
        self.no_secrets_exposed()
        self.checkpoint("client state is private and credentials are absent from API, config, and process logs")

    def close(self):
        self.stop_console()
        if not self.arguments.keep_temp:
            import shutil
            shutil.rmtree(self.directory)


class HarnessTests(unittest.TestCase):
    def test_child_environment_drops_runtime_config_and_provider_credentials(self):
        values = {"HOME": "/original-home", "PATH": "/bin", "PCP_HOME": "/real-pcp",
                  "PCP_STORE_PATH": "/real-store", "PCP_RUNTIME_SOCKET": "/real-socket",
                  "INFRA_PROTOCOL_RUNTIME_DIR": "/real-infra", "OPENAI_API_KEY": "unused"}
        self.assertEqual(clean_environment(values), {"HOME": "/original-home", "PATH": "/bin"})

    def test_public_response_secret_fields_are_rejected_recursively(self):
        for name in ("credential", "password", "clientSecret", "access_token"):
            with self.subTest(name=name), self.assertRaises(ContractError):
                public_fields({"config": [{name: "redacted"}]})
        public_fields({"env": {"PCP_ENROLLMENT_FILE": "/private/test/enrollment.json"}})

    def test_private_state_rejects_symlink_and_open_modes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            state = root / "state"
            state.write_text("private fixture")
            state.chmod(0o600)
            private_path(state, root)
            alias = root / "alias"
            alias.symlink_to(state)
            with self.assertRaises(ContractError):
                private_path(alias, root)
            state.chmod(0o644)
            with self.assertRaises(ContractError):
                private_path(state, root)


def parse_arguments():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--console-binary", type=Path, help="built pcp-console (default target/debug/pcp-console)")
    parser.add_argument("--runtime-binary", type=Path, help="built pcp-runtime (default sibling of Console)")
    parser.add_argument("--mcp-binary", type=Path, help="built pcp-mcp (default sibling of Console)")
    parser.add_argument("--timeout", type=float, default=40, help="startup/RPC timeout in seconds")
    parser.add_argument("--report", type=Path, help="save a redacted JSON validation report")
    parser.add_argument("--keep-temp", action="store_true", help="retain private test home and logs")
    parser.add_argument("--self-test", action="store_true", help="check harness helpers without starting PCP")
    arguments = parser.parse_args()
    if arguments.self_test:
        return arguments
    require(os.name == "posix", "this harness requires Unix process groups and file permissions")
    require(arguments.timeout > 0, "--timeout must be positive")
    root = Path(__file__).resolve().parent.parent
    console = arguments.console_binary or root / "target" / "debug" / "pcp-console"
    arguments.console_binary = console.resolve()
    arguments.runtime_binary = (arguments.runtime_binary or console.parent / "pcp-runtime").resolve()
    arguments.mcp_binary = (arguments.mcp_binary or console.parent / "pcp-mcp").resolve()
    for name in ("console_binary", "runtime_binary", "mcp_binary"):
        path = getattr(arguments, name)
        require(path.is_file() and os.access(path, os.X_OK), "missing executable --" + name.replace("_", "-"))
    return arguments


def main():
    arguments = parse_arguments()
    if arguments.self_test:
        result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(HarnessTests))
        return 0 if result.wasSuccessful() else 1
    report = {"schema": "pcp.console.local-client.validation.v1", "passed": False,
              "binaries": {name: {"path": str(getattr(arguments, name)),
                                   "sha256": digest_file(getattr(arguments, name))}
                           for name in ("console_binary", "runtime_binary", "mcp_binary")}}
    harness = LocalClientHarness(arguments)
    try:
        harness.run()
        report["passed"] = True
    except (ContractError, OSError, subprocess.TimeoutExpired) as error:
        # Deliberately omit subprocess stderr and HTTP/RPC bodies from diagnostics.
        report["failureStage"] = harness.stage
        report["error"] = str(error) if isinstance(error, ContractError) else type(error).__name__
    finally:
        report["checks"] = harness.checks
        if arguments.keep_temp:
            report["privateTestDirectory"] = str(harness.directory)
        harness.close()
    if arguments.report:
        with arguments.report.open("w") as target:
            json.dump(report, target, indent=2)
            target.write("\n")
    print(json.dumps(report, indent=2))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except ContractError as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
