use std::{fs, path::Path, sync::Mutex};

use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use crate::models::{AiAnalysis, AppSettings, FocusSegment, FocusSession, ReportPeriod};

pub struct Database {
    connection: Mutex<Connection>,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let connection = Connection::open(path).map_err(|error| error.to_string())?;
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|error| error.to_string())?;
        if version > 1 {
            return Err(format!(
                "Database version {version} is newer than this app supports"
            ));
        }
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 PRAGMA journal_mode = WAL;
                 CREATE TABLE IF NOT EXISTS settings (
                   key TEXT PRIMARY KEY,
                   value TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS focus_sessions (
                   id TEXT PRIMARY KEY,
                   started_at_ms INTEGER NOT NULL,
                   ended_at_ms INTEGER,
                   planned_seconds INTEGER NOT NULL,
                   outcome TEXT NOT NULL CHECK(outcome IN (
                     'running', 'completed', 'completed_after_pause', 'interrupted'
                   ))
                 );
                 CREATE TABLE IF NOT EXISTS focus_segments (
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   session_id TEXT NOT NULL REFERENCES focus_sessions(id) ON DELETE CASCADE,
                   started_at_ms INTEGER NOT NULL,
                   ended_at_ms INTEGER NOT NULL,
                   CHECK(ended_at_ms >= started_at_ms)
                 );
                 CREATE INDEX IF NOT EXISTS idx_segments_range
                   ON focus_segments(started_at_ms, ended_at_ms);
                 CREATE TABLE IF NOT EXISTS analysis_reports (
                   id TEXT PRIMARY KEY,
                   period_start TEXT NOT NULL,
                   period_end TEXT NOT NULL,
                   data_hash TEXT NOT NULL,
                   model TEXT NOT NULL,
                   content TEXT NOT NULL,
                   generated_at_ms INTEGER NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS idx_reports_period
                   ON analysis_reports(period_start, period_end, generated_at_ms DESC);
                 PRAGMA user_version = 1;",
            )
            .map_err(|error| error.to_string())?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn load_settings(&self) -> Result<AppSettings, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let value: Option<String> = connection
            .query_row("SELECT value FROM settings WHERE key = 'app'", [], |row| {
                row.get(0)
            })
            .optional()
            .map_err(|error| error.to_string())?;
        match value {
            Some(value) => serde_json::from_str(&value).map_err(|error| error.to_string()),
            None => Ok(AppSettings::default()),
        }
    }

    pub fn save_settings(&self, settings: &AppSettings) -> Result<(), String> {
        settings.validate()?;
        let value = serde_json::to_string(settings).map_err(|error| error.to_string())?;
        self.connection
            .lock()
            .map_err(|error| error.to_string())?
            .execute(
                "INSERT INTO settings(key, value) VALUES('app', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [value],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn begin_session(
        &self,
        planned_seconds: i64,
        started_at_ms: i64,
    ) -> Result<String, String> {
        let id = Uuid::new_v4().to_string();
        self.connection
            .lock()
            .map_err(|error| error.to_string())?
            .execute(
                "INSERT INTO focus_sessions(id, started_at_ms, planned_seconds, outcome)
                 VALUES(?1, ?2, ?3, 'running')",
                params![id, started_at_ms, planned_seconds],
            )
            .map_err(|error| error.to_string())?;
        Ok(id)
    }

    pub fn add_segment(
        &self,
        session_id: &str,
        started_at_ms: i64,
        ended_at_ms: i64,
    ) -> Result<(), String> {
        if ended_at_ms <= started_at_ms {
            return Ok(());
        }
        self.connection
            .lock()
            .map_err(|error| error.to_string())?
            .execute(
                "INSERT INTO focus_segments(session_id, started_at_ms, ended_at_ms)
                 VALUES(?1, ?2, ?3)",
                params![session_id, started_at_ms, ended_at_ms],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn finish_session(
        &self,
        session_id: &str,
        ended_at_ms: i64,
        completed: bool,
        was_paused: bool,
    ) -> Result<(), String> {
        let outcome = match (completed, was_paused) {
            (true, true) => "completed_after_pause",
            (true, false) => "completed",
            (false, _) => "interrupted",
        };
        self.connection
            .lock()
            .map_err(|error| error.to_string())?
            .execute(
                "UPDATE focus_sessions SET ended_at_ms = ?1, outcome = ?2 WHERE id = ?3",
                params![ended_at_ms, outcome, session_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn finalize_running_sessions(&self, ended_at_ms: i64) -> Result<(), String> {
        self.connection
            .lock()
            .map_err(|error| error.to_string())?
            .execute(
                "UPDATE focus_sessions
                 SET ended_at_ms = ?1, outcome = 'interrupted'
                 WHERE outcome = 'running'",
                [ended_at_ms],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn records_between(
        &self,
        start_ms: i64,
        end_ms: i64,
    ) -> Result<(Vec<FocusSession>, Vec<FocusSegment>), String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let mut session_statement = connection
            .prepare(
                "SELECT s.id, s.started_at_ms, COALESCE(s.ended_at_ms, s.started_at_ms),
                        s.planned_seconds, s.outcome,
                        COALESCE(SUM(MAX(0, MIN(g.ended_at_ms, ?2) - MAX(g.started_at_ms, ?1))), 0) / 1000
                 FROM focus_sessions s
                 LEFT JOIN focus_segments g ON g.session_id = s.id
                   AND g.ended_at_ms > ?1 AND g.started_at_ms < ?2
                 WHERE g.id IS NOT NULL
                    OR (s.started_at_ms >= ?1 AND s.started_at_ms < ?2)
                 GROUP BY s.id
                 ORDER BY s.started_at_ms DESC",
            )
            .map_err(|error| error.to_string())?;
        let sessions = session_statement
            .query_map(params![start_ms, end_ms], |row| {
                Ok(FocusSession {
                    id: row.get(0)?,
                    started_at_ms: row.get(1)?,
                    ended_at_ms: row.get(2)?,
                    planned_seconds: row.get(3)?,
                    outcome: row.get(4)?,
                    active_seconds: row.get(5)?,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;

        let mut segment_statement = connection
            .prepare(
                "SELECT session_id, MAX(started_at_ms, ?1), MIN(ended_at_ms, ?2)
                 FROM focus_segments
                 WHERE ended_at_ms > ?1 AND started_at_ms < ?2
                 ORDER BY started_at_ms",
            )
            .map_err(|error| error.to_string())?;
        let segments = segment_statement
            .query_map(params![start_ms, end_ms], |row| {
                Ok(FocusSegment {
                    session_id: row.get(0)?,
                    started_at_ms: row.get(1)?,
                    ended_at_ms: row.get(2)?,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        Ok((sessions, segments))
    }

    pub fn delete_session(&self, id: &str) -> Result<(), String> {
        self.connection
            .lock()
            .map_err(|error| error.to_string())?
            .execute("DELETE FROM focus_sessions WHERE id = ?1", [id])
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn clear_history(&self) -> Result<(), String> {
        self.connection
            .lock()
            .map_err(|error| error.to_string())?
            .execute_batch("DELETE FROM focus_sessions; DELETE FROM analysis_reports;")
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn save_ai_analysis(
        &self,
        period: &ReportPeriod,
        analysis: &AiAnalysis,
    ) -> Result<(), String> {
        self.connection
            .lock()
            .map_err(|error| error.to_string())?
            .execute(
                "INSERT INTO analysis_reports(
                   id, period_start, period_end, data_hash, model, content, generated_at_ms
                 ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    Uuid::new_v4().to_string(),
                    period.start_date,
                    period.end_date,
                    analysis.data_hash,
                    analysis.model,
                    analysis.content,
                    analysis.generated_at_ms
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn latest_ai_analysis(&self, period: &ReportPeriod) -> Result<Option<AiAnalysis>, String> {
        self.connection
            .lock()
            .map_err(|error| error.to_string())?
            .query_row(
                "SELECT content, model, generated_at_ms, data_hash
                 FROM analysis_reports
                 WHERE period_start = ?1 AND period_end = ?2
                 ORDER BY generated_at_ms DESC LIMIT 1",
                params![period.start_date, period.end_date],
                |row| {
                    Ok(AiAnalysis {
                        content: row.get(0)?,
                        model: row.get(1)?,
                        generated_at_ms: row.get(2)?,
                        data_hash: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_segments_are_summed_and_deleted() {
        let directory = tempfile::tempdir().unwrap();
        let database = Database::open(&directory.path().join("test.db")).unwrap();
        let version: i64 = database
            .connection
            .lock()
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1);
        let id = database.begin_session(1500, 1_000).unwrap();
        database.add_segment(&id, 1_000, 61_000).unwrap();
        database.add_segment(&id, 121_000, 181_000).unwrap();
        database.finish_session(&id, 181_000, true, true).unwrap();

        let (sessions, segments) = database.records_between(0, 200_000).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].active_seconds, 120);
        assert_eq!(sessions[0].outcome, "completed_after_pause");
        assert_eq!(segments.len(), 2);

        let interrupted = database.begin_session(1500, 190_000).unwrap();
        database
            .finish_session(&interrupted, 190_000, false, false)
            .unwrap();
        let (sessions, _) = database.records_between(0, 200_000).unwrap();
        assert_eq!(sessions.len(), 2);
        assert!(sessions.iter().any(|session| {
            session.id == interrupted
                && session.active_seconds == 0
                && session.outcome == "interrupted"
        }));

        database.delete_session(&id).unwrap();
        let (sessions, segments) = database.records_between(0, 200_000).unwrap();
        assert_eq!(sessions.len(), 1);
        assert!(segments.is_empty());
    }
}
