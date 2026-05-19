# Sessio Memory Backend Abstraction Plan

## Goal

Sessio memory should support arbitrary memory implementations later, not only qmd.

The target shape is:

- Sessio remains the source of truth for session metadata, source refs, card provenance, and continuation lineage.
- qmd becomes one replaceable search/index backend over generated memory records.
- The memory build pipeline does not know whether cards are eventually searched by qmd, SQLite FTS, a vector DB, a remote service, or an in-process test backend.
- CLI and skill APIs stay stable while backend implementations change underneath.

This is a design plan only. Do not implement it until the current qmd-backed v1 behavior is stable enough to refactor safely.

## Current Coupling

The current implementation has several useful pieces, but their boundaries are too qmd-shaped:

- `src-tauri/src/memory/mod.rs` mixes domain models, repository interfaces, and job concepts.
- `src-tauri/src/memory/build.rs` generates memory cards, persists metadata, writes markdown files, and returns paths needed by backend sync.
- `src-tauri/src/memory/qmd.rs` is a concrete command wrapper, but callers import it directly.
- `src-tauri/src/indexer/mod.rs` queues qmd-specific jobs and records qmd-specific job kinds.
- CLI commands expose `qmd` as a first-class namespace, which is still useful for diagnostics but should not be the only memory backend path.

The main issue is not that qmd exists. The issue is that the build pipeline's output contract is "markdown files under a qmd cards root" instead of "memory records plus source/provenance metadata".

## Desired Architecture

Split memory into four logical layers:

```text
providers
  parse agent-specific sources into SessionSource + MessageEvent

memory-core
  pure data model and algorithms
  normalize, hash, card generation, continuation dedupe

memory-service
  orchestration
  build project/source memory, persist metadata, emit index changes

memory-backends
  replaceable retrieval/index implementations
  qmd, sqlite-fts, vector, remote, noop/test
```

The dependency direction should be:

```text
CLI/UI/indexer
  -> memory-service
      -> memory-core
      -> MemoryRepository
      -> MemoryIndexBackend
          -> qmd/sqlite/vector/remote implementations
```

No backend should depend on provider internals. No provider should know about memory backends.

## Terminology

Use these names consistently:

- `MemoryRecord`: the backend-indexable unit. Replaces `MemoryCard` outright — no transitional alias.
- `MemoryRepository`: Sessio-owned structured persistence for records, source refs, fingerprints, continuations, and jobs.
- `MemoryIndexBackend`: replaceable retrieval/index backend.
- `MemoryArtifactSink`: optional file/object writer used by backends that need external artifacts, such as qmd markdown files.
- `MemoryService`: orchestrates builds and backend sync.

The abstraction must not name any backend-neutral field `qmd_path`.

## Core Data Model Changes

Replace `MemoryCard` with `MemoryRecord`. The `qmd_path` column on `memory_cards` is dropped — artifact location is no longer part of record identity.

Current shape:

```rust
pub struct MemoryCard {
    pub card_id: String,
    pub project_key: String,
    pub canonical_hash: String,
    pub simhash: Option<String>,
    pub qmd_path: String,
    pub title: String,
    pub summary: Option<String>,
    pub body: String,
    pub available: bool,
    pub updated_at: i64,
}
```

Target shape:

```rust
pub struct MemoryRecord {
    pub record_id: String,
    pub project_key: String,
    pub canonical_hash: String,
    pub simhash: Option<String>,
    pub title: String,
    pub summary: Option<String>,
    pub body: String,
    pub kind: MemoryRecordKind,
    pub available: bool,
    pub updated_at: i64,
}

pub struct MemoryArtifact {
    pub record_id: String,
    pub backend: String,
    pub artifact_uri: String,
    pub content_hash: String,
    pub updated_at: i64,
}
```

Artifact metadata lives in SQLite from the first extraction. Backends do not stat the filesystem to discover what they wrote — they look it up:

```sql
CREATE TABLE IF NOT EXISTS memory_artifacts (
    record_id    TEXT NOT NULL,
    backend      TEXT NOT NULL,
    artifact_uri TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    updated_at   INTEGER NOT NULL,
    PRIMARY KEY(record_id, backend)
);
```

Rationale: a future remote/object-store backend cannot derive `artifact_uri` from `(project_key, record_id)` deterministically, and even for `QmdBackend` "did we already write this file?" should be a DB lookup, not a `stat`. Pay the table cost once now.

## Artifact Path Layout

The on-disk layout for any file-emitting backend is:

```text
~/.sessio/memory/<backend>/projects/<project_key>/sessions/<record_id>.md
```

For example:

```text
~/.sessio/memory/qmd/projects/-Users-alex-Work-cloudgeek-sessio/sessions/sessio-codex-019e2730-....md
```

The old layout `~/.sessio/qmd-memory/projects/<project_key>/cards/*.md` is removed entirely. There is no compatibility shim — existing artifacts are wiped and rebuilt from SQLite (which is the source of truth). The CLI flag previously named `--cards-root` becomes `--artifacts-root`, and the default is derived from backend config.

Including `<backend>` in the path lets multiple backends coexist without overwriting each other's artifacts. `sessions/` reflects that one record corresponds to one session's memory, not an opaque "card".

## Repository Boundary

The repository is Sessio's source of truth. It does not search external indexes. It stores structured memory state and provenance.

Target trait:

```rust
pub trait MemoryRepository: Send + Sync {
    fn upsert_record(&self, record: &MemoryRecord) -> Result<()>;
    fn replace_record_sources(&self, record_id: &str, sources: &[MemorySource]) -> Result<()>;
    fn replace_record_continuation(
        &self,
        record_id: &str,
        continuation: Option<&CardContinuation>,
    ) -> Result<()>;

    fn list_records_for_source(
        &self,
        source: &SessionSource,
    ) -> Result<Vec<MemoryRecord>>;
    fn list_project_records(&self, project_key: &str) -> Result<Vec<MemoryRecord>>;
    fn record_by_id(&self, record_id: &str) -> Result<Option<MemoryRecord>>;
    fn sources_for_record(&self, record_id: &str) -> Result<Vec<MemorySource>>;

    fn mark_record_unavailable(&self, record_id: &str) -> Result<()>;
    fn mark_source_records_unavailable(&self, source: &SessionSource) -> Result<()>;

    fn replace_turn_fingerprints(
        &self,
        project_key: &str,
        source: &SessionSource,
        fingerprints: &[TurnFingerprint],
    ) -> Result<()>;
    fn find_turn_fingerprint_candidates(
        &self,
        project_key: &str,
        exclude_source: &SessionSource,
        canonical_hashes: &[&str],
        limit: usize,
    ) -> Result<Vec<TurnFingerprintCandidate>>;

    fn record_memory_job(&self, job: &MemoryJobUpdate) -> Result<()>;
    fn list_memory_jobs(&self, filter: MemoryJobFilter) -> Result<Vec<MemoryJob>>;
}
```

The current `MemoryStore` can be adapted gradually. Avoid a large rename-only diff until the backend interface exists.

## Backend Boundary

Backends receive project-level record changes and expose search. They do not own canonical memory facts.

Target trait:

```rust
pub trait MemoryIndexBackend: Send + Sync {
    fn name(&self) -> &'static str;
    fn status(&self) -> MemoryBackendStatus;

    fn sync_project(
        &self,
        project_key: &str,
        records: &[MemoryRecord],
        artifacts: &dyn MemoryArtifactSink,
    ) -> Result<MemorySyncReport>;

    fn remove_project(&self, project_key: &str) -> Result<MemorySyncReport>;

    fn search(
        &self,
        project_key: &str,
        query: &str,
        options: MemorySearchOptions,
    ) -> Result<MemoryBackendSearchResult>;
}
```

Search result should return backend-neutral IDs whenever possible:

```rust
pub struct MemoryBackendSearchResult {
    pub backend: String,
    pub hits: Vec<MemoryBackendHit>,
    pub raw: Option<serde_json::Value>,
}

pub struct MemoryBackendHit {
    pub record_id: Option<String>,
    pub artifact_uri: Option<String>,
    pub score: Option<f64>,
    pub snippet: Option<String>,
}
```

The service maps backend hits back to repository records. If a backend returns only an artifact URI, the repository or artifact sink resolves it to a record ID.

## Artifact Sink

Some backends need materialized files. qmd needs markdown documents. Other backends may not.

Use a small abstraction instead of making the build pipeline write qmd files directly:

```rust
pub trait MemoryArtifactSink: Send + Sync {
    fn write_record_artifact(
        &self,
        backend: &str,
        project_key: &str,
        record: &MemoryRecord,
    ) -> Result<MemoryArtifact>;

    fn remove_record_artifact(
        &self,
        backend: &str,
        project_key: &str,
        record_id: &str,
    ) -> Result<()>;
}
```

Initial implementations:

- `MarkdownArtifactSink`: writes to `~/.sessio/memory/<backend>/projects/<project_key>/sessions/*.md`.
- `NoopArtifactSink`: used by backends that index records directly without files.
- `TempArtifactSink`: used by tests.

This keeps qmd's markdown requirement out of `memory/build.rs`.

## Memory Service

`MemoryService` should be the only layer that coordinates repository writes and backend sync.

Target responsibilities:

- Build memory for a project or source.
- Persist records, sources, fingerprints, and continuations.
- Emit a `MemoryChangeSet` describing affected project keys and record IDs.
- Submit backend sync jobs.
- Convert backend search hits into stable CLI/UI payloads.
- Record backend job state without using qmd-specific job names in generic paths.

Sketch:

```rust
pub struct MemoryService {
    repository: Arc<dyn MemoryRepository>,
    backend: Arc<dyn MemoryIndexBackend>,
    artifact_sink: Arc<dyn MemoryArtifactSink>,
}

impl MemoryService {
    pub fn build_project(&self, options: MemoryBuildOptions) -> Result<MemoryBuildSummary>;
    pub fn build_source(&self, source: &SessionSource) -> Result<MemoryBuildSourceResult>;
    pub fn sync_project(&self, project_key: &str) -> Result<MemorySyncReport>;
    pub fn search(&self, request: MemorySearchRequest) -> Result<MemorySearchResponse>;
    pub fn resolve(&self, record_id: &str) -> Result<MemoryResolveResponse>;
}
```

The indexer should call `MemoryService`, not `memory::qmd`.

## qmd As One Backend

`QmdBackend` implements `MemoryIndexBackend`:

- `status`: wraps binary discovery and `qmd --version`.
- `sync_project`: writes markdown artifacts through `MarkdownArtifactSink`, ensures collection, runs qmd update.
- `search`: runs qmd search and maps document paths back to record IDs via `memory_artifacts`.

`embed` is not part of the trait. How a backend implements retrieval (BM25 full-text, vector embeddings, hybrid, remote service) is entirely backend-internal. The service layer only supplies `MemoryRecord` corpus and queries. Whether qmd runs `qmd embed` internally is a `QmdBackend` configuration concern, not a generic capability.

Expose backend diagnostics through `sessio memory status` and `sessio memory sync` while qmd remains the current backend. qmd stays an implementation detail behind `MemoryIndexBackend`; any qmd-specific diagnostic options like `--embed` live under the memory namespace rather than a top-level `qmd` command.

## Backend Configuration

A backend configuration block ships in this iteration, even though qmd is the only implementation. Wiring it up now means the config loader, defaults, and override precedence are already in place when a second backend lands — no schema or path layout churn later.

Config file (`~/.sessio/config.toml` or equivalent):

```toml
[memory]
backend = "qmd"

[memory.backends.qmd]
binary = null              # null = auto-discover via PATH
index = "sessio"
artifacts_root = "~/.sessio/memory/qmd/projects"
```

Defaults if the file or section is missing:

- `memory.backend = "qmd"`
- `memory.backends.qmd.binary = null` (auto-discover)
- `memory.backends.qmd.index = "sessio"`
- `memory.backends.qmd.artifacts_root = "~/.sessio/memory/qmd/projects"`

Environment overrides (highest precedence, for CLI testing and developer workflows):

```text
SESSIO_QMD_BINARY=/path/to/qmd
SESSIO_QMD_INDEX=sessio
SESSIO_QMD_ARTIFACTS_ROOT=/tmp/sessio-qmd-test
```

`SESSIO_MEMORY_BACKEND` and a CLI `--backend` flag are intentionally not exposed yet — qmd is the only valid value, so adding the switch would be noise. The internal `MemoryService` resolves the backend by reading `[memory].backend` from config, but only `"qmd"` is accepted; any other value is a config error.

Backend configuration must not leak into provider parsing or memory record generation.

## CLI Contract

Keep stable commands:

```bash
sessio memory build --project <path> --json
sessio memory search --project <path> "<query>" --json
sessio memory resolve --card-id <id> --json
sessio memory jobs --project-key <key> --json
```

No `--backend` flag is added in this iteration. Backend-related CLI surface (`sessio memory backend status`, `sessio memory backend sync`, `--backend <name>`) is deferred until a second backend lands.

Response shape stays backend-neutral:

```json
{
  "query": "provider abstraction",
  "projectKey": "-Users-alex-Work-cloudgeek-sessio",
  "backend": "qmd",
  "backendError": null,
  "hits": [
    {
      "cardId": "sessio-codex-...",
      "title": "Memory backend abstraction",
      "summary": "Sessio should treat qmd as one backend behind a MemoryIndexBackend trait.",
      "score": 0.82,
      "snippet": null,
      "sources": []
    }
  ]
}
```

Avoid adding qmd-specific fields to normal `memory search` output. If debugging needs raw qmd payload, keep it behind `--include-raw`.

## Indexer And CLI Integration

Both the desktop indexer and the CLI go through the same `MemoryService` + job queue. There is no second code path for CLI.

Flow:

```text
caller -> MemoryService build -> MemoryChangeSet -> enqueue MemoryBackendSyncJob -> backend.sync_project
```

- Desktop indexer: fire-and-forget. `MemoryService::build_*` enqueues the job; the worker drains it asynchronously; UI listens for `memory_index_updated` events.
- CLI: same enqueue path, then `MemoryService` blocks on a oneshot signal until the relevant `MemoryBackendSyncJob` for the affected project completes (or fails into a structured `backendError`).

This keeps debounce, retry, and job-state tracking unified — the CLI gets the same observability as the indexer instead of a parallel synchronous code path.

Target job:

```rust
struct MemoryBackendSyncJob {
    backend: String,
    project_key: String,
    project_path: String,
    changed_record_ids: Vec<String>,
    removed_record_ids: Vec<String>,
    dependent_source_paths: Vec<PathBuf>,
}
```

Backend-internal debounce (e.g. qmd batching update+embed) lives inside `QmdBackend` or its worker. The indexer only knows that a memory backend sync is pending.

## Migration Plan

### Phase A: Document And Stabilize

- Keep the current qmd implementation running.
- Preserve existing CLI JSON behavior on stable commands.
- Land this design document and reference it from the qmd memory TODOs.

### Phase B: Backend-Neutral Types And Schema

- Rename `MemoryCard` → `MemoryRecord` outright (no alias).
- Drop the `memory_cards.qmd_path` column.
- Add the `memory_artifacts` table.
- Add `MemoryBackendStatus`, `MemorySearchOptions`, `MemoryBackendHit`, `MemorySyncReport`.
- Tests around backend-neutral search response mapping.

### Phase C: Path Layout And Artifact Sink

- Move artifact root from `~/.sessio/qmd-memory/projects/<key>/cards/` to `~/.sessio/memory/<backend>/projects/<key>/sessions/`.
- Delete the old root on first run; rebuild artifacts from SQLite.
- Move markdown writing/removal out of `memory/build.rs` into `MarkdownArtifactSink`.
- Rename CLI `--cards-root` → `--artifacts-root`.

### Phase D: `MemoryIndexBackend` Trait

- Wrap `memory/qmd.rs` in `QmdBackend`.
- Update CLI memory search to call the backend trait.
- Expose `sessio memory status` / `sessio memory sync` as thin diagnostics over the configured backend; qmd remains hidden behind `QmdBackend`, including diagnostic `--embed`.
- Replace `QmdSyncJob` with `MemoryBackendSyncJob`.
- Map qmd hit document paths back to `record_id` via `memory_artifacts`.

### Phase E: `MemoryService`

- Move build/search/resolve orchestration behind `MemoryService`.
- Indexer and CLI both depend on `MemoryService`.
- CLI mode blocks on the relevant `MemoryBackendSyncJob` via a oneshot signal.
- Repository persistence stays in SQLite.

### Phase F: Validate End-To-End With qmd

- Confirm `memory build` produces identical records before and after the refactor (canonical hashes match).
- Confirm `memory search` returns the same hit payloads as the pre-refactor implementation.
- Confirm indexer + watcher path still triggers backend sync correctly.
- Confirm CLI `memory build` blocks until backend sync completes and surfaces `backendError` on failure.

Proving the abstraction with a second non-qmd backend (e.g. SQLite FTS) is **out of scope** for this iteration. The seam is verified by qmd-through-the-trait alone; second backends land later as additive work.

## Testing Strategy

Unit tests:

- Record generation remains backend-neutral.
- Continuation dedupe does not require backend-specific paths.
- `MarkdownArtifactSink` writes/removes files under the new `~/.sessio/memory/<backend>/projects/<key>/sessions/` layout.
- `QmdBackend` maps qmd document paths back to `record_id` via `memory_artifacts`.

Integration tests:

- `memory build` produces records with identical `canonical_hash` before and after the refactor.
- `memory search` returns the same stable hit payloads through the trait as it did through the direct qmd wrapper.
- Missing qmd binary returns `backendError` without failing JSON output.
- Unavailable records do not appear in search results.
- CLI `memory build` blocks until backend sync completes.

Regression checks:

- `cargo test`
- `cargo check`
- `sessio memory search --project <repo> <query> --json`
- `sessio memory resolve --card-id <id> --json`

## Non-Goals

- Do not redesign provider parsing as part of this refactor.
- Do not change record generation quality in the same change set.
- Do not remove qmd diagnostic commands.
- Do not add a second backend (SQLite FTS, vector, remote) in this iteration.
- Do not expose backend selection to end users (`--backend` flag, `[memory] backend = ...` config) yet.
- Do not upload raw session JSONL anywhere by default.

## Resolved Decisions

The Open Questions previously listed have been resolved:

- **`MemoryRecord` vs `MemoryCard`**: replace outright, no transitional alias.
- **Artifact metadata storage**: `memory_artifacts` table in SQLite from Phase B. Backends do not stat the filesystem to find their own artifacts, and remote/object-store backends would not be able to derive paths deterministically anyway.
- **CLI sync vs async**: CLI and indexer share one path. CLI mode blocks on the relevant `MemoryBackendSyncJob` via a oneshot signal rather than running a parallel synchronous pipeline.
- **`embed` placement**: not part of `MemoryIndexBackend`. How a backend implements retrieval (BM25 / vector / hybrid / local / remote) is opaque to the service layer; `qmd embed` stays as a qmd-internal/diagnostic concern.
- **Backend selection exposure**: config block `[memory] backend = "qmd"` ships in this iteration so the loader is in place when a second backend arrives. No user-facing `--backend` flag and no `SESSIO_MEMORY_BACKEND` env var yet — `"qmd"` is the only accepted value.
