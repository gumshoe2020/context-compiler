All people, handles, facts, dates and canaries here are invented. The telephone numbers use
the reserved fictional 202-555-01xx range and the email uses example.invalid.
memory.py and model.py are local file stand-ins; neither accesses a network or a model.
The acceptance runner builds every database, transcript, log and installation under
target/priv1/. Deliberate search is tested separately from automatic admission.

PRIV-1c adds recall.py/recall_mcp.py (constructed drawer MCP responses with 910 shared
markers and a fixed deadline) and sticky_tags.py (backfill and actual settle with file-only
adapters). Their scratch paths are under target/priv1c/. The MCP crate also tests tagged
diary writes through its actual stdio service.

KG-EV1 adds kg_evidence.py: actual settle with file-only adapters checks full-entry
evidence, aliases, repointing, search fallback, skipped relations and report counts.
Its scratch path is confined to target/kgev1/.
