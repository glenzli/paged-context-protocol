import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from conversation_eval import LOCAL_FIXTURES, digest, fixtures, prepare, score
from evaluate import surface


@unittest.skipUnless(
    LOCAL_FIXTURES.is_file(),
    "Local conversations.json is not bundled; see memory-trigger-eval/README.md",
)
class ConversationReplayTests(unittest.TestCase):
    def setUp(self):
        self.cases = fixtures()
        self.catalog = {"instructions": "fixture policy", "tools": [
            {"name": name, "inputSchema": {"type": "object"}}
            for name in ["pcp_capture", "pcp_submit_candidate", "pcp_publish_activity",
                         "pcp_read_activity", "pcp_submit_feedback", "pcp_ensure_project_scope",
                         "pcp_whoami", "pcp_list_scopes"]]}
        self.prepared = prepare(self.cases, self.catalog, "fixture AGENTS")
        self.results = {"preparedSha256": digest(self.prepared), "responses": []}
        for case, probe in zip(self.cases, self.prepared["probes"]):
            expected = case["expect"]
            output = []
            if expected["required"]:
                args = dict(expected["arguments"])
                if expected["required"] == "pcp_publish_activity":
                    args.setdefault("topicKey", "alder:example")
                    args["summary"] = "仍在比较两种解释，暂未定论；下一步检查边界。"
                output.append({"type": "function_call", "name": expected["required"],
                               "arguments": json.dumps(args)})
            self.results["responses"].append({"id": probe["id"], "response": {
                "status": "completed", "output": output}})

    def test_model_input_excludes_expected_actions_and_case_labels(self):
        for case, probe in zip(self.cases, self.prepared["probes"]):
            text = json.dumps(probe["request"])
            self.assertNotIn('"expect"', text)
            self.assertNotIn(case["id"], text)
            self.assertEqual(probe["request"]["input"][1:], case["input"])
        self.assertNotIn("checkpoint", json.dumps([c["input"] for c in self.cases]))

    def test_correct_calls_and_nonwrite_continuations_pass(self):
        result = score(self.cases, self.prepared, self.results)
        self.assertEqual(result["passed"], len(self.cases))
        self.assertEqual(result["activityCaptured"], result["activityOpportunities"])

    def test_wrong_project_version_and_scope_fail(self):
        output = self.results["responses"][1]["response"]["output"][0]
        args = json.loads(output["arguments"])
        args.update(projectKey="example.test/wrong", expectedVersion=8, scope="user:example")
        output["arguments"] = json.dumps(args)
        result = score(self.cases, self.prepared, self.results)
        self.assertEqual(result["argumentFailures"], 1)
        self.assertEqual(len(result["failures"][0]["errors"]), 3)

    def test_noop_and_duplicate_writes_are_not_successes(self):
        output = self.results["responses"][0]["response"]["output"]
        output.append(copy.deepcopy(output[0]))
        self.results["responses"][-1]["response"]["output"] = [
            {"type": "function_call", "name": "pcp_capture", "arguments": "{}"}]
        result = score(self.cases, self.prepared, self.results)
        self.assertEqual(result["unexpectedWrites"], 1)
        self.assertEqual(len(result["failures"]), 2)
        self.assertLess(result["activityCaptured"], result["activityOpportunities"])

    def test_stale_partial_duplicate_or_failed_results_rejected(self):
        variants = []
        stale = copy.deepcopy(self.results); stale["preparedSha256"] = "wrong"; variants.append(stale)
        partial = copy.deepcopy(self.results); partial["responses"].pop(); variants.append(partial)
        duplicate = copy.deepcopy(self.results); duplicate["responses"][-1] = duplicate["responses"][0]; variants.append(duplicate)
        failed = copy.deepcopy(self.results); failed["responses"][0]["response"]["status"] = "failed"; variants.append(failed)
        for value in variants:
            with self.assertRaises(ValueError):
                score(self.cases, self.prepared, value)

    def test_new_card_does_not_invent_version_or_exceed_summary_budget(self):
        call = self.results["responses"][0]["response"]["output"][0]
        args = json.loads(call["arguments"])
        args.update(expectedVersion=1, summary="字" * 181)
        call["arguments"] = json.dumps(args)
        result = score(self.cases, self.prepared, self.results)
        self.assertEqual(len(result["failures"][0]["errors"]), 2)


class ConversationEvaluatorSetupTests(unittest.TestCase):
    def test_routing_replay_exposes_project_registration(self):
        _, descriptions, _ = surface(Path(__file__).resolve().parents[2] / "crates/pcp-mcp/src/lib.rs")
        self.assertIn("pcp_ensure_project_scope", descriptions)


    def test_missing_local_data_has_actionable_cli_error(self):
        with tempfile.TemporaryDirectory(prefix="pcp-eval-no-data-") as directory:
            root = Path(directory)
            output = root / "probes.json"
            result = subprocess.run([
                sys.executable, str(Path(__file__).with_name("conversation_eval.py")),
                "prepare", "--fixtures", str(root / "missing-conversations.json"),
                "--catalog", str(root / "catalog.json"),
                "--agents", str(root / "AGENTS.md"), "--output", str(output),
            ], text=True, capture_output=True)
            self.assertEqual(result.returncode, 2)
            self.assertIn("Datasets are not bundled", result.stderr)
            self.assertIn("--fixtures", result.stderr)
            self.assertNotIn("Traceback", result.stderr)
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
