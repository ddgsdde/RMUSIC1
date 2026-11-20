"""No Docker execution: regress honest core+plugin idle measurement reporting."""
import unittest
from unittest.mock import patch
import json
import subprocess

import benchmark


class IdlePluginTest(unittest.TestCase):
    def test_proc_probe_is_valid_python_argument(self):
        def docker(*args):
            if args[0] == "stats":
                return subprocess.CompletedProcess(args, 0, stdout="{}")
            program=args[-1]
            self.assertNotIn("\0", program)
            compile(program,"proc-probe","exec")
            return subprocess.CompletedProcess(args,0,stdout=json.dumps({"processes":[],"rss_sum_bytes":0}))
        with patch.object(benchmark,"docker",docker):
            self.assertEqual(benchmark.resource_sample("isolated")["rss_sum_bytes"],0)

    def sample(self):
        return {"processes": [
            {"pid": 1, "name": "rmusic-core", "command": ["rmusic-core"], "rss_bytes": 100},
            {"pid": 17, "name": "python3", "command": ["python3", "-u", "/data/plugins/qqmusic/provider.py"], "rss_bytes": 50},
            {"pid": 18, "name": "python3", "command": ["python3", "-c", "healthcheck"], "rss_bytes": 20},
        ], "rss_sum_bytes": 170}

    def test_health_starts_before_sample_and_identifies_actual_plugin(self):
        events = []
        def health(path, token):
            events.append("health")
            self.assertEqual(path, "/api/v1/plugins/qqmusic/health")
            return {"status": "ready", "authenticated": False}, 4.25
        def sample(identity):
            events.append("sample")
            return self.sample()
        with patch.object(benchmark, "request", health), patch.object(benchmark, "resource_sample", sample):
            provider, resources = benchmark.idle_with_qqmusic("isolated", "test-only")
        self.assertEqual(events, ["health", "sample"])
        self.assertTrue(provider["included_in_idle_rss"])
        self.assertEqual(provider["process_ids"], [17])
        self.assertEqual(resources["core_and_qq_rss_bytes"], 150)
        self.assertEqual(resources["rss_sum_bytes"], 170)

    def test_health_failure_does_not_claim_plugin_inclusion(self):
        with patch.object(benchmark, "request", side_effect=OSError("explicit health failure")), patch.object(benchmark, "resource_sample", return_value=self.sample()):
            provider, resources = benchmark.idle_with_qqmusic("isolated", "test-only")
        self.assertEqual(provider["status"], "failed")
        self.assertIn("health failure", provider["error"])
        self.assertFalse(provider["included_in_idle_rss"])
        self.assertIsNone(resources["core_and_qq_rss_bytes"])

    def test_ready_response_without_actual_qq_process_is_not_included(self):
        sample = self.sample()
        sample["processes"] = [sample["processes"][0], sample["processes"][2]]
        with patch.object(benchmark, "request", return_value=({"status": "ready"}, 1.)), patch.object(benchmark, "resource_sample", return_value=sample):
            provider, resources = benchmark.idle_with_qqmusic("isolated", "test-only")
        self.assertFalse(provider["included_in_idle_rss"])
        self.assertEqual(provider["process_ids"], [])
        self.assertIsNone(resources["core_and_qq_rss_bytes"])


if __name__ == "__main__":
    unittest.main()
