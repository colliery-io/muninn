---
id: 001-muninn-system-architecture-and
level: adr
title: "Muninn System Architecture and Repo-Local Storage Design"
number: 1
short_code: "PROJEC-A-0001"
created_at: 2026-01-07T17:34:51.457351+00:00
updated_at: 2026-01-10T17:13:13.629701+00:00
decision_date: 
decision_maker: 
parent: 
archived: false

tags:
  - "#adr"
  - "#phase/decided"


exit_criteria_met: false
strategy_id: NULL
initiative_id: NULL
---

# ADR-1: Muninn System Architecture and Repo-Local Storage Design

## Context

Muninn needs to provide:
1. **Recursive context selection** - RLM-style intelligent exploration of codebases
2. **Persistent memory** - Learnings that survive across sessions and accumulate over time
3. **Team collaboration** - Memory that can be shared, branched, and merged like code
4. **Privacy-first** - All processing local, no cloud dependencies

The critical design challenge is making memory **version-control native** - living inside the git repo as a first-class artifact that participates in branching, merging, and rebasing workflows naturally.

## Decision

### System Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────┐
│  Claude Code (or any OpenAI-compatible client)                          │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼ OpenAI-compatible API
┌─────────────────────────────────────────────────────────────────────────┐
│  Muninn Gateway                                                         │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────────────┐  │
│  │ Request Router  │→ │ Context Engine  │→ │ Memory Manager          │  │
│  │                 │  │ (RLM-style)     │  │                         │  │
│  │ - Session mgmt  │  │ - Recursive     │  │ - Read/write layers     │  │
│  │ - Auth/routing  │  │   exploration   │  │ - Query across layers   │  │
│  │                 │  │ - Tool dispatch │  │ - Reconciliation        │  │
│  └─────────────────┘  └─────────────────┘  └─────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────┘
          │                       │                       │
          ▼                       ▼                       ▼
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────────────┐
│ LLM Backend     │    │ Repo Tools      │    │ .muninn/                │
│ (vLLM, ollama,  │    │ - grep, read    │    │ (repo-local storage)    │
│  cloud, etc.)   │    │ - tree, list    │    │                         │
└─────────────────┘    │ - Metis query   │    │ See: Memory Layers      │
                       └─────────────────┘    └─────────────────────────┘
```

### Memory Layers

Memory is organized into three distinct layers with different characteristics:

#### Layer 1: Derived Cache (Ephemeral)

**What**: Precomputed indexes, embeddings, structural analysis
**Storage**: `.muninn/cache/` (gitignored or LFS)
**Characteristics**:
- 100% rebuildable from source code
- Can be pruned aggressively for space
- Not version controlled (or LFS for sharing)
- Rebuilt on-demand or via `muninn index`

**Contents**:
- Vector embeddings of code chunks (sqlite-vec)
- Code structure graphs (graphqlite)  
- File dependency maps
- Symbol tables

#### Layer 2: Session Memory (Main + Sessions)

**What**: Learnings from AI sessions - what files were relevant, what patterns were discovered

**Storage Structure**:
```
.muninn/memory/
├── main.jsonl              # Consolidated canonical memory (git-tracked)
└── sessions/               # Transient session files (git-tracked)
    ├── session_abc123.jsonl   # From feature-x work
    ├── session_def456.jsonl   # From feature-y work
    └── ...                    # Cleared after reconciliation
```

**Two-tier model**:
- **main.jsonl**: Consolidated, reconciled memory. The canonical "what we know" for the project.
- **sessions/**: Transient session files created during branch work. Named by session ID to avoid git merge conflicts.

**Characteristics**:
- Session files are append-only during work
- Git merge just moves session files to main (no conflicts - unique filenames)
- Reconciliation is a separate manual chore (not tied to git merge)
- Session files deleted after reconciliation

**Contents**:
- Context selections: "For query X, files Y were relevant"
- Discovered patterns: "Module A depends on B for auth"
- Session summaries: "Implemented feature X, touched files Y"

**Query behavior**:
- On main: query `main.jsonl` only
- On branch: query `main.jsonl` + relevant session files (union)
- Duplicates are acceptable (better than gaps)

**Format** (JSONL):
```jsonl
{"ts": "2026-01-07T10:00:00Z", "type": "context_hit", "query_hash": "abc123", "files": ["src/auth.py"], "relevance": 0.95}
{"ts": "2026-01-07T10:05:00Z", "type": "pattern", "pattern": "auth_flow", "entities": ["src/auth.py", "src/middleware.py"], "confidence": 0.8}
{"ts": "2026-01-07T11:00:00Z", "type": "session_summary", "task": "MUNI-T-0042", "files_touched": ["src/api.py"], "learnings": ["API uses FastAPI dependency injection"]}
```

#### Layer 3: Curated Knowledge (Canonical)

**What**: Team-authored facts, conventions, decisions that aren't derivable from code
**Storage**: `.muninn/knowledge/` (git-tracked, text format)
**Characteristics**:
- Human-authored or human-approved
- Portable across projects (conventions, patterns)
- Integrates with Metis ADRs
- Standard markdown + frontmatter

**Contents**:
- Team conventions: "We use repository pattern for data access"
- Architectural decisions: Links to Metis ADRs
- Project glossary: Domain terms and definitions
- Cross-project patterns: Importable knowledge packs

**Format**:
```markdown
---
type: convention
domain: error-handling
confidence: high
source: team-decision
---
# Error Handling Convention

All API endpoints use structured error responses with:
- `error_code`: Machine-readable code
- `message`: Human-readable message  
- `details`: Optional additional context

Exceptions are caught at the middleware layer, never in individual handlers.
```

### Branch and Merge Model

Git branching and memory are decoupled. Git handles file movement; reconciliation is a separate chore.

```
main:     [main.jsonl] ───●───●───●───●───●─── [main.jsonl updated]
                         \           ↑        ↑
                          \     git merge    muninn reconcile
feature-x:                 ●───●───●──┘      (manual chore)
                      [session_xyz.jsonl created]
```

**Branch Creation**: `git checkout -b feature-x`
- `main.jsonl` comes with the branch (it's in git)
- New session file created: `sessions/session_<id>.jsonl`
- Derived cache may be shared or rebuilt locally

**During Development**:
- Session memories append to `sessions/session_<id>.jsonl`
- Queries read `main.jsonl` + session files (union)
- Curated knowledge edits are just file changes

**Git Merge**: `git merge feature-x`
- Git moves session files to main branch
- No conflicts (session files have unique names)
- `main.jsonl` unchanged by merge
- Session files accumulate in `sessions/` on main

**Key insight**: Git merge is just file movement. No special handling needed.

### Reconciliation Process

Reconciliation is a **manual chore**, decoupled from git merge. Run it on main when convenient (weekly, before release, when sessions accumulate).

```
┌─────────────────────────────────────────────────────────────────┐
│  muninn reconcile                                                │
│                                                                  │
│  Input:                                                          │
│    - main.jsonl (current canonical memory)                       │
│    - sessions/*.jsonl (accumulated session files)                │
│                                                                  │
│  Process:                                                        │
│    1. Read all session files                                     │
│    2. LLM-assisted deduplication against main.jsonl:             │
│       - "Same insight, different words" → dedupe                 │
│       - "New insight" → add to main                              │
│       - "Contradictory insight" → flag for human review          │
│    3. Confidence boosting (multiple sessions found same → higher)│
│                                                                  │
│  Output:                                                         │
│    - Updated main.jsonl                                          │
│    - Conflict report (if any) for human review                   │
│    - Session files deleted (or archived with --archive flag)     │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

**LLM-Assisted Consolidation** handles:
- Semantic deduplication ("same insight, different words")
- Confidence scoring (multiple sessions found same pattern → higher confidence)
- Temporal ordering (newer observations may supersede older)

**Human Review Queue** for:
- Contradictory facts ("module X uses pattern A" vs "module X uses pattern B")
- Anything the LLM flags as uncertain

**CLI Flow**:
```bash
# On main branch, after merges have accumulated session files
git checkout main
git pull

muninn reconcile
# Reads sessions/*.jsonl, consolidates into main.jsonl
# Outputs: "5 memories consolidated, 2 duplicates removed, 1 conflict"

muninn conflicts
# Shows conflicts requiring human decision

muninn resolve <conflict-id> --keep=left|right|both
# Resolves specific conflict

# Session files are deleted by default
# Use --archive to move to sessions/archive/ instead

git add .muninn/ && git commit -m "Reconcile memories"
git push
```

**Operational cadence**: Reconciliation is a team chore, like dependency updates or changelog maintenance. Run it when:
- Session files accumulate (5+)
- Before a release
- Weekly/bi-weekly as habit

### Storage Format Decisions

| Layer | Format | Git Strategy | Rationale |
|-------|--------|--------------|-----------|
| Derived Cache | SQLite (sqlite-vec, graphqlite) | `.gitignore` or LFS | Binary, large, rebuildable |
| Session Memory | JSONL (one event per line) | Normal git | Diffable, append-friendly, mergeable |
| Curated Knowledge | Markdown + YAML frontmatter | Normal git | Human-readable, editable, portable |

**Why JSONL for session memory**:
- Append-only = minimal merge conflicts
- Line-based = git can merge non-overlapping additions
- Human-readable = debuggable
- Streamable = can process large logs incrementally

**Why Markdown for curated knowledge**:
- Already used by Metis, familiar
- Portable across projects
- Human-authorable without tooling
- Frontmatter enables structured queries

### Metis Integration

Muninn queries Metis documents as part of context:

```
.metis/
├── vision.md          ← Muninn reads for project context
├── initiatives/       ← Muninn knows current work focus  
├── tasks/            ← Muninn links sessions to tasks
└── adrs/             ← Muninn treats as curated knowledge
```

**Session → Task linking**:
- Sessions can be tagged with Metis task IDs
- Memory entries reference which task they relate to
- Enables: "What did we learn while working on MUNI-T-0042?"

**ADR → Knowledge linking**:
- ADRs are treated as high-confidence curated knowledge
- Muninn can surface relevant ADRs during context selection
- "This query touches auth - here's ADR-003 on our auth architecture"

## Alternatives Considered

### Alternative: External Database (rejected)

Store memory in external SQLite/Postgres outside the repo.

| Aspect | Pros | Cons |
|--------|------|------|
| Simplicity | Standard DB patterns | Not portable with repo |
| Performance | Optimized queries | Requires separate backup/sync |
| Collaboration | N/A | Manual sharing, no branch semantics |

**Rejected because**: Violates "memory is a repo artifact" principle. Doesn't get git branching for free.

### Alternative: Pure Embedding RAG (rejected)

Use only vector embeddings, no session memory.

| Aspect | Pros | Cons |
|--------|------|------|
| Simplicity | Well-understood pattern | No learning over time |
| Storage | Compact | No team knowledge capture |
| Accuracy | Good for similarity | Misses discovered patterns |

**Rejected because**: Doesn't accumulate knowledge. Every session starts from embeddings alone.

### Alternative: Git LFS for Everything (deferred)

Store all memory layers in Git LFS.

| Aspect | Pros | Cons |
|--------|------|------|
| Size | Handles large files | Requires LFS setup |
| Compatibility | Standard git workflow | Binary = no diff/merge |
| Performance | Lazy loading | Network dependency |

**Deferred**: May use LFS for derived cache layer. Session memory stays text-based for mergeability.

## Consequences

### Positive
- Memory participates naturally in git workflows (branch, merge, rebase)
- Teams share and accumulate knowledge through normal git operations
- Clear separation of rebuildable vs curated data
- Human-readable formats enable debugging and manual editing
- Portable knowledge can transfer across projects
- **Git merge is trivial** - no conflicts due to unique session filenames
- **Reconciliation is decoupled** - doesn't block merges, run when convenient

### Negative
- Reconciliation is a manual chore (requires discipline)
- main.jsonl can grow large over time (need pruning strategy)
- LLM-assisted consolidation has cost (tokens) and latency
- New concepts for users to learn (memory layers, reconciliation)
- Memory may be temporarily stale between reconciliations (acceptable)

### Neutral
- Derived cache can be gitignored (no sharing) or LFS (shared but large)
- Reconciliation quality depends on LLM capability
- Session files accumulate until reconciliation (by design)

## Open Questions

1. **Pruning strategy**: How aggressively to prune old entries from main.jsonl? Time-based? Confidence-based? Size-based?

2. **Cross-repo knowledge**: How to import/export curated knowledge between projects? Package format?

3. **Conflict UX**: What's the best interface for human conflict resolution? CLI? TUI? Web?

4. **Cache sharing**: Is it worth sharing derived cache via LFS, or always rebuild locally?

5. **Memory queries**: Query language/API for "find memories related to X"? SQL? Natural language?

6. **Session identity**: How does a session get its ID? UUID? Branch name + timestamp? User configurable?

## Review Triggers

- After implementing basic reconciliation: evaluate consolidation quality
- After 3 months usage: evaluate main.jsonl growth and pruning needs
- After multi-user testing: evaluate reconciliation cadence and conflict frequency