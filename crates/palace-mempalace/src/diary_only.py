# Run by palace-mempalace::diary_only on stdin:
#   python -I - <palace-copy> <marker> <pinned-tool-root>
# Deletes rows whose source_file contains the marker from the two MemPalace collections of a COPY,
# after proving this interpreter and its chromadb belong to the pinned uv tool tree.
# Exit 10 = not the pinned interpreter/chromadb, 11 = the copy is broken or unreadable.
import json
import os
import sys

palace, marker, tool = sys.argv[1], sys.argv[2], os.path.realpath(sys.argv[3])


def fail(code, message):
    print(message, file=sys.stderr)
    sys.exit(code)


def inside(path, root):
    return os.path.realpath(path).startswith(root + os.sep)


if not sys.flags.isolated or os.path.realpath(sys.prefix) != tool:
    fail(10, f"interpreter check: prefix {sys.prefix} (isolated={sys.flags.isolated}) is not {tool}")
if os.path.realpath(os.path.dirname(sys.executable)) != os.path.join(tool, "bin"):
    fail(10, f"interpreter check: {sys.executable} is not under {tool}/bin")
try:
    import chromadb
except Exception as error:  # noqa: BLE001 - any import failure means "not the pinned tool"
    fail(10, f"interpreter check: chromadb not importable: {error}")
if not inside(chromadb.__file__, tool):
    fail(10, f"interpreter check: chromadb {chromadb.__file__} is outside {tool}")

try:
    client = chromadb.PersistentClient(path=palace)
    collections = {
        name: client.get_collection(name) for name in ("mempalace_drawers", "mempalace_closets")
    }
except Exception as error:  # noqa: BLE001
    fail(11, f"broken palace copy {palace}: {error}")

report = {}
for name, collection in collections.items():
    rows = collection.get(include=["metadatas"])
    ids = [
        row_id
        for row_id, meta in zip(rows["ids"], rows["metadatas"])
        if marker in str((meta or {}).get("source_file") or "")
    ]
    if ids:
        collection.delete(ids=ids)
    report[name] = {"removed": len(ids), "remaining": collection.count()}
print(json.dumps(report))
