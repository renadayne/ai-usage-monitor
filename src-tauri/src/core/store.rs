//! SQLite persistence: the latest snapshot per provider plus a usage history.

use std::path::Path;
use std::sync::Mutex;

use chrono::{DateTime, Duration, TimeZone, Utc};
use rusqlite::{params, Connection};
use serde::Serialize;

use super::model::{ProviderId, ProviderSnapshot};

pub const HISTORY_RETENTION_DAYS: i64 = 30;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HistoryPoint {
    pub window_id: String,
    pub label: String,
    pub used_percent: f32,
    pub recorded_at: DateTime<Utc>,
}

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS snapshots_latest (
                 provider   TEXT PRIMARY KEY,
                 json       TEXT NOT NULL,
                 fetched_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS snapshots_history (
                 id           INTEGER PRIMARY KEY AUTOINCREMENT,
                 provider     TEXT NOT NULL,
                 window_id    TEXT NOT NULL,
                 label        TEXT NOT NULL,
                 used_percent REAL NOT NULL,
                 recorded_at  INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_history_provider_time
                 ON snapshots_history (provider, recorded_at);",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn save_latest(&self, snapshot: &ProviderSnapshot) -> rusqlite::Result<()> {
        let json = serde_json::to_string(snapshot)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        self.conn().execute(
            "INSERT INTO snapshots_latest (provider, json, fetched_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(provider) DO UPDATE SET json = excluded.json, fetched_at = excluded.fetched_at",
            params![
                snapshot.provider.as_str(),
                json,
                snapshot.fetched_at.timestamp()
            ],
        )?;
        Ok(())
    }

    /// Rows that no longer deserialize (schema changed) are skipped.
    pub fn load_latest(&self) -> rusqlite::Result<Vec<ProviderSnapshot>> {
        let conn = self.conn();
        let mut statement = conn.prepare("SELECT json FROM snapshots_latest")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut snapshots = Vec::new();
        for json in rows {
            if let Ok(snapshot) = serde_json::from_str::<ProviderSnapshot>(&json?) {
                snapshots.push(snapshot);
            }
        }
        Ok(snapshots)
    }

    /// Appends one history row per window that reports a percentage, then prunes old rows.
    pub fn record_history(&self, snapshot: &ProviderSnapshot) -> rusqlite::Result<()> {
        let conn = self.conn();
        let recorded_at = snapshot.fetched_at.timestamp();
        for window in &snapshot.windows {
            if let Some(used) = window.used_percent {
                conn.execute(
                    "INSERT INTO snapshots_history (provider, window_id, label, used_percent, recorded_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        snapshot.provider.as_str(),
                        window.id,
                        window.label,
                        f64::from(used),
                        recorded_at
                    ],
                )?;
            }
        }
        let cutoff = (snapshot.fetched_at - Duration::days(HISTORY_RETENTION_DAYS)).timestamp();
        conn.execute(
            "DELETE FROM snapshots_history WHERE recorded_at < ?1",
            params![cutoff],
        )?;
        Ok(())
    }

    pub fn history(
        &self,
        provider: ProviderId,
        since: DateTime<Utc>,
    ) -> rusqlite::Result<Vec<HistoryPoint>> {
        let conn = self.conn();
        let mut statement = conn.prepare(
            "SELECT window_id, label, used_percent, recorded_at FROM snapshots_history
             WHERE provider = ?1 AND recorded_at >= ?2 ORDER BY recorded_at ASC, id ASC",
        )?;
        let rows = statement.query_map(params![provider.as_str(), since.timestamp()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        let mut points = Vec::new();
        for row in rows {
            let (window_id, label, used_percent, recorded_at) = row?;
            if let Some(recorded_at) = Utc.timestamp_opt(recorded_at, 0).single() {
                points.push(HistoryPoint {
                    window_id,
                    label,
                    used_percent: used_percent as f32,
                    recorded_at,
                });
            }
        }
        Ok(points)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::model::{Activity, Health, QuotaWindow};

    fn snapshot(provider: ProviderId, used: f32, at: DateTime<Utc>) -> ProviderSnapshot {
        ProviderSnapshot {
            provider,
            activity: Activity::Active,
            health: Health::Ok,
            plan: Some("pro".into()),
            current_model: Some("model-x".into()),
            last_activity_at: Some(at),
            windows: vec![
                QuotaWindow::percent("five_hour", "5 hours", used),
                QuotaWindow {
                    used_percent: None,
                    ..QuotaWindow::percent("unknown", "Unknown", 0.0)
                },
            ],
            note: None,
            fetched_at: at,
        }
    }

    fn at(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_800_000_000 + seconds, 0).unwrap()
    }

    #[test]
    fn latest_is_upserted_per_provider() {
        let store = Store::open_in_memory().unwrap();
        store
            .save_latest(&snapshot(ProviderId::Claude, 10.0, at(0)))
            .unwrap();
        store
            .save_latest(&snapshot(ProviderId::Claude, 20.0, at(60)))
            .unwrap();
        store
            .save_latest(&snapshot(ProviderId::Codex, 5.0, at(60)))
            .unwrap();

        let mut latest = store.load_latest().unwrap();
        latest.sort_by_key(|s| s.provider.as_str());
        assert_eq!(latest.len(), 2);
        assert_eq!(latest[0], snapshot(ProviderId::Claude, 20.0, at(60)));
        assert_eq!(latest[1].provider, ProviderId::Codex);
    }

    #[test]
    fn history_records_only_windows_with_percent() {
        let store = Store::open_in_memory().unwrap();
        store
            .record_history(&snapshot(ProviderId::Claude, 10.0, at(0)))
            .unwrap();
        store
            .record_history(&snapshot(ProviderId::Claude, 30.0, at(300)))
            .unwrap();
        store
            .record_history(&snapshot(ProviderId::Codex, 99.0, at(300)))
            .unwrap();

        let points = store.history(ProviderId::Claude, at(-10)).unwrap();
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].window_id, "five_hour");
        assert_eq!(points[0].used_percent, 10.0);
        assert_eq!(points[1].used_percent, 30.0);
        assert_eq!(points[1].recorded_at, at(300));

        assert_eq!(store.history(ProviderId::Claude, at(100)).unwrap().len(), 1);
    }

    #[test]
    fn history_older_than_retention_is_pruned() {
        let store = Store::open_in_memory().unwrap();
        store
            .record_history(&snapshot(ProviderId::Claude, 10.0, at(0)))
            .unwrap();
        let later = at(0) + Duration::days(HISTORY_RETENTION_DAYS + 1);
        store
            .record_history(&snapshot(ProviderId::Claude, 50.0, later))
            .unwrap();

        let points = store.history(ProviderId::Claude, at(-10)).unwrap();
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].used_percent, 50.0);
    }

    #[test]
    fn persists_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.sqlite");
        {
            let store = Store::open(&path).unwrap();
            store
                .save_latest(&snapshot(ProviderId::Copilot, 42.0, at(0)))
                .unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.load_latest().unwrap().len(), 1);
    }
}
