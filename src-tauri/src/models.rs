use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TimerMode {
    Focus,
    ShortBreak,
    LongBreak,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TimerStatus {
    Idle,
    Running,
    Paused,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TimerSettings {
    pub focus_minutes: u32,
    pub short_break_minutes: u32,
    pub long_break_minutes: u32,
    pub sessions_per_cycle: u32,
}

impl Default for TimerSettings {
    fn default() -> Self {
        Self {
            focus_minutes: 25,
            short_break_minutes: 5,
            long_break_minutes: 15,
            sessions_per_cycle: 4,
        }
    }
}

impl TimerSettings {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=180).contains(&self.focus_minutes) {
            return Err("Focus duration must be between 1 and 180 minutes".into());
        }
        if !(1..=60).contains(&self.short_break_minutes)
            || !(1..=60).contains(&self.long_break_minutes)
        {
            return Err("Break durations must be between 1 and 60 minutes".into());
        }
        if !(1..=12).contains(&self.sessions_per_cycle) {
            return Err("Sessions per cycle must be between 1 and 12".into());
        }
        Ok(())
    }

    pub fn duration_ms(&self, mode: TimerMode) -> u64 {
        let minutes = match mode {
            TimerMode::Focus => self.focus_minutes,
            TimerMode::ShortBreak => self.short_break_minutes,
            TimerMode::LongBreak => self.long_break_minutes,
        };
        u64::from(minutes) * 60_000
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub timer: TimerSettings,
    pub locale: String,
    pub always_on_top: bool,
    pub sound_enabled: bool,
    pub notifications_enabled: bool,
    #[serde(default)]
    pub window_x: Option<i32>,
    #[serde(default)]
    pub window_y: Option<i32>,
    pub ai_base_url: String,
    pub ai_model: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            timer: TimerSettings::default(),
            locale: "system".into(),
            always_on_top: false,
            sound_enabled: true,
            notifications_enabled: true,
            window_x: None,
            window_y: None,
            ai_base_url: "https://api.openai.com/v1".into(),
            ai_model: String::new(),
        }
    }
}

impl AppSettings {
    pub fn validate(&self) -> Result<(), String> {
        self.timer.validate()?;
        if !matches!(self.locale.as_str(), "system" | "zh-CN" | "en") {
            return Err("Unsupported locale".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TimerSnapshot {
    pub mode: TimerMode,
    pub status: TimerStatus,
    pub remaining_ms: u64,
    pub total_ms: u64,
    pub end_at_epoch_ms: Option<i64>,
    pub completed_focuses: u32,
    pub settings: AppSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FocusSession {
    pub id: String,
    pub started_at_ms: i64,
    pub ended_at_ms: i64,
    pub planned_seconds: i64,
    pub active_seconds: i64,
    pub outcome: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FocusSegment {
    pub session_id: String,
    pub started_at_ms: i64,
    pub ended_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportPeriod {
    pub start_date: String,
    pub end_date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatePoint {
    pub date: String,
    pub focus_seconds: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HourPoint {
    pub hour: u32,
    pub focus_seconds: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReportMetrics {
    pub total_focus_seconds: i64,
    pub active_days: u32,
    pub attempted_sessions: u32,
    pub completed_sessions: u32,
    pub completion_rate: f64,
    pub average_session_seconds: i64,
    pub longest_streak_days: u32,
    pub peak_hour: Option<u32>,
    pub late_night_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeriodComparison {
    pub focus_seconds_change_ratio: Option<f64>,
    pub completion_rate_change: f64,
    pub active_days_change: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HabitInsight {
    pub kind: String,
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportData {
    pub period: ReportPeriod,
    pub previous_period: ReportPeriod,
    pub metrics: ReportMetrics,
    pub previous_metrics: ReportMetrics,
    pub comparison: PeriodComparison,
    pub daily: Vec<DatePoint>,
    pub hourly: Vec<HourPoint>,
    pub insights: Vec<HabitInsight>,
    pub sessions: Vec<FocusSession>,
    pub data_hash: String,
    pub ai_analysis: Option<String>,
    pub ai_analysis_stale: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiConfig {
    pub base_url: String,
    pub model: String,
    pub has_api_key: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiAnalysis {
    pub content: String,
    pub model: String,
    pub generated_at_ms: i64,
    pub data_hash: String,
}
