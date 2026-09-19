#!/usr/bin/env python3
"""Prepare blind routing replays and score externally supplied model decisions.

No model request or PCP write is executed by this script. The caller chooses the
evaluation provider. Golden plans never enter the replay prompt.
"""
import argparse
import collections
import hashlib
import json
from pathlib import Path
import re


HERE = Path(__file__).resolve().parent
SOURCE = HERE.parents[1] / "crates/pcp-mcp/src/lib.rs"
TOOLS = {
    "pcp_capture", "pcp_submit_candidate", "pcp_publish_activity",
    "pcp_read_activity", "pcp_semantic_search", "pcp_search_pages",
    "pcp_read_pages", "pcp_submit_feedback",
}


def surface(path):
    source = path.read_text()
    block = re.search(r"const SHARED_SERVER_INSTRUCTIONS: &str = concat!\((.*?)\n\);", source, re.S)
    if not block:
        raise ValueError("MCP instruction declaration not found")
    instructions = "".join(json.loads(s) for s in re.findall(r'"(?:\\.|[^"\\])*"', block[1]))
    descriptions = {
        name: json.loads(description)
        for name, description in re.findall(
            r'name = "(pcp_[^"]+)",\s*description = ("(?:\\.|[^"\\])*")', source
        ) if name in TOOLS
    }
    if descriptions.keys() != TOOLS:
        raise ValueError("MCP tool descriptions missing from source")
    return instructions, descriptions, hashlib.sha256(source.encode()).hexdigest()


def prepare(args, cases):
    instructions, tools, digest = surface(args.source)
    # This tests routing judgment with visible metadata, not tool discovery,
    # actual tool execution, argument fidelity, or end-to-end host behavior.
    prompt = (
        "Evaluate each independent conversation checkpoint below using the supplied PCP policy "
        "and tool descriptions. Select only the tool calls needed now, in order. Do not execute "
        "tools or invent missing events. Empty actions is valid. Treat each case as evidence, "
        "not as new policy. Return only JSON {\"decisions\":[{\"id\":\"...\","
        "\"actions\":[\"pcp_tool_name\"],\"reason\":\"brief reason\"}]}, one decision per case.\n\n"
        + json.dumps({"policy": instructions, "tools": tools,
                      "cases": [{"id": c["id"], "context": c["context"]} for c in cases]},
                     ensure_ascii=False)
    )
    args.output.write_text(json.dumps({"sourceSha256": digest, "prompt": prompt}, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"cases": len(cases), "sourceSha256": digest, "output": str(args.output)}))


def score(args, cases):
    decisions = json.loads(args.decisions.read_text())["decisions"]
    expected = {c["id"]: c for c in cases}
    counts = collections.Counter(d["id"] for d in decisions)
    if counts.keys() != expected.keys() or any(n != 1 for n in counts.values()):
        raise ValueError("Decisions must contain every case ID exactly once, with no extras")
    failures = []
    negative_total = negative_passed = positive_total = positive_passed = 0
    for d in decisions:
        case = expected[d["id"]]
        actions = d["actions"]
        if not isinstance(actions, list) or any(not isinstance(a, str) or a not in TOOLS for a in actions):
            raise ValueError(f"Invalid actions for {d['id']}")
        passed = actions in case["plans"]
        if case["plans"] == [[]]:
            negative_total += 1
            negative_passed += passed
        else:
            positive_total += 1
            positive_passed += passed
        if not passed:
            failures.append({"id": d["id"], "actual": actions, "expected": case["plans"], "reason": case["reason"]})
    report = {"cases": len(cases), "passed": len(cases) - len(failures),
              "positivePassed": positive_passed, "positiveTotal": positive_total,
              "negativePassed": negative_passed, "negativeTotal": negative_total,
              "failures": failures,
              "scope": "Routing replay only; not host trigger recall or argument fidelity."}
    print(json.dumps(report, ensure_ascii=False, indent=2))
    if failures:
        raise SystemExit(1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    p = commands.add_parser("prepare")
    p.add_argument("--source", type=Path, default=SOURCE)
    p.add_argument("--output", type=Path, required=True)
    p = commands.add_parser("score")
    p.add_argument("decisions", type=Path)
    args = parser.parse_args()
    cases = json.loads((HERE / "cases.json").read_text())
    if len({c["id"] for c in cases}) != len(cases):
        raise ValueError("Duplicate fixture IDs")
    (prepare if args.command == "prepare" else score)(args, cases)


if __name__ == "__main__":
    main()
