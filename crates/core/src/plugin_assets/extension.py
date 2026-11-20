"""Read-only extension template. Run `plugin_dev.py check` before packaging."""
from pathlib import Path
from rmusic_plugin import Plugin

plugin = Plugin(Path(__file__).with_name("manifest.json"))


@plugin.method("extension_invoke")
def invoke(params):
    if params.get("version") != 1 or params.get("method") != "summarize":
        raise ValueError("unsupported_extension")
    context = params["context"]
    return {"version": 1, "output": {"tracks": context["track_count"],
                                      "sources": len(context["source_counts"])}}


if __name__ == "__main__":
    plugin.serve()
