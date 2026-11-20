import json
import pathlib
import tempfile
import time
import unittest
import zipfile

import plugin_dev as tool


class PluginDevelopmentTests(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.TemporaryDirectory(prefix="rmusic-plugin-sdk-")
        self.root = pathlib.Path(self.work.name)
        self.addCleanup(self.work.cleanup)

    def scaffold(self, kind="extension"):
        directory = self.root / kind
        tool.init("my-" + kind, kind, directory)
        return directory

    def test_both_templates_pass_rpc_and_pack_without_secrets(self):
        for kind in ("source", "extension"):
            with self.subTest(kind=kind):
                directory = self.scaffold(kind)
                (directory / ".env").write_text("SECRET=must-not-pack")
                (directory / "configuration.json").write_text('{"key":"must-not-pack"}')
                destination = self.root / (kind + ".zip")
                tool.pack(directory, destination)
                with zipfile.ZipFile(destination) as archive:
                    self.assertEqual(set(archive.namelist()), {"manifest.json", "provider.py", "rmusic_plugin.py", "README.md"})
                    self.assertFalse(any(b"must-not-pack" in archive.read(name) for name in archive.namelist()))
                with self.assertRaises(FileExistsError):
                    tool.pack(directory, destination)

    def test_legacy_source_without_optional_capabilities_still_passes(self):
        directory = self.scaffold("source")
        manifest = json.loads((directory / "manifest.json").read_text())
        manifest.pop("capabilities")
        (directory / "manifest.json").write_text(json.dumps(manifest))
        self.assertEqual(tool.check(directory)["id"], "my-source")

    def test_source_probe_rejects_unimplemented_declared_methods(self):
        directory = self.scaffold("source")
        manifest = json.loads((directory / "manifest.json").read_text())
        manifest["capabilities"].append("resolve")
        (directory / "manifest.json").write_text(json.dumps(manifest))
        with self.assertRaisesRegex(ValueError, "implemented method"):
            tool.check(directory)

    def test_source_scenarios_and_ownership_are_checked(self):
        directory = self.scaffold("source")
        cases = self.root / "cases.json"
        cases.write_text(json.dumps([{"method": "search", "params": {"query": "test"}, "expected": {"total": 0}}]))
        self.assertEqual(tool.exercise(directory, cases)["id"], "my-source")
        provider = directory / "provider.py"
        provider.write_text(provider.read_text().replace('return {"items": [], "total": 0}', 'return {"items": [{"id":"foreign:x","source":"foreign"}], "total": 0}'))
        with self.assertRaisesRegex(ValueError, "foreign track"):
            tool.exercise(directory, cases)

    def test_scenarios_check_classified_errors_without_leaking_exception(self):
        directory = self.scaffold("source")
        cases = self.root / "errors.json"
        cases.write_text(json.dumps([{"method":"unimplemented", "expected_error":"plugin_request_failed"}]))
        self.assertEqual(tool.exercise(directory, cases)["id"], "my-source")
        cases.write_text(json.dumps([{"method":"unimplemented", "expected_error":"rate_limited"}]))
        with self.assertRaisesRegex(ValueError,"classification"):
            tool.exercise(directory, cases)

    def test_extension_executes_real_context_without_host_actions(self):
        directory = self.scaffold()
        output = tool.rpc(directory, [("extension_invoke", {"version": 1, "method": "summarize", "input": {}, "context": {"track_count": 123, "source_counts": [{"source": "local", "count": 123}]}})])
        self.assertEqual(output, [{"version": 1, "output": {"tracks": 123, "sources": 1}}])

    def test_sdk_never_echoes_exception_secrets(self):
        directory = self.scaffold()
        provider = directory / "provider.py"
        provider.write_text('from rmusic_plugin import Plugin\np=Plugin("manifest.json")\n@p.method("explode")\ndef explode(params): raise ValueError("secret-must-not-leak")\np.serve()\n')
        with self.assertRaisesRegex(ValueError, "RPC correlation or result failed"):
            tool.rpc(directory, [("explode", {})])

    def test_no_overwrite_and_invalid_ids(self):
        directory = self.scaffold()
        with self.assertRaises(FileExistsError):
            tool.init("again", "extension", directory)
        for identifier in ("../escape", "Uppercase", "a/b", "a" * 65):
            with self.assertRaises(ValueError):
                tool.init(identifier, "source", self.root / "invalid")

    def test_closed_bounded_schema_and_scope_validation(self):
        manifest = json.loads((tool.ASSETS / "extension.json").read_text())
        manifest["extensions"][0]["input"] = {"type": "object", "properties": {"q": {"type": "string"}}, "additionalProperties": False}
        with self.assertRaisesRegex(ValueError, "maxLength"):
            tool.validate(manifest)
        manifest["extensions"][0]["input"] = {"type": "object", "properties": {}, "additionalProperties": False}
        manifest["extensions"][0]["effect"] = "proposal"
        with self.assertRaisesRegex(ValueError, "cannot propose"):
            tool.validate(manifest)

    def test_oversized_output_and_timeout_kill_the_process(self):
        directory = self.scaffold()
        (directory / "provider.py").write_text('print("x" * 200000, flush=True)\n')
        with self.assertRaisesRegex(ValueError, "128 KiB"):
            tool.rpc(directory, [("manifest", {})], timeout=2)
        (directory / "provider.py").write_text('import time\ntime.sleep(60)\n')
        started = time.monotonic()
        with self.assertRaisesRegex(ValueError, "timed out"):
            tool.rpc(directory, [("manifest", {})], timeout=.15)
        self.assertLess(time.monotonic() - started, 2)

    def test_scaffold_contains_standalone_checker(self):
        import subprocess
        directory = self.scaffold()
        result = subprocess.run([tool.sys.executable, "plugin_dev.py", "check", "."], cwd=directory, capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        self.assertTrue(json.loads(result.stdout)["passed"])

    def test_package_rejects_linked_code(self):
        directory = self.scaffold()
        (directory / "external.py").symlink_to(directory / "provider.py")
        with self.assertRaisesRegex(ValueError, "symlinks"):
            tool.pack(directory, self.root / "invalid.zip")


if __name__ == "__main__":
    unittest.main()
