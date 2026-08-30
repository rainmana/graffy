//! Memory & persistence (ADR-0006) — embedded libSQL, schema v1 (Phase 1 M4).
//!
//! One database file (`graffy.db` under the platform data dir) carries:
//! * the **graph registry** — installed durable graph objects with their
//!   verbatim TOML, SHA-256, provenance source (builtin / imported /
//!   authored), and install time; `graffy graph list/export/import` read and
//!   write here,
//! * **run history** — one row per run with status, totals, and the journal
//!   path (the journal file stays canonical; this is the queryable index),
//! * the **journal mirror** — every RunEvent as a row (protobuf payload +
//!   kind column), so Phase 3 detectors and Phase 4 evals can query runs
//!   with SQL instead of re-parsing files,
//! * **sessions** — coordination sessions spanning runs (Phase 3 wake-up
//!   layers build on this).
//!
//! Later phases add: verbatim episodic log with FTS, native vector search
//! (`F32_BLOB` + `vector_top_k`, embeddings via fastembed or cloud), and the
//! temporal knowledge graph with validity windows — all in this same file,
//! all behind migrations (`meta.schema_version`).

use std::path::{Path, PathBuf};

use thiserror::Error;

use graffy_core::error::{CompileError, SpecError};
use graffy_core::exec::sha256_hex;
use graffy_core::graph::CompiledGraph;
use graffy_core::journal::{event_kind, wire};
use graffy_core::spec::GraphSpec;
use graffy_proto::prost::Message;

/// Storage errors.
#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("store io: {0}")]
    Io(#[from] std::io::Error),
    #[error("libsql: {0}")]
    Db(#[from] libsql::Error),
    #[error(transparent)]
    Spec(#[from] SpecError),
    #[error(transparent)]
    Compile(#[from] CompileError),
    #[error("store corruption: {0}")]
    Corrupt(String),
}

/// Schema v1 — every statement idempotent; `meta.schema_version` gates
/// future migrations.
const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS graphs (
  id           TEXT PRIMARY KEY,
  name         TEXT NOT NULL,
  version      TEXT NOT NULL,
  description  TEXT NOT NULL DEFAULT '',
  tags         TEXT NOT NULL DEFAULT '',
  license      TEXT NOT NULL DEFAULT '',
  source       TEXT NOT NULL,
  spec_toml    TEXT NOT NULL,
  spec_sha256  TEXT NOT NULL,
  installed_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS runs (
  run_id        TEXT PRIMARY KEY,
  graph_id      TEXT NOT NULL,
  graph_name    TEXT NOT NULL,
  session_id    TEXT NOT NULL,
  status        TEXT NOT NULL,
  started_at    INTEGER NOT NULL,
  duration_ms   INTEGER NOT NULL,
  input_tokens  INTEGER NOT NULL,
  output_tokens INTEGER NOT NULL,
  total_usd     REAL NOT NULL,
  journal_path  TEXT NOT NULL,
  spec_sha256   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_runs_started ON runs(started_at DESC);
CREATE TABLE IF NOT EXISTS sessions (
  session_id  TEXT PRIMARY KEY,
  created_at  INTEGER NOT NULL,
  last_run_at INTEGER NOT NULL,
  run_count   INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS run_events (
  run_id     TEXT NOT NULL,
  seq        INTEGER NOT NULL,
  at_unix_ms INTEGER NOT NULL,
  kind       TEXT NOT NULL,
  payload    BLOB NOT NULL,
  PRIMARY KEY (run_id, seq)
);
CREATE INDEX IF NOT EXISTS idx_events_kind ON run_events(run_id, kind);
CREATE TABLE IF NOT EXISTS mcp_servers (
  name           TEXT PRIMARY KEY,
  transport      TEXT NOT NULL,
  command        TEXT NOT NULL,
  args           TEXT NOT NULL DEFAULT '',
  role_default   TEXT NOT NULL DEFAULT 'effector',
  evidence_level TEXT NOT NULL DEFAULT 'L1',
  tools_json     TEXT NOT NULL DEFAULT '[]',
  added_at       INTEGER NOT NULL
);
INSERT OR IGNORE INTO meta (key, value) VALUES ('schema_version', '1');
INSERT OR REPLACE INTO meta (key, value) VALUES ('schema_version', '2');
"#;

/// An installed graph, as stored.
#[derive(Debug, Clone)]
pub struct GraphRecord {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub tags: Vec<String>,
    pub license: String,
    /// "builtin" | "imported" | "authored".
    pub source: String,
    pub spec_toml: String,
    pub spec_sha256: String,
    pub installed_at: i64,
}

/// One run's index row (the journal file stays canonical).
#[derive(Debug, Clone)]
pub struct RunRecord {
    pub run_id: String,
    pub graph_id: String,
    pub graph_name: String,
    pub session_id: String,
    pub status: String,
    pub started_at: i64,
    pub duration_ms: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_usd: f64,
    pub journal_path: String,
    pub spec_sha256: String,
}

/// A registered MCP server (transport binding lives here, never in specs —
/// docs/design/phase-2-mcp.md §5).
#[derive(Debug, Clone)]
pub struct McpServerRecord {
    pub name: String,
    /// "stdio" | "http" (http wiring lands in the next slice).
    pub transport: String,
    /// Executable (stdio) or URL (http).
    pub command: String,
    /// Space-separated argv tail for stdio servers.
    pub args: String,
    /// Default role for tools lacking annotations: "evidence" | "effector".
    pub role_default: String,
    /// Evidence level granted to this server's results (L0–L2).
    pub evidence_level: String,
    /// Cached discovery metadata (JSON array of tools).
    pub tools_json: String,
    pub added_at: i64,
}

/// The embedded store. One per data dir; open is idempotent and migrating.
pub struct Store {
    _db: libsql::Database,
    conn: libsql::Connection,
    path: PathBuf,
}

impl Store {
    /// Open (creating and migrating as needed) the store at `path`.
    pub async fn open(path: &Path) -> Result<Self, MemoryError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let db = libsql::Builder::new_local(path).build().await?;
        let conn = db.connect()?;
        conn.execute_batch(SCHEMA_V1).await?;
        Ok(Self {
            _db: db,
            conn,
            path: path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Validate (parse + compile) and upsert a graph spec.
    pub async fn register_graph(
        &self,
        spec_toml: &str,
        source: &str,
    ) -> Result<GraphRecord, MemoryError> {
        let spec = GraphSpec::from_toml_str(spec_toml)?;
        CompiledGraph::compile(&spec)?;

        let record = GraphRecord {
            id: spec.graph.id.clone(),
            name: spec.graph.name.clone(),
            version: spec.graph.version.clone(),
            description: spec.graph.description.clone(),
            tags: spec.graph.tags.clone(),
            license: spec.graph.license.clone().unwrap_or_default(),
            source: source.to_owned(),
            spec_toml: spec_toml.to_owned(),
            spec_sha256: sha256_hex(spec_toml.as_bytes()),
            installed_at: unix_now(),
        };
        self.conn
            .execute(
                "INSERT INTO graphs
                   (id, name, version, description, tags, license, source,
                    spec_toml, spec_sha256, installed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(id) DO UPDATE SET
                   name = excluded.name,
                   version = excluded.version,
                   description = excluded.description,
                   tags = excluded.tags,
                   license = excluded.license,
                   source = excluded.source,
                   spec_toml = excluded.spec_toml,
                   spec_sha256 = excluded.spec_sha256,
                   installed_at = excluded.installed_at",
                libsql::params![
                    record.id.clone(),
                    record.name.clone(),
                    record.version.clone(),
                    record.description.clone(),
                    record.tags.join(","),
                    record.license.clone(),
                    record.source.clone(),
                    record.spec_toml.clone(),
                    record.spec_sha256.clone(),
                    record.installed_at,
                ],
            )
            .await?;
        Ok(record)
    }

    /// Ensure shipped built-ins exist (idempotent; upgrades on content change).
    pub async fn seed_builtins(&self, builtins: &[(&str, &str)]) -> Result<usize, MemoryError> {
        let mut seeded = 0;
        for (id, toml) in builtins {
            let sha = sha256_hex(toml.as_bytes());
            let existing: Option<String> = {
                let mut rows = self
                    .conn
                    .query(
                        "SELECT spec_sha256 FROM graphs WHERE id = ?1",
                        libsql::params![(*id).to_owned()],
                    )
                    .await?;
                match rows.next().await? {
                    Some(row) => Some(row.get::<String>(0)?),
                    None => None,
                }
            };
            if existing.as_deref() != Some(sha.as_str()) {
                self.register_graph(toml, "builtin").await?;
                seeded += 1;
            }
        }
        Ok(seeded)
    }

    pub async fn list_graphs(&self) -> Result<Vec<GraphRecord>, MemoryError> {
        let mut rows = self
            .conn
            .query(
                "SELECT id, name, version, description, tags, license, source,
                        spec_toml, spec_sha256, installed_at
                 FROM graphs ORDER BY id",
                (),
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(graph_from_row(&row)?);
        }
        Ok(out)
    }

    pub async fn get_graph(&self, id: &str) -> Result<Option<GraphRecord>, MemoryError> {
        let mut rows = self
            .conn
            .query(
                "SELECT id, name, version, description, tags, license, source,
                        spec_toml, spec_sha256, installed_at
                 FROM graphs WHERE id = ?1",
                libsql::params![id.to_owned()],
            )
            .await?;
        match rows.next().await? {
            Some(row) => Ok(Some(graph_from_row(&row)?)),
            None => Ok(None),
        }
    }

    /// Index a finished run and bump its session.
    pub async fn record_run(&self, run: &RunRecord) -> Result<(), MemoryError> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO runs
                   (run_id, graph_id, graph_name, session_id, status, started_at,
                    duration_ms, input_tokens, output_tokens, total_usd,
                    journal_path, spec_sha256)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                libsql::params![
                    run.run_id.clone(),
                    run.graph_id.clone(),
                    run.graph_name.clone(),
                    run.session_id.clone(),
                    run.status.clone(),
                    run.started_at,
                    run.duration_ms,
                    run.input_tokens,
                    run.output_tokens,
                    run.total_usd,
                    run.journal_path.clone(),
                    run.spec_sha256.clone(),
                ],
            )
            .await?;
        self.conn
            .execute(
                "INSERT INTO sessions (session_id, created_at, last_run_at, run_count)
                 VALUES (?1, ?2, ?2, 1)
                 ON CONFLICT(session_id) DO UPDATE SET
                   last_run_at = excluded.last_run_at,
                   run_count = sessions.run_count + 1",
                libsql::params![run.session_id.clone(), run.started_at],
            )
            .await?;
        Ok(())
    }

    /// Mirror journal frames into the queryable event table.
    pub async fn mirror_journal(&self, events: &[wire::RunEvent]) -> Result<usize, MemoryError> {
        let mut inserted = 0;
        for frame in events {
            let at_unix_ms = frame
                .at
                .as_ref()
                .map(|t| t.seconds * 1000 + i64::from(t.nanos) / 1_000_000)
                .unwrap_or_default();
            let n = self
                .conn
                .execute(
                    "INSERT OR IGNORE INTO run_events
                       (run_id, seq, at_unix_ms, kind, payload)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    libsql::params![
                        frame.run_id.clone(),
                        frame.seq as i64,
                        at_unix_ms,
                        event_kind(frame).to_owned(),
                        frame.encode_to_vec(),
                    ],
                )
                .await?;
            inserted += n as usize;
        }
        Ok(inserted)
    }

    pub async fn recent_runs(&self, limit: u32) -> Result<Vec<RunRecord>, MemoryError> {
        let mut rows = self
            .conn
            .query(
                "SELECT run_id, graph_id, graph_name, session_id, status, started_at,
                        duration_ms, input_tokens, output_tokens, total_usd,
                        journal_path, spec_sha256
                 FROM runs ORDER BY started_at DESC, run_id DESC LIMIT ?1",
                libsql::params![i64::from(limit)],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(RunRecord {
                run_id: row.get::<String>(0)?,
                graph_id: row.get::<String>(1)?,
                graph_name: row.get::<String>(2)?,
                session_id: row.get::<String>(3)?,
                status: row.get::<String>(4)?,
                started_at: row.get::<i64>(5)?,
                duration_ms: row.get::<i64>(6)?,
                input_tokens: row.get::<i64>(7)?,
                output_tokens: row.get::<i64>(8)?,
                total_usd: row.get::<f64>(9)?,
                journal_path: row.get::<String>(10)?,
                spec_sha256: row.get::<String>(11)?,
            });
        }
        Ok(out)
    }

    /// Register (upsert) an MCP server binding.
    pub async fn add_mcp_server(&self, server: &McpServerRecord) -> Result<(), MemoryError> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO mcp_servers
                   (name, transport, command, args, role_default, evidence_level,
                    tools_json, added_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                libsql::params![
                    server.name.clone(),
                    server.transport.clone(),
                    server.command.clone(),
                    server.args.clone(),
                    server.role_default.clone(),
                    server.evidence_level.clone(),
                    server.tools_json.clone(),
                    server.added_at,
                ],
            )
            .await?;
        Ok(())
    }

    pub async fn list_mcp_servers(&self) -> Result<Vec<McpServerRecord>, MemoryError> {
        let mut rows = self
            .conn
            .query(
                "SELECT name, transport, command, args, role_default, evidence_level,
                        tools_json, added_at
                 FROM mcp_servers ORDER BY name",
                (),
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(McpServerRecord {
                name: row.get::<String>(0)?,
                transport: row.get::<String>(1)?,
                command: row.get::<String>(2)?,
                args: row.get::<String>(3)?,
                role_default: row.get::<String>(4)?,
                evidence_level: row.get::<String>(5)?,
                tools_json: row.get::<String>(6)?,
                added_at: row.get::<i64>(7)?,
            });
        }
        Ok(out)
    }

    pub async fn get_mcp_server(&self, name: &str) -> Result<Option<McpServerRecord>, MemoryError> {
        let servers = self.list_mcp_servers().await?;
        Ok(servers.into_iter().find(|s| s.name == name))
    }

    /// Counts for `graffy doctor`.
    pub async fn stats(&self) -> Result<(u64, u64, u64), MemoryError> {
        let graphs = self.count("SELECT COUNT(*) FROM graphs").await?;
        let runs = self.count("SELECT COUNT(*) FROM runs").await?;
        let events = self.count("SELECT COUNT(*) FROM run_events").await?;
        Ok((graphs, runs, events))
    }

    async fn count(&self, sql: &str) -> Result<u64, MemoryError> {
        let mut rows = self.conn.query(sql, ()).await?;
        let row = rows
            .next()
            .await?
            .ok_or_else(|| MemoryError::Corrupt("count query returned no row".into()))?;
        Ok(row.get::<i64>(0)? as u64)
    }
}

fn graph_from_row(row: &libsql::Row) -> Result<GraphRecord, MemoryError> {
    let tags_raw = row.get::<String>(4)?;
    Ok(GraphRecord {
        id: row.get::<String>(0)?,
        name: row.get::<String>(1)?,
        version: row.get::<String>(2)?,
        description: row.get::<String>(3)?,
        tags: if tags_raw.is_empty() {
            Vec::new()
        } else {
            tags_raw.split(',').map(str::to_owned).collect()
        },
        license: row.get::<String>(5)?,
        source: row.get::<String>(6)?,
        spec_toml: row.get::<String>(7)?,
        spec_sha256: row.get::<String>(8)?,
        installed_at: row.get::<i64>(9)?,
    })
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use graffy_proto::journal::v1::run_event::Event;

    fn temp_db(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "graffy-store-test-{tag}-{}.db",
            graffy_core::id::RunId::generate()
        ))
    }

    #[tokio::test]
    async fn register_list_get_roundtrip_with_validation() {
        let path = temp_db("roundtrip");
        let store = Store::open(&path).await.unwrap();

        let toml = graffy_graphs::DEFAULT_CONVERSATION_TOML;
        let record = store.register_graph(toml, "builtin").await.unwrap();
        assert_eq!(record.id, "graffy.builtin.conversation");
        assert_eq!(record.spec_sha256, sha256_hex(toml.as_bytes()));

        let listed = store.list_graphs().await.unwrap();
        assert_eq!(listed.len(), 1);
        let fetched = store
            .get_graph("graffy.builtin.conversation")
            .await
            .unwrap()
            .expect("registered graph must be fetchable");
        assert_eq!(fetched.spec_toml, toml);
        assert!(fetched.tags.contains(&"builtin".to_owned()));

        // Invalid specs are rejected at the door — the registry never holds
        // a graph that cannot compile.
        let bad = r#"
            [graph]
            id = "t.bad"
            name = "Bad"
            version = "0"
            [[node]]
            id = "a"
            kind = "model"
            [[node]]
            id = "b"
            kind = "model"
            [[edge]]
            from = "a"
            to = "b"
            [[edge]]
            from = "b"
            to = "a"
        "#;
        assert!(matches!(
            store.register_graph(bad, "imported").await,
            Err(MemoryError::Compile(CompileError::UnguardedCycle))
        ));
        std::fs::remove_file(&path).ok();
    }

    #[tokio::test]
    async fn seeding_is_idempotent_and_upgrades_on_change() {
        let path = temp_db("seed");
        let store = Store::open(&path).await.unwrap();
        let builtins = graffy_graphs::builtin_specs();

        let first = store.seed_builtins(&builtins).await.unwrap();
        assert_eq!(first, builtins.len());
        let second = store.seed_builtins(&builtins).await.unwrap();
        assert_eq!(second, 0, "unchanged built-ins must not reseed");
        assert_eq!(store.list_graphs().await.unwrap().len(), builtins.len());
        std::fs::remove_file(&path).ok();
    }

    #[tokio::test]
    async fn mcp_server_registry_roundtrip() {
        let path = temp_db("mcp");
        let store = Store::open(&path).await.unwrap();
        store
            .add_mcp_server(&McpServerRecord {
                name: "everything".into(),
                transport: "stdio".into(),
                command: "npx".into(),
                args: "-y @modelcontextprotocol/server-everything".into(),
                role_default: "evidence".into(),
                evidence_level: "L1".into(),
                tools_json: "[]".into(),
                added_at: 1_700_000_000,
            })
            .await
            .unwrap();
        let servers = store.list_mcp_servers().await.unwrap();
        assert_eq!(servers.len(), 1);
        let fetched = store.get_mcp_server("everything").await.unwrap().unwrap();
        assert_eq!(fetched.transport, "stdio");
        assert_eq!(fetched.role_default, "evidence");
        assert!(store.get_mcp_server("ghost").await.unwrap().is_none());
        std::fs::remove_file(&path).ok();
    }

    #[tokio::test]
    async fn run_recording_session_bump_and_journal_mirror() {
        let path = temp_db("runs");
        let store = Store::open(&path).await.unwrap();

        let run = RunRecord {
            run_id: "run_TEST".into(),
            graph_id: "g".into(),
            graph_name: "G".into(),
            session_id: "ses_TEST".into(),
            status: "Succeeded".into(),
            started_at: 1_700_000_000,
            duration_ms: 42,
            input_tokens: 10,
            output_tokens: 20,
            total_usd: 0.0,
            journal_path: "/tmp/x.journal".into(),
            spec_sha256: "abc".into(),
        };
        store.record_run(&run).await.unwrap();
        store.record_run(&run).await.unwrap(); // idempotent on run_id

        let frames = vec![
            wire::RunEvent {
                run_id: "run_TEST".into(),
                seq: 1,
                at: Some(graffy_core::exec::now_ts()),
                event: Some(Event::RunStarted(wire::RunManifest::default())),
            },
            wire::RunEvent {
                run_id: "run_TEST".into(),
                seq: 2,
                at: Some(graffy_core::exec::now_ts()),
                event: Some(Event::RunFinished(wire::RunFinished::default())),
            },
        ];
        assert_eq!(store.mirror_journal(&frames).await.unwrap(), 2);
        assert_eq!(
            store.mirror_journal(&frames).await.unwrap(),
            0,
            "mirror is idempotent"
        );

        let recent = store.recent_runs(10).await.unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].run_id, "run_TEST");

        let (graphs, runs, events) = store.stats().await.unwrap();
        assert_eq!((graphs, runs, events), (0, 1, 2));
        std::fs::remove_file(&path).ok();
    }
}
