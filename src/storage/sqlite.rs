use super::{ToolCallRecord, ToolRegistry};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::path::Path;
use std::sync::Arc;
use tokio_rusqlite::Connection as AsyncConnection;
use tracing::{debug, info};

pub struct StorageManager {
    conn: Arc<AsyncConnection>,
}

impl StorageManager {
    pub async fn new(db_path: &str) -> Result<Self> {
        // Expand ~ in path
        let expanded_path = shellexpand::tilde(db_path).to_string();

        // Create parent directory if it doesn't exist
        if let Some(parent) = Path::new(&expanded_path).parent() {
            tokio::fs::create_dir_all(parent).await.with_context(|| {
                format!("Failed to create database directory: {}", parent.display())
            })?;
        }

        info!("Opening database at: {}", expanded_path);

        let conn = AsyncConnection::open(&expanded_path)
            .await
            .with_context(|| format!("Failed to open database: {}", expanded_path))?;

        let manager = Self {
            conn: Arc::new(conn),
        };

        manager.initialize_schema().await?;
        Ok(manager)
    }

    async fn initialize_schema(&self) -> Result<()> {
        self.conn
            .call(|conn| {
                conn.execute(
                    r#"
                    CREATE TABLE IF NOT EXISTS tool_calls (
                        id          INTEGER PRIMARY KEY AUTOINCREMENT,
                        tool_id     TEXT    NOT NULL,
                        server_name TEXT    NOT NULL,
                        tool_name   TEXT    NOT NULL,
                        success     BOOLEAN NOT NULL,
                        latency_ms  INTEGER NOT NULL,
                        error_type  TEXT,
                        called_at   INTEGER NOT NULL
                    )
                    "#,
                    [],
                )?;

                conn.execute(
                    r#"
                    CREATE INDEX IF NOT EXISTS idx_tool_calls_tool_id_time 
                    ON tool_calls(tool_id, called_at)
                    "#,
                    [],
                )?;

                conn.execute(
                    r#"
                    CREATE TABLE IF NOT EXISTS tool_registry (
                        tool_id     TEXT PRIMARY KEY,
                        server_name TEXT NOT NULL,
                        tool_name   TEXT NOT NULL,
                        description TEXT NOT NULL DEFAULT '',
                        schema_json TEXT NOT NULL DEFAULT '',
                        first_seen  INTEGER NOT NULL,
                        last_seen   INTEGER NOT NULL
                    )
                    "#,
                    [],
                )?;

                conn.execute(
                    r#"
                    CREATE TABLE IF NOT EXISTS daily_stats (
                        tool_id       TEXT    NOT NULL,
                        date          TEXT    NOT NULL,
                        call_count    INTEGER NOT NULL,
                        success_count INTEGER NOT NULL,
                        avg_latency   REAL    NOT NULL,
                        p95_latency   REAL    NOT NULL,
                        PRIMARY KEY (tool_id, date)
                    )
                    "#,
                    [],
                )?;

                Ok::<_, tokio_rusqlite::Error>(())
            })
            .await
            .context("Failed to initialize database schema")?;

        debug!("Database schema initialized");
        Ok(())
    }

    pub async fn record_tool_call(
        &self,
        tool_id: &str,
        server_name: &str,
        tool_name: &str,
        success: bool,
        latency_ms: u64,
        error_type: Option<String>,
    ) -> Result<()> {
        let tool_id = tool_id.to_string();
        let server_name = server_name.to_string();
        let tool_name = tool_name.to_string();
        let now = Utc::now();

        self.conn
            .call(move |conn| {
                conn.execute(
                    r#"
                    INSERT INTO tool_calls (tool_id, server_name, tool_name, success, latency_ms, error_type, called_at)
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                    "#,
                    rusqlite::params![
                        tool_id,
                        server_name,
                        tool_name,
                        success,
                        latency_ms.min(i64::MAX as u64) as i64,
                        error_type,
                        now.timestamp()
                    ],
                )?;
                Ok::<_, tokio_rusqlite::Error>(())
            })
            .await
            .context("Failed to record tool call")?;

        Ok(())
    }

    pub async fn get_recent_calls(
        &self,
        tool_id: &str,
        limit: usize,
    ) -> Result<Vec<ToolCallRecord>> {
        let tool_id = tool_id.to_string();

        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    r#"
                    SELECT id, tool_id, server_name, tool_name, success, latency_ms, error_type, called_at
                    FROM tool_calls
                    WHERE tool_id = ?1
                    ORDER BY called_at DESC
                    LIMIT ?2
                    "#,
                )?;

                let records = stmt
                    .query_map(rusqlite::params![tool_id, limit as i64], |row| {
                        Ok(ToolCallRecord {
                            id: row.get(0)?,
                            tool_id: row.get(1)?,
                            server_name: row.get(2)?,
                            tool_name: row.get(3)?,
                            success: row.get(4)?,
                            latency_ms: row.get::<_, i64>(5)? as u64,
                            error_type: row.get(6)?,
                            called_at: DateTime::from_timestamp(row.get(7)?, 0)
                                .unwrap_or_else(|| Utc::now()),
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;

                Ok(records)
            })
            .await
            .context("Failed to get recent calls")
    }

    pub async fn get_p95_latency(&self, tool_id: &str, window_size: usize) -> Result<f64> {
        let tool_id = tool_id.to_string();

        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    r#"
                    SELECT latency_ms
                    FROM tool_calls
                    WHERE tool_id = ?1 AND success = 1
                    ORDER BY called_at DESC
                    LIMIT ?2
                    "#,
                )?;

                let latencies: Vec<i64> = stmt
                    .query_map(rusqlite::params![tool_id, window_size as i64], |row| {
                        row.get(0)
                    })?
                    .collect::<Result<Vec<_>, _>>()?;

                if latencies.is_empty() {
                    return Ok(0.0);
                }

                let mut sorted = latencies.clone();
                sorted.sort_unstable();

                let p95_index = ((sorted.len() as f64) * 0.95) as usize;
                let p95_index = p95_index.min(sorted.len() - 1);

                Ok(sorted[p95_index] as f64)
            })
            .await
            .context("Failed to calculate p95 latency")
    }

    pub async fn get_call_count_window(&self, tool_id: &str, days: u64) -> Result<u32> {
        let tool_id = tool_id.to_string();
        let cutoff = Utc::now().timestamp() - (days * 86400) as i64;

        self.conn
            .call(move |conn| {
                let count: i64 = conn.query_row(
                    r#"
                    SELECT COUNT(*)
                    FROM tool_calls
                    WHERE tool_id = ?1 AND called_at >= ?2
                    "#,
                    rusqlite::params![tool_id, cutoff],
                    |row| row.get(0),
                )?;

                Ok(count as u32)
            })
            .await
            .context("Failed to get call count")
    }

    pub async fn register_tool(&self, tool: ToolRegistry) -> Result<()> {
        self.conn
            .call(move |conn| {
                conn.execute(
                    r#"
                    INSERT OR REPLACE INTO tool_registry 
                    (tool_id, server_name, tool_name, description, schema_json, first_seen, last_seen)
                    VALUES (?1, ?2, ?3, ?4, ?5, 
                        COALESCE((SELECT first_seen FROM tool_registry WHERE tool_id = ?1), ?6),
                        ?7)
                    "#,
                    rusqlite::params![
                        tool.tool_id,
                        tool.server_name,
                        tool.tool_name,
                        tool.description,
                        tool.schema_json,
                        tool.first_seen.timestamp(),
                        tool.last_seen.timestamp(),
                    ],
                )?;
                Ok::<_, tokio_rusqlite::Error>(())
            })
            .await
            .context("Failed to register tool")?;

        Ok(())
    }

    pub async fn get_all_registered_tools(&self) -> Result<Vec<ToolRegistry>> {
        self.conn
            .call(|conn| {
                let mut stmt = conn.prepare(
                    r#"
                    SELECT tool_id, server_name, tool_name, description, schema_json, first_seen, last_seen
                    FROM tool_registry
                    ORDER BY tool_id
                    "#,
                )?;

                let tools = stmt
                    .query_map([], |row| {
                        Ok(ToolRegistry {
                            tool_id: row.get(0)?,
                            server_name: row.get(1)?,
                            tool_name: row.get(2)?,
                            description: row.get(3)?,
                            schema_json: row.get(4)?,
                            first_seen: DateTime::from_timestamp(row.get(5)?, 0)
                                .unwrap_or_else(|| Utc::now()),
                            last_seen: DateTime::from_timestamp(row.get(6)?, 0)
                                .unwrap_or_else(|| Utc::now()),
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;

                Ok(tools)
            })
            .await
            .context("Failed to get registered tools")
    }

    pub async fn aggregate_daily_stats(&self, date: &str) -> Result<()> {
        let date_owned = date.to_string();

        self.conn
            .call(move |conn| {
                let date = date_owned;
                // Previous single-statement version put a correlated subquery
                // inside LIMIT/OFFSET, which SQLite rejects with
                // "no such column: tool_calls.tool_id" (found in eval round 2:
                // daily aggregation never worked). Rewritten with a window
                // function: ROW_NUMBER over (tool_id, success) ordered by
                // latency gives the p95 row directly, no OFFSET needed.
                conn.execute(
                    r#"
                    INSERT OR REPLACE INTO daily_stats (tool_id, date, call_count, success_count, avg_latency, p95_latency)
                    WITH ranked AS (
                        SELECT tool_id, success, latency_ms,
                            ROW_NUMBER() OVER (
                                PARTITION BY tool_id, success ORDER BY latency_ms
                            ) AS rn,
                            COUNT(*) OVER (PARTITION BY tool_id, success) AS cnt
                        FROM tool_calls
                        WHERE date(called_at, 'unixepoch') = date(?1)
                    ),
                    p95 AS (
                        SELECT tool_id, latency_ms FROM ranked
                        WHERE success = 1 AND rn = CAST(cnt * 0.95 AS INTEGER)
                    )
                    SELECT t.tool_id,
                        date(?1),
                        COUNT(*),
                        SUM(CASE WHEN t.success = 1 THEN 1 ELSE 0 END),
                        AVG(t.latency_ms),
                        COALESCE((SELECT p.latency_ms FROM p95 p WHERE p.tool_id = t.tool_id), 0.0)
                    FROM tool_calls t
                    WHERE date(t.called_at, 'unixepoch') = date(?1)
                    GROUP BY t.tool_id
                    "#,
                    rusqlite::params![date],
                )?;
                Ok::<_, tokio_rusqlite::Error>(())
            })
            .await
            .context("Failed to aggregate daily stats")?;

        debug!("Aggregated daily stats for {}", date);
        Ok(())
    }

    pub async fn cleanup_old_records(&self, retention_days: u64) -> Result<usize> {
        let cutoff = Utc::now().timestamp() - (retention_days * 86400) as i64;

        self.conn
            .call(move |conn| {
                let count = conn.execute(
                    "DELETE FROM tool_calls WHERE called_at < ?1",
                    rusqlite::params![cutoff],
                )?;
                Ok(count)
            })
            .await
            .context("Failed to cleanup old records")
    }
}
