"""R-Music Python RPC SDK v1; standard library only, no network or host actions."""
import json
import pathlib
import sys

MAX_REQUEST = 128 * 1024
MAX_REPLY = 128 * 1024


class Plugin:
    def __init__(self, manifest_path):
        self.manifest = json.loads(pathlib.Path(manifest_path).read_text(encoding="utf-8"))
        self.methods = {"manifest": lambda _: self.manifest,
                        "health": lambda _: {"status": "ready"},
                        "contract": lambda _: {"version": 1, "methods": sorted(self.methods)}}

    def method(self, name):
        def register(function):
            self.methods[name] = function
            return function
        return register

    def serve(self):
        while True:
            line = sys.stdin.buffer.readline(MAX_REQUEST + 1)
            if not line:
                return
            if len(line) > MAX_REQUEST or not line.endswith(b"\n"):
                return  # Keep an oversized request from being split into valid requests.
            request = None
            try:
                request = json.loads(line)
                if not isinstance(request, dict) or not isinstance(request.get("method"), str):
                    raise ValueError("request")
                handler = self.methods.get(request["method"])
                if handler is None:
                    raise ValueError("method")
                result = handler(request.get("params", {}))
                reply = {"id": request.get("id"), "result": result}
                encoded = json.dumps(reply, ensure_ascii=False).encode("utf-8")
                if len(encoded) > MAX_REPLY - 1:
                    raise ValueError("response_limit")
            except Exception:
                # Exceptions can contain credentials. Keep diagnostics out of the RPC payload.
                encoded = json.dumps({"id": request.get("id") if isinstance(request, dict) else None,
                                      "error": {"code": "plugin_request_failed",
                                                "message": "插件请求不符合合同或执行失败"}},
                                     ensure_ascii=False).encode("utf-8")
            sys.stdout.buffer.write(encoded + b"\n")
            sys.stdout.buffer.flush()


def configuration(data_dir):
    """Read your own persisted configuration; never log or return secret fields."""
    path = pathlib.Path(data_dir) / "configuration.json"
    if not path.exists():
        return {}
    if path.stat().st_size > 64 * 1024:
        raise ValueError("configuration_limit")
    return json.loads(path.read_text(encoding="utf-8"))
