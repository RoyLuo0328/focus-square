use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Local, NaiveDate, TimeZone, Timelike, Utc};

use crate::{
    database::Database,
    models::{
        DatePoint, FocusSegment, FocusSession, HabitInsight, HourPoint, PeriodComparison,
        ReportData, ReportMetrics, ReportPeriod,
    },
};

struct Summary {
    metrics: ReportMetrics,
    daily: Vec<DatePoint>,
    hourly: Vec<HourPoint>,
}

pub fn build_report(
    database: &Database,
    period: ReportPeriod,
    locale: &str,
) -> Result<ReportData, String> {
    let (start_date, end_date, start_ms, end_ms) = period_bounds(&period)?;
    let days = (end_date - start_date).num_days() + 1;
    let previous_end = start_date
        .pred_opt()
        .ok_or_else(|| "Invalid report period".to_string())?;
    let previous_start = previous_end - Duration::days(days - 1);
    let previous_period = ReportPeriod {
        start_date: previous_start.format("%Y-%m-%d").to_string(),
        end_date: previous_end.format("%Y-%m-%d").to_string(),
    };
    let previous_start_ms = local_midnight_ms(previous_start)?;
    let previous_end_ms = local_midnight_ms(
        previous_end
            .succ_opt()
            .ok_or_else(|| "Invalid report period".to_string())?,
    )?;

    let (sessions, segments) = database.records_between(start_ms, end_ms)?;
    let (previous_sessions, previous_segments) =
        database.records_between(previous_start_ms, previous_end_ms)?;
    let current = summarize(start_date, end_date, &sessions, &segments)?;
    let previous = summarize(
        previous_start,
        previous_end,
        &previous_sessions,
        &previous_segments,
    )?;
    let comparison = compare(&current.metrics, &previous.metrics);
    let insights = make_insights(&current, &comparison, locale);
    let data_hash = report_hash(&period, &current);
    let saved_ai = database.latest_ai_analysis(&period)?;
    let ai_analysis_stale = saved_ai
        .as_ref()
        .is_some_and(|analysis| analysis.data_hash != data_hash);
    let ai_analysis = saved_ai.map(|analysis| analysis.content);

    Ok(ReportData {
        period,
        previous_period,
        metrics: current.metrics,
        previous_metrics: previous.metrics,
        comparison,
        daily: current.daily,
        hourly: current.hourly,
        insights,
        sessions,
        data_hash,
        ai_analysis,
        ai_analysis_stale,
    })
}

fn period_bounds(period: &ReportPeriod) -> Result<(NaiveDate, NaiveDate, i64, i64), String> {
    let start = NaiveDate::parse_from_str(&period.start_date, "%Y-%m-%d")
        .map_err(|_| "Invalid start date".to_string())?;
    let end = NaiveDate::parse_from_str(&period.end_date, "%Y-%m-%d")
        .map_err(|_| "Invalid end date".to_string())?;
    if end < start {
        return Err("End date must not be before start date".into());
    }
    let next = end
        .succ_opt()
        .ok_or_else(|| "Invalid end date".to_string())?;
    Ok((
        start,
        end,
        local_midnight_ms(start)?,
        local_midnight_ms(next)?,
    ))
}

fn local_midnight_ms(date: NaiveDate) -> Result<i64, String> {
    let naive = date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| "Invalid local date".to_string())?;
    Local
        .from_local_datetime(&naive)
        .earliest()
        .or_else(|| Local.from_local_datetime(&naive).latest())
        .map(|value| value.timestamp_millis())
        .ok_or_else(|| "Local date does not exist".to_string())
}

fn summarize(
    start_date: NaiveDate,
    end_date: NaiveDate,
    sessions: &[FocusSession],
    segments: &[FocusSegment],
) -> Result<Summary, String> {
    let mut daily = BTreeMap::new();
    let mut date = start_date;
    while date <= end_date {
        daily.insert(date, 0_i64);
        date = date.succ_opt().ok_or_else(|| "Date overflow".to_string())?;
    }
    let mut hourly = [0_i64; 24];
    let mut total_ms = 0_i64;
    for segment in segments {
        let duration = (segment.ended_at_ms - segment.started_at_ms).max(0);
        total_ms += duration;
        split_segment_by_day(segment, &mut daily)?;
        split_segment_by_hour(segment, &mut hourly)?;
    }

    let attempted = sessions.len() as u32;
    let completed = sessions
        .iter()
        .filter(|session| session.outcome.starts_with("completed"))
        .count() as u32;
    let active_days = daily.values().filter(|seconds| **seconds > 0).count() as u32;
    let total_seconds = total_ms / 1000;
    let completion_rate = if attempted == 0 {
        0.0
    } else {
        f64::from(completed) / f64::from(attempted)
    };
    let average_session_seconds = if attempted == 0 {
        0
    } else {
        total_seconds / i64::from(attempted)
    };
    let peak_hour = hourly
        .iter()
        .enumerate()
        .max_by_key(|(_, seconds)| **seconds)
        .and_then(|(hour, seconds)| (*seconds > 0).then_some(hour as u32));
    let late_night_seconds: i64 = hourly[0..6].iter().sum();
    let late_night_ratio = if total_seconds == 0 {
        0.0
    } else {
        late_night_seconds as f64 / total_seconds as f64
    };

    Ok(Summary {
        metrics: ReportMetrics {
            total_focus_seconds: total_seconds,
            active_days,
            attempted_sessions: attempted,
            completed_sessions: completed,
            completion_rate,
            average_session_seconds,
            longest_streak_days: longest_streak(&daily),
            peak_hour,
            late_night_ratio,
        },
        daily: daily
            .into_iter()
            .map(|(date, focus_seconds)| DatePoint {
                date: date.format("%Y-%m-%d").to_string(),
                focus_seconds,
            })
            .collect(),
        hourly: hourly
            .into_iter()
            .enumerate()
            .map(|(hour, focus_seconds)| HourPoint {
                hour: hour as u32,
                focus_seconds,
            })
            .collect(),
    })
}

fn split_segment_by_day(
    segment: &FocusSegment,
    daily: &mut BTreeMap<NaiveDate, i64>,
) -> Result<(), String> {
    let mut cursor = segment.started_at_ms;
    while cursor < segment.ended_at_ms {
        let local = local_datetime(cursor)?;
        let date = local.date_naive();
        let next_date = date.succ_opt().ok_or_else(|| "Date overflow".to_string())?;
        let boundary = local_midnight_ms(next_date)?;
        let next = segment.ended_at_ms.min(boundary.max(cursor + 1));
        if let Some(seconds) = daily.get_mut(&date) {
            *seconds += (next - cursor) / 1000;
        }
        cursor = next;
    }
    Ok(())
}

fn split_segment_by_hour(segment: &FocusSegment, hourly: &mut [i64; 24]) -> Result<(), String> {
    let mut cursor = segment.started_at_ms;
    while cursor < segment.ended_at_ms {
        let local = local_datetime(cursor)?;
        let hour = local.hour() as usize;
        let next_naive = local
            .naive_local()
            .date()
            .and_hms_opt(local.hour(), 0, 0)
            .ok_or_else(|| "Invalid local hour".to_string())?
            + Duration::hours(1);
        let boundary = Local
            .from_local_datetime(&next_naive)
            .earliest()
            .or_else(|| Local.from_local_datetime(&next_naive).latest())
            .map(|value| value.timestamp_millis())
            .unwrap_or(cursor + 3_600_000)
            .max(cursor + 1);
        let next = segment.ended_at_ms.min(boundary);
        hourly[hour] += (next - cursor) / 1000;
        cursor = next;
    }
    Ok(())
}

fn local_datetime(timestamp_ms: i64) -> Result<DateTime<Local>, String> {
    let utc = DateTime::<Utc>::from_timestamp_millis(timestamp_ms)
        .ok_or_else(|| "Invalid timestamp".to_string())?;
    Ok(utc.with_timezone(&Local))
}

fn longest_streak(daily: &BTreeMap<NaiveDate, i64>) -> u32 {
    let mut longest = 0;
    let mut current = 0;
    for seconds in daily.values() {
        if *seconds > 0 {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

fn compare(current: &ReportMetrics, previous: &ReportMetrics) -> PeriodComparison {
    PeriodComparison {
        focus_seconds_change_ratio: (previous.total_focus_seconds > 0).then(|| {
            (current.total_focus_seconds - previous.total_focus_seconds) as f64
                / previous.total_focus_seconds as f64
        }),
        completion_rate_change: current.completion_rate - previous.completion_rate,
        active_days_change: current.active_days as i32 - previous.active_days as i32,
    }
}

fn make_insights(
    summary: &Summary,
    comparison: &PeriodComparison,
    locale: &str,
) -> Vec<HabitInsight> {
    let chinese = locale.starts_with("zh");
    let metrics = &summary.metrics;
    if metrics.attempted_sessions < 3 || metrics.active_days < 2 {
        return vec![insight(
            "info",
            if chinese {
                "数据仍在积累"
            } else {
                "More data needed"
            },
            if chinese {
                "根据本地记录，至少完成3次尝试并覆盖2个活跃日后，习惯建议会更可靠。"
            } else {
                "Based on local records, habit guidance becomes more useful after 3 attempts across 2 active days."
            },
        )];
    }

    let peak = metrics.peak_hour.unwrap_or(0);
    let mut insights = vec![insight(
        "pattern",
        if chinese {
            "常用专注时段"
        } else {
            "Typical focus time"
        },
        &if chinese {
            format!(
                "根据本地记录，你最常在 {peak:02}:00–{:02}:00 进入专注。",
                (peak + 1) % 24
            )
        } else {
            format!(
                "Based on local records, you focus most often around {peak:02}:00–{:02}:00.",
                (peak + 1) % 24
            )
        },
    )];

    if metrics.completion_rate < 0.6 {
        insights.push(insight(
            "suggestion",
            if chinese { "尝试缩短单轮时长" } else { "Try shorter sessions" },
            if chinese {
                "根据本地记录，未完成比例较高。可先把专注时长缩短5分钟，稳定后再逐步增加。"
            } else {
                "Based on local records, interruptions are frequent. Try reducing focus sessions by five minutes, then increase gradually."
            },
        ));
    }

    let active_values: Vec<f64> = summary
        .daily
        .iter()
        .filter(|point| point.focus_seconds > 0)
        .map(|point| point.focus_seconds as f64)
        .collect();
    if coefficient_of_variation(&active_values) > 0.6 {
        insights.push(insight(
            "suggestion",
            if chinese { "固定一个启动时段" } else { "Use a consistent start time" },
            if chinese {
                "根据本地记录，每日投入波动较大。先固定一个容易坚持的专注时段，比追求单日高时长更稳。"
            } else {
                "Based on local records, daily focus varies widely. A repeatable start time is more sustainable than occasional long days."
            },
        ));
    }
    if metrics.late_night_ratio > 0.35 {
        insights.push(insight(
            "suggestion",
            if chinese { "留意深夜专注比例" } else { "Review late-night focus" },
            if chinese {
                "根据本地记录，较多专注发生在凌晨。若这并非主动安排，可尝试把最重要的一轮提前。"
            } else {
                "Based on local records, much of your focus happens after midnight. If unintentional, move the most important session earlier."
            },
        ));
    }
    if comparison
        .focus_seconds_change_ratio
        .is_some_and(|change| change > 0.1)
    {
        insights.push(insight(
            "positive",
            if chinese { "专注投入正在增加" } else { "Focus time is growing" },
            if chinese {
                "根据本地记录，本周期专注时间较上一等长周期有所增加。保持当前节奏即可。"
            } else {
                "Based on local records, focus time increased versus the previous equal period. Keep the current rhythm."
            },
        ));
    }
    insights.truncate(4);
    insights
}

fn coefficient_of_variation(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    if mean == 0.0 {
        return 0.0;
    }
    let variance = values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / values.len() as f64;
    variance.sqrt() / mean
}

fn insight(kind: &str, title: &str, detail: &str) -> HabitInsight {
    HabitInsight {
        kind: kind.into(),
        title: title.into(),
        detail: detail.into(),
    }
}

fn report_hash(period: &ReportPeriod, summary: &Summary) -> String {
    let payload = serde_json::to_vec(&(period, &summary.metrics, &summary.daily, &summary.hourly))
        .unwrap_or_default();
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in payload {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insufficient_data_has_one_clear_message() {
        let summary = Summary {
            metrics: ReportMetrics {
                attempted_sessions: 2,
                active_days: 1,
                ..ReportMetrics::default()
            },
            daily: vec![],
            hourly: vec![],
        };
        let insights = make_insights(
            &summary,
            &PeriodComparison {
                focus_seconds_change_ratio: None,
                completion_rate_change: 0.0,
                active_days_change: 0,
            },
            "zh-CN",
        );
        assert_eq!(insights.len(), 1);
        assert!(insights[0].detail.contains("至少完成3次"));
    }

    #[test]
    fn comparison_uses_previous_total() {
        let current = ReportMetrics {
            total_focus_seconds: 150,
            ..Default::default()
        };
        let previous = ReportMetrics {
            total_focus_seconds: 100,
            ..Default::default()
        };
        let value = compare(&current, &previous)
            .focus_seconds_change_ratio
            .unwrap();
        assert!((value - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn segment_crossing_local_midnight_is_split_between_days() {
        let directory = tempfile::tempdir().unwrap();
        let database = Database::open(&directory.path().join("test.db")).unwrap();
        let first_day = Local::now().date_naive() - Duration::days(2);
        let second_day = first_day.succ_opt().unwrap();
        let midnight = local_midnight_ms(second_day).unwrap();
        let session_id = database.begin_session(1_200, midnight - 600_000).unwrap();
        database
            .add_segment(&session_id, midnight - 600_000, midnight + 600_000)
            .unwrap();
        database
            .finish_session(&session_id, midnight + 600_000, true, false)
            .unwrap();

        let value = build_report(
            &database,
            ReportPeriod {
                start_date: first_day.format("%Y-%m-%d").to_string(),
                end_date: second_day.format("%Y-%m-%d").to_string(),
            },
            "en",
        )
        .unwrap();
        assert_eq!(value.metrics.total_focus_seconds, 1_200);
        assert_eq!(value.daily[0].focus_seconds, 600);
        assert_eq!(value.daily[1].focus_seconds, 600);
    }
}
