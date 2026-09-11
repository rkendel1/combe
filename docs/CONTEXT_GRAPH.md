# Combe Context Graph v1

`COMBE_CONTEXT_GRAPH` version 1 is a derived relationship and navigation projection. FeltDB records remain authoritative for Work, conversations, decisions, artifacts, and provenance. Git, worktrees, and the filesystem remain authoritative for repository facts and file contents.

The projection contains deterministic, metadata-only nodes for Work, conversations, messages, decisions, artifacts, files, commits, provider exchanges, worktrees, and context snapshots. Semantic edges retain a stable source identity, confidence class, and optional reason. Message bodies, file contents, provider payloads, and credentials are never graph properties.

Projection fingerprints are SHA-256 hashes of canonically ordered nodes, edges, and the derived source revision. `generated_at` is operational metadata and is excluded. Reapplying an observation is idempotent; rebuilding from equivalent canonical sources produces the same fingerprint.

Repository integrations submit `ChangeObservation` values. A change provider supplies repository, worktree, commit, parent, branch, file identity, path-at-revision, rename, and confidence facts. Combe does not independently calculate Git history. Stable file identities preserve rename continuity; path-only artifacts are explicitly marked as path-confidence fallbacks.

Traversal is breadth-first, deterministic, and bounded by depth, node count, edge count, and serialized bytes. Optional node filters and Work scope are enforced before relationships enter the result. The AI continuity resolver uses graph traversal only to select canonical references, then fingerprints those ordered references as part of `COMBE_AI_CONTEXT` version 1.

Run `combe graph rebuild` once to initialize the projection. `combe graph status`, `combe graph verify`, `combe graph inspect <node>`, and `combe graph related <node>` inspect it; every command supports `--json`. Verification is read-only and reports missing nodes, missing edges, orphaned edges, duplicate relationships, stale sources, and fingerprint divergence.

Provider delivery consumes the graph only through the bounded projection described in [CONTEXT_ASSEMBLY.md](CONTEXT_ASSEMBLY.md). The graph remains the relevance source; Work remains authoritative.
