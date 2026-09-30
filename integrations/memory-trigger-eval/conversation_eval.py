#!/usr/bin/env python3
"""Prepare blind conversation continuations and score raw Responses tool calls.

No model calls or PCP writes. Supply an MCP initialize/tools-list snapshot from the
binary under test, and pass each prepared request to a provider without executing
its proposed tools. Expectations and case names never enter model input.
"""
import argparse
import hashlib
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
LOCAL_FIXTURES = HERE / "conversations.json"
WRITES = {"pcp_capture", "pcp_submit_candidate", "pcp_publish_activity",
          "pcp_submit_feedback", "pcp_ensure_project_scope"}


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, ensure_ascii=False).encode()).hexdigest()


def fixtures(path=LOCAL_FIXTURES):
    path = Path(path)
    if not path.is_file():
        raise ValueError(
            f"Local conversation fixtures are missing: {path}. "
            "Datasets are not bundled; supply --fixtures with your local JSON file. "
            "See integrations/memory-trigger-eval/README.md."
        )
    cases = json.loads(path.read_text())
    if not cases or len({c["id"] for c in cases}) != len(cases):
        raise ValueError("Fixture IDs must be unique and nonempty")
    return cases


def prepare(cases, catalog, agents):
    # The catalog is the actual wire representation, not a second schema copy.
    names = {t["name"] for t in catalog["tools"]}
    if not WRITES | {"pcp_read_activity", "pcp_whoami", "pcp_list_scopes"} <= names:
        raise ValueError("A context toolset catalog with project registration is required")
    tools = [{"type": "function", "name": t["name"],
              "description": t.get("description", ""), "parameters": t["inputSchema"]}
             for t in catalog["tools"]]
    probes = []
    for i, case in enumerate(cases):
        # Session facts are synthetic permissions/identity/receipts, never expected actions.
        history = [{"role": "developer", "content": json.dumps(case["session"], ensure_ascii=False)}]
        history.extend(case["input"])
        probes.append({"id": f"probe-{i + 1:03d}", "request": {
            "instructions": agents + "\n\n" + catalog["instructions"],
            "input": history, "tools": tools,
        }})
    return {"format": "pcp-conversation-probes-v1", "fixturesSha256": digest(cases),
            "catalogSha256": digest(catalog), "agentsSha256": digest(agents), "probes": probes}


def tool_calls(response):
    if response.get("status") != "completed" or not isinstance(response.get("output"), list):
        raise ValueError("Every probe needs a completed raw Responses result")
    calls = []
    for item in response["output"]:
        if item.get("type") == "function_call":
            args = item.get("arguments")
            if isinstance(args, str):
                args = json.loads(args)
            if not isinstance(args, dict) or not isinstance(item.get("name"), str):
                raise ValueError("Invalid function call")
            calls.append({"name": item["name"], "arguments": args})
    return calls


def score(cases, prepared, results):
    if prepared.get("fixturesSha256") != digest(cases):
        raise ValueError("Prepared fixture identity differs")
    if results.get("preparedSha256") != digest(prepared):
        raise ValueError("Results do not identify these exact prepared inputs")
    rows = results["responses"]
    ids = [p["id"] for p in prepared["probes"]]
    if len(rows) != len(ids) or {r["id"] for r in rows} != set(ids):
        raise ValueError("Each prepared probe must have exactly one response")
    responses = {r["id"]: r["response"] for r in rows}
    report = {"probes": len(cases), "activityOpportunities": 0, "activityCaptured": 0,
              "unexpectedWrites": 0, "argumentFailures": 0, "passed": 0, "failures": []}
    for case, probe in zip(cases, prepared["probes"], strict=True):
        calls = tool_calls(responses[probe["id"]])
        expected = case["expect"]
        names = [c["name"] for c in calls]
        allowed = expected.get("allowed", [])
        required = expected.get("required")
        errors = []
        if required == "pcp_publish_activity":
            report["activityOpportunities"] += 1
            report["activityCaptured"] += names.count(required) == 1
        if required and names.count(required) != 1:
            errors.append(f"expected exactly one {required}")
        for call in calls:
            name, args = call["name"], call["arguments"]
            if name != required and name not in allowed:
                errors.append(f"unexpected call: {name}")
                report["unexpectedWrites"] += name in WRITES
            if name != required:
                continue
            argument_errors = []
            for key, value in expected.get("arguments", {}).items():
                if args.get(key) != value:
                    argument_errors.append(f"{key} differs from known session state")
            for key in expected.get("absent", []):
                if args.get(key) is not None:
                    argument_errors.append(f"{key} must be omitted")
            if name == "pcp_publish_activity":
                summary, topic = args.get("summary"), args.get("topicKey")
                if not isinstance(summary, str) or not 1 <= len(summary.strip()) <= 180:
                    argument_errors.append("summary must be 1..180 characters")
                if not isinstance(topic, str) or not 1 <= len(topic.strip()) <= 64:
                    argument_errors.append("topicKey must be 1..64 characters")
                if args.get("scope") not in (None, case["session"].get("projectScope")):
                    argument_errors.append("scope must match project ownership")
            if argument_errors:
                report["argumentFailures"] += 1
                errors.extend(argument_errors)
        # Repeated identical actions must not earn extra opportunity credit.
        if len(names) != len(set(names)):
            errors.append("duplicate call in one continuation")
        if errors:
            report["failures"].append({"id": case["id"], "probe": probe["id"],
                                       "errors": errors, "calls": calls})
        else:
            report["passed"] += 1
    report["scope"] = ("Blind synthetic multi-turn continuations with real metadata; "
                       "not live host discovery, executed writes or longitudinal recall. "
                       "Review summary meaning and attribution separately.")
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    p = commands.add_parser("prepare")
    p.add_argument("--fixtures", type=Path, default=LOCAL_FIXTURES,
                   help="Local conversation JSON; datasets are not bundled")
    p.add_argument("--catalog", type=Path, required=True)
    p.add_argument("--agents", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    p = commands.add_parser("score")
    p.add_argument("--fixtures", type=Path, default=LOCAL_FIXTURES,
                   help="The same local conversation JSON used for prepare")
    p.add_argument("prepared", type=Path)
    p.add_argument("results", type=Path)
    args = parser.parse_args()
    try:
        cases = fixtures(args.fixtures)
    except (OSError, ValueError) as error:
        parser.error(str(error))
    if args.command == "prepare":
        output = prepare(cases, json.loads(args.catalog.read_text()), args.agents.read_text())
        args.output.write_text(json.dumps(output, ensure_ascii=False, indent=2) + "\n")
        print(json.dumps({"probes": len(cases), "preparedSha256": digest(output)}))
    else:
        report = score(cases, json.loads(args.prepared.read_text()), json.loads(args.results.read_text()))
        print(json.dumps(report, ensure_ascii=False, indent=2))
        if report["failures"]:
            raise SystemExit(1)


if __name__ == "__main__":
    main()
