"""Source skeleton. Implement authorized platform search before publishing."""
from pathlib import Path
from rmusic_plugin import Plugin

plugin = Plugin(Path(__file__).with_name("manifest.json"))


@plugin.method("search")
def search(params):
    # Track IDs must be '<plugin-id>:<stable-id>'; source must equal plugin-id.
    # Names alone never prove a recording, release or performer identity.
    return {"items": [], "total": 0}


if __name__ == "__main__":
    plugin.serve()
