use crate::{
    database::Database,
    models::{AppSettings, TimerMode, TimerSnapshot, TimerStatus},
};

#[derive(Debug, Clone, Copy)]
pub struct Completion {
    pub finished_mode: TimerMode,
    pub next_mode: TimerMode,
}

pub struct TimerEngine {
    mode: TimerMode,
    status: TimerStatus,
    remaining_ms: u64,
    total_ms: u64,
    end_at_epoch_ms: Option<i64>,
    completed_focuses: u32,
    settings: AppSettings,
    active_session_id: Option<String>,
    active_segment_started_at_ms: Option<i64>,
    focus_was_paused: bool,
    generation: u64,
}

impl TimerEngine {
    pub fn new(settings: AppSettings) -> Self {
        let total_ms = settings.timer.duration_ms(TimerMode::Focus);
        Self {
            mode: TimerMode::Focus,
            status: TimerStatus::Idle,
            remaining_ms: total_ms,
            total_ms,
            end_at_epoch_ms: None,
            completed_focuses: 0,
            settings,
            active_session_id: None,
            active_segment_started_at_ms: None,
            focus_was_paused: false,
            generation: 0,
        }
    }

    pub fn snapshot(&self, now_ms: i64) -> TimerSnapshot {
        let remaining_ms = if self.status == TimerStatus::Running {
            self.end_at_epoch_ms
                .map(|end| end.saturating_sub(now_ms).max(0) as u64)
                .unwrap_or(self.remaining_ms)
        } else {
            self.remaining_ms
        };
        TimerSnapshot {
            mode: self.mode,
            status: self.status,
            remaining_ms,
            total_ms: self.total_ms,
            end_at_epoch_ms: self.end_at_epoch_ms,
            completed_focuses: self.completed_focuses,
            settings: self.settings.clone(),
        }
    }

    pub fn start(&mut self, database: &Database, now_ms: i64) -> Result<u64, String> {
        if self.status == TimerStatus::Running {
            return Ok(self.generation);
        }
        if self.status == TimerStatus::Completed {
            return Err("Choose the next stage before starting".into());
        }
        if self.mode == TimerMode::Focus {
            if self.active_session_id.is_none() {
                self.active_session_id =
                    Some(database.begin_session((self.total_ms / 1000) as i64, now_ms)?);
                self.focus_was_paused = false;
            }
            self.active_segment_started_at_ms = Some(now_ms);
        }
        self.end_at_epoch_ms = Some(now_ms.saturating_add(self.remaining_ms as i64));
        self.status = TimerStatus::Running;
        self.bump_generation();
        Ok(self.generation)
    }

    pub fn pause(&mut self, database: &Database, now_ms: i64) -> Result<(), String> {
        if self.status != TimerStatus::Running {
            return Ok(());
        }
        self.remaining_ms = self
            .end_at_epoch_ms
            .map(|end| end.saturating_sub(now_ms).max(0) as u64)
            .unwrap_or(self.remaining_ms);
        self.close_active_segment(database, now_ms)?;
        if self.mode == TimerMode::Focus && self.active_session_id.is_some() {
            self.focus_was_paused = true;
        }
        self.end_at_epoch_ms = None;
        self.status = TimerStatus::Paused;
        self.bump_generation();
        Ok(())
    }

    pub fn reset(&mut self, database: &Database, now_ms: i64) -> Result<(), String> {
        self.interrupt_session(database, now_ms)?;
        self.status = TimerStatus::Idle;
        self.end_at_epoch_ms = None;
        self.total_ms = self.settings.timer.duration_ms(self.mode);
        self.remaining_ms = self.total_ms;
        self.bump_generation();
        Ok(())
    }

    pub fn advance(&mut self, database: &Database, now_ms: i64) -> Result<u64, String> {
        if self.status != TimerStatus::Completed {
            return Err("The current stage has not completed".into());
        }
        self.mode = self.next_mode();
        self.status = TimerStatus::Idle;
        self.total_ms = self.settings.timer.duration_ms(self.mode);
        self.remaining_ms = self.total_ms;
        self.end_at_epoch_ms = None;
        self.start(database, now_ms)
    }

    pub fn defer(&mut self) {
        if self.status == TimerStatus::Completed {
            self.mode = self.next_mode();
            self.status = TimerStatus::Idle;
            self.total_ms = self.settings.timer.duration_ms(self.mode);
            self.remaining_ms = self.total_ms;
            self.end_at_epoch_ms = None;
            self.bump_generation();
        }
    }

    pub fn update_settings(&mut self, settings: AppSettings) -> Result<(), String> {
        settings.validate()?;
        self.settings = settings;
        if self.status == TimerStatus::Idle {
            self.total_ms = self.settings.timer.duration_ms(self.mode);
            self.remaining_ms = self.total_ms;
        }
        self.bump_generation();
        Ok(())
    }

    pub fn complete_if_due(
        &mut self,
        database: &Database,
        now_ms: i64,
        expected_generation: u64,
    ) -> Result<Option<Completion>, String> {
        if self.generation != expected_generation || self.status != TimerStatus::Running {
            return Ok(None);
        }
        let Some(end_at) = self.end_at_epoch_ms else {
            return Ok(None);
        };
        if now_ms < end_at {
            return Ok(None);
        }
        self.close_active_segment(database, end_at)?;
        if self.mode == TimerMode::Focus {
            if let Some(session_id) = self.active_session_id.take() {
                database.finish_session(&session_id, end_at, true, self.focus_was_paused)?;
            }
            self.focus_was_paused = false;
            self.completed_focuses = self.completed_focuses.saturating_add(1);
        }
        self.remaining_ms = 0;
        self.end_at_epoch_ms = None;
        self.status = TimerStatus::Completed;
        let completion = Completion {
            finished_mode: self.mode,
            next_mode: self.next_mode(),
        };
        self.bump_generation();
        Ok(Some(completion))
    }

    pub fn finalize_on_exit(&mut self, database: &Database, now_ms: i64) -> Result<(), String> {
        self.interrupt_session(database, now_ms)
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn end_at_epoch_ms(&self) -> Option<i64> {
        self.end_at_epoch_ms
    }

    pub fn settings(&self) -> &AppSettings {
        &self.settings
    }

    pub fn set_window_position(&mut self, x: i32, y: i32) {
        self.settings.window_x = Some(x);
        self.settings.window_y = Some(y);
    }

    pub fn active_session_id(&self) -> Option<&str> {
        self.active_session_id.as_deref()
    }

    fn next_mode(&self) -> TimerMode {
        match self.mode {
            TimerMode::Focus => {
                if self.completed_focuses > 0
                    && self.completed_focuses % self.settings.timer.sessions_per_cycle == 0
                {
                    TimerMode::LongBreak
                } else {
                    TimerMode::ShortBreak
                }
            }
            TimerMode::ShortBreak | TimerMode::LongBreak => TimerMode::Focus,
        }
    }

    fn close_active_segment(
        &mut self,
        database: &Database,
        ended_at_ms: i64,
    ) -> Result<(), String> {
        if let (Some(session_id), Some(started_at_ms)) = (
            self.active_session_id.as_deref(),
            self.active_segment_started_at_ms.take(),
        ) {
            database.add_segment(session_id, started_at_ms, ended_at_ms)?;
        }
        Ok(())
    }

    fn interrupt_session(&mut self, database: &Database, now_ms: i64) -> Result<(), String> {
        self.close_active_segment(database, now_ms)?;
        if let Some(session_id) = self.active_session_id.take() {
            database.finish_session(&session_id, now_ms, false, self.focus_was_paused)?;
        }
        self.focus_was_paused = false;
        Ok(())
    }

    fn bump_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paused_time_is_not_recorded_and_fourth_focus_uses_long_break() {
        let directory = tempfile::tempdir().unwrap();
        let database = Database::open(&directory.path().join("test.db")).unwrap();
        let mut timer = TimerEngine::new(AppSettings::default());
        timer.total_ms = 1_000;
        timer.remaining_ms = 1_000;

        timer.start(&database, 0).unwrap();
        timer.pause(&database, 400).unwrap();
        assert_eq!(timer.snapshot(5_000).remaining_ms, 600);
        let generation = timer.start(&database, 10_000).unwrap();
        let completion = timer
            .complete_if_due(&database, 10_600, generation)
            .unwrap()
            .unwrap();
        assert_eq!(completion.finished_mode, TimerMode::Focus);

        let (sessions, _) = database.records_between(0, 20_000).unwrap();
        assert_eq!(sessions[0].active_seconds, 1);
        assert_eq!(sessions[0].outcome, "completed_after_pause");

        timer.settings.timer.short_break_minutes = 1;
        let break_generation = timer.advance(&database, 20_000).unwrap();
        assert_eq!(timer.snapshot(20_000).mode, TimerMode::ShortBreak);
        let break_completion = timer
            .complete_if_due(&database, 80_000, break_generation)
            .unwrap()
            .unwrap();
        assert_eq!(break_completion.next_mode, TimerMode::Focus);
        timer.advance(&database, 80_000).unwrap();
        assert_eq!(timer.snapshot(80_000).mode, TimerMode::Focus);

        timer.completed_focuses = 4;
        assert_eq!(timer.next_mode(), TimerMode::LongBreak);
        assert_ne!(generation, 0);
    }
}
