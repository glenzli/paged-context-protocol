#!/usr/bin/env python3
"""Parse every Console client export and use it against isolated real PCP services.

Requires Node.js, Python 3.11+ and PyYAML. Uses test_console_local_clients.py's
private process harness; never edits an installed client's settings or user Store.
Optional --codex-binary and --antigravity-binary check installed client parsers in
fresh temporary homes, without a model call or copying authentication state.
Build PCP first, then run:
  python3 scripts/test_console_client_exports.py --report /tmp/pcp-exports.json
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib
import urllib.request

import yaml
from test_console_local_clients import LocalClientHarness, require, digest_file, clean_environment

ROOT = Path(__file__).resolve().parent.parent
MODULE = ROOT / "crates/pcp-console/src/local-client-config.js"


def export_all(config, node):
    script = """
import fs from 'node:fs';
const module = await import(process.argv[1]);
const config = JSON.parse(fs.readFileSync(0, 'utf8'));
console.log(JSON.stringify(module.LOCAL_CLIENT_TARGETS.map(target => ({id:target.id,
  ...module.renderLocalClientConfiguration(config,target.id)}))));
"""
    result = subprocess.run([node, "--input-type=module", "-e", script, MODULE.as_uri()],
                            input=json.dumps(config), text=True, capture_output=True, timeout=15)
    require(result.returncode == 0, "client export module failed")
    return json.loads(result.stdout)


def decode(artifact):
    name, target = artifact["serverName"], artifact["id"]
    if target == "codex":
        return tomllib.loads(artifact["text"])["mcp_servers"][name]
    if target == "dsh":
        patch = yaml.safe_load(artifact["text"])
        require(len(patch) == 1 and len(patch[0]["insert"]) == 1, "DSH export is not one insert patch")
        plugin = patch[0]["insert"][0]
        require(plugin["name"] == "@deepseek-ai/dsh-mcp-client", "DSH export uses the wrong plugin")
        entry = plugin["config"]
        require(entry["transport"] == "stdio" and entry["serverName"] == name and len(name) <= 32,
                "DSH export has an invalid transport or namespace")
        return {key: entry[key] for key in ("command", "args", "env")}
    body = json.loads(artifact["text"])
    if target == "vscode":
        entry = body["servers"][name]
        require(entry.pop("type") == "stdio", "VS Code export is not stdio")
        return entry
    if target.startswith("opencode-"):
        entry = (body["mcp"]["servers"] if target == "opencode-v2" else body["mcp"])[name]
        require(entry["type"] == "local", "OpenCode export is not local")
        return {"command": entry["command"][0], "args": entry["command"][1:], "env": entry["environment"]}
    return body["mcpServers"][name]


def parser_checks(exports, node):
    # Exercise real TOML/YAML decoders with strings that break naive quoting.
    server = {"command": "/tmp/测试 person's PCP/bin", "args": ["a b", 'quote"', "line\nnext", "\\backslash", "'$HOME`echo`$(echo)"],
              "env": {"PCP_CLIENT_ID": "client:test", "PCP_ENROLLMENT_FILE": "/tmp/测试's/enrollment.json", "PCP_MCP_SURFACE": "generic", "PCP_MCP_TOOLSET": "context"}}
    name = exports[0]["serverName"]
    for artifact in export_all({"mcpServers": {name: server}}, node):
        require(decode(artifact) == server, "roundtrip changed quoted arguments for " + artifact["id"])


def installed_parser(binary, target, artifact, directory, expected):
    home = directory / (target + "-parser-home")
    home.mkdir(mode=0o700)
    environment = clean_environment(os.environ)
    environment["HOME"] = str(home)
    if target == "codex":
        config_home = home / "codex"
        config_home.mkdir(mode=0o700)
        (config_home / "config.toml").write_text(artifact["text"])
        environment["CODEX_HOME"] = str(config_home)
        result = subprocess.run([str(binary), "mcp", "list", "--json"], cwd=home, env=environment,
                                capture_output=True, text=True, timeout=30)
        require(result.returncode == 0, "installed Codex could not parse exported TOML")
        entries = json.loads(result.stdout)
        entry = next((item for item in entries if item.get("name") == artifact["serverName"]), None)
        require(entry is not None, "Codex did not load the exported server")
        transport = entry.get("transport", {})
        require(transport.get("command") == expected["command"] and transport.get("args", []) == expected["args"]
                and transport.get("env") == expected["env"], "Codex changed the exported launch specification")
    else:
        config_home = home / ".gemini" / "config"
        config_home.mkdir(mode=0o700, parents=True)
        (config_home / "mcp_config.json").write_text(artifact["text"])
        result = subprocess.run([str(binary), "mcp", "list"], cwd=home, env=environment,
                                capture_output=True, text=True, timeout=30)
        require(result.returncode == 0 and artifact["serverName"] in result.stdout,
                "installed Antigravity did not load exported mcp_config.json")
    return {"client": target, "level": "installed configuration parser", "binarySha256": digest_file(binary)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("console", "runtime", "mcp"):
        parser.add_argument("--" + name + "-binary", type=Path, default=ROOT / "target/release" / ("pcp-" + name))
    parser.add_argument("--node", default=shutil.which("node"))
    parser.add_argument("--codex-binary", type=Path)
    parser.add_argument("--antigravity-binary", type=Path)
    parser.add_argument("--timeout", type=float, default=30)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    require(args.node is not None, "Node.js is required")
    for name in ("console", "runtime", "mcp"):
        setattr(args, name + "_binary", getattr(args, name + "_binary").resolve())
    harness = LocalClientHarness(args)
    report = {"passed": False, "exports": [], "installedClientChecks": [], "productionStateUsed": False}
    try:
        harness.start_console()
        with urllib.request.urlopen(harness.url + "/local-client-config.js", timeout=args.timeout) as response:
            require(response.read() == MODULE.read_bytes(), "packaged Console serves a different export module")
        payload = harness.payload("Export verification reader")
        profile = harness.ok("POST", "/api/local-clients", payload)
        prefix = "/api/local-clients/" + profile["id"]
        profile = harness.ok("POST", prefix + "/approve")
        canonical, expected, _ = harness.configuration(profile)
        artifacts = export_all(canonical["config"], args.node)
        parser_checks(artifacts, args.node)
        # Seed a real Page through a separate contributor in this temporary Store.
        writer = harness.ok("POST", "/api/local-clients", harness.payload("Export fixture contributor", "contribute"))
        writer = harness.ok("POST", "/api/local-clients/" + writer["id"] + "/approve")
        _, writer_config, _ = harness.configuration(writer)
        with harness.session(writer_config, "export-fixture") as session:
            session.initialize()
            namespace = harness.grants(session, writer)
            written = session.data("pcp_capture", harness.capture(namespace))
        for artifact in artifacts:
            decoded = decode(artifact)
            require(decoded == expected, "export changed the approved launch specification: " + artifact["id"])
            with harness.session(decoded, "export-" + artifact["id"]) as session:
                session.initialize()
                require(harness.grants(session, profile) == namespace, "export changed the approved Scope")
                content = session.data("pcp_read_pages", {"pageIds": [written["pageId"]]})
                require("temporary integration Store" in json.dumps(content), "export could not read the real fixture Page")
                denied = session.tool("pcp_capture", harness.capture(namespace))
                require("error" in denied or denied.get("result", {}).get("isError") is True, "reader export acquired write access")
            report["exports"].append({"client": artifact["id"], "parsed": True, "identityAndGrant": True,
                                      "readFixturePage": True, "writeDenied": True})
        for client, binary in (("codex", args.codex_binary), ("antigravity", args.antigravity_binary)):
            if binary:
                artifact = next(item for item in artifacts if item["id"] == client)
                report["installedClientChecks"].append(installed_parser(binary.resolve(), client, artifact, harness.directory, expected))
        report["passed"] = True
        report["sourceSha256"] = digest_file(MODULE)
        report["binarySha256"] = {name: digest_file(getattr(args, name + "_binary")) for name in ("console", "runtime", "mcp")}
    finally:
        harness.stop_console()
        shutil.rmtree(harness.directory)
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"passed": report["passed"], "exportCount": len(report["exports"]), "installedClientChecks": report["installedClientChecks"]}))


if __name__ == "__main__":
    main()
