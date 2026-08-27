use std::time::Duration;

use chrono::Utc;
use keyring::Entry;
use reqwest::Url;
use serde::Deserialize;
use serde_json::json;

use crate::{
    database::Database,
    models::{AiAnalysis, AiConfig, AppSettings, ReportData, ReportPeriod},
};

const KEYRING_SERVICE: &str = "com.royluomac.focussquare";
const KEYRING_ACCOUNT: &str = "compatible-api-key";

pub fn save_api_key(value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err("API key must not be empty".into());
    }
    key_entry()?
        .set_password(value.trim())
        .map_err(|error| format!("Could not store API key: {error}"))
}

pub fn delete_api_key() -> Result<(), String> {
    match key_entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(format!("Could not delete API key: {error}")),
    }
}

pub fn config(settings: &AppSettings) -> AiConfig {
    AiConfig {
        base_url: settings.ai_base_url.clone(),
        model: settings.ai_model.clone(),
        has_api_key: read_api_key().is_ok(),
    }
}

pub async fn test_connection(settings: &AppSettings) -> Result<String, String> {
    request_text(settings, "Reply with OK only.").await
}

pub async fn generate(
    database: &Database,
    settings: &AppSettings,
    report: &ReportData,
) -> Result<AiAnalysis, String> {
    let prompt = report_prompt(report, &settings.locale)?;
    let content = request_text(settings, &prompt).await?;
    let analysis = AiAnalysis {
        content,
        model: settings.ai_model.clone(),
        generated_at_ms: Utc::now().timestamp_millis(),
        data_hash: report.data_hash.clone(),
    };
    database.save_ai_analysis(&report.period, &analysis)?;
    Ok(analysis)
}

async fn request_text(settings: &AppSettings, prompt: &str) -> Result<String, String> {
    request_text_with_timeout(settings, prompt, Duration::from_secs(60)).await
}

async fn request_text_with_timeout(
    settings: &AppSettings,
    prompt: &str,
    timeout: Duration,
) -> Result<String, String> {
    let endpoint = endpoint_url(&settings.ai_base_url)?;
    if settings.ai_model.trim().is_empty() {
        return Err("Configure a model before using AI analysis".into());
    }
    let local = is_local(&endpoint);
    let api_key = read_api_key().ok();
    if !local && api_key.is_none() {
        return Err("Configure an API key before using this endpoint".into());
    }

    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|error| error.to_string())?;
    let body = json!({
        "model": settings.ai_model,
        "messages": [
            {
                "role": "system",
                "content": "You are a focus-habit analyst. Use only the supplied aggregate metrics. Do not invent facts, diagnose health conditions, or claim access to raw sessions. Return plain text with: summary, strengths, risks, and exactly three actionable suggestions."
            },
            { "role": "user", "content": prompt }
        ],
        "stream": false
    });
    let mut request = client.post(endpoint).json(&body);
    if let Some(api_key) = api_key {
        request = request.bearer_auth(api_key);
    }
    let response = request.send().await.map_err(|error| {
        if error.is_timeout() {
            "AI request timed out after 60 seconds".to_string()
        } else {
            format!("AI request failed: {error}")
        }
    })?;
    let status = response.status();
    let text = response.text().await.map_err(|error| error.to_string())?;
    if !status.is_success() {
        let detail: String = text.chars().take(240).collect();
        return Err(format!("AI endpoint returned {status}: {detail}"));
    }
    let parsed: ChatResponse = serde_json::from_str(&text)
        .map_err(|_| "AI endpoint returned an unsupported response".to_string())?;
    let content = parsed
        .choices
        .into_iter()
        .next()
        .and_then(|choice| choice.message.content)
        .filter(|content| !content.trim().is_empty())
        .ok_or_else(|| "AI endpoint returned no text".to_string())?;
    Ok(content.trim().to_string())
}

fn report_prompt(report: &ReportData, locale: &str) -> Result<String, String> {
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct AggregatePayload<'a> {
        period: &'a ReportPeriod,
        previous_period: &'a ReportPeriod,
        metrics: &'a crate::models::ReportMetrics,
        previous_metrics: &'a crate::models::ReportMetrics,
        comparison: &'a crate::models::PeriodComparison,
        daily_totals: &'a [crate::models::DatePoint],
        hourly_totals: &'a [crate::models::HourPoint],
        local_insights: &'a [crate::models::HabitInsight],
        response_language: &'a str,
    }
    let payload = AggregatePayload {
        period: &report.period,
        previous_period: &report.previous_period,
        metrics: &report.metrics,
        previous_metrics: &report.previous_metrics,
        comparison: &report.comparison,
        daily_totals: &report.daily,
        hourly_totals: &report.hourly,
        local_insights: &report.insights,
        response_language: if locale.starts_with("zh") {
            "Chinese"
        } else {
            "English"
        },
    };
    serde_json::to_string_pretty(&payload).map_err(|error| error.to_string())
}

fn endpoint_url(base_url: &str) -> Result<Url, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    let value = if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/chat/completions")
    };
    let url = Url::parse(&value).map_err(|_| "Invalid AI service URL".to_string())?;
    let allowed = url.scheme() == "https" || (url.scheme() == "http" && is_local(&url));
    if !allowed {
        return Err("Use HTTPS, or HTTP only for localhost/127.0.0.1".into());
    }
    Ok(url)
}

fn is_local(url: &Url) -> bool {
    matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"))
}

fn key_entry() -> Result<Entry, String> {
    Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT).map_err(|error| error.to_string())
}

fn read_api_key() -> Result<String, String> {
    key_entry()?
        .get_password()
        .map_err(|error| error.to_string())
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct ChatMessage {
    content: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    use crate::models::{PeriodComparison, ReportMetrics};

    #[test]
    fn endpoint_accepts_https_and_local_http_only() {
        assert_eq!(
            endpoint_url("https://example.com/v1").unwrap().path(),
            "/v1/chat/completions"
        );
        assert!(endpoint_url("http://localhost:11434/v1").is_ok());
        assert!(endpoint_url("http://example.com/v1").is_err());
    }

    #[test]
    fn ai_payload_contains_aggregates_but_not_raw_sessions() {
        let report = ReportData {
            period: ReportPeriod {
                start_date: "2026-08-01".into(),
                end_date: "2026-08-07".into(),
            },
            previous_period: ReportPeriod {
                start_date: "2026-07-25".into(),
                end_date: "2026-07-31".into(),
            },
            metrics: ReportMetrics {
                total_focus_seconds: 3_600,
                ..Default::default()
            },
            previous_metrics: ReportMetrics::default(),
            comparison: PeriodComparison {
                focus_seconds_change_ratio: None,
                completion_rate_change: 0.0,
                active_days_change: 0,
            },
            daily: vec![],
            hourly: vec![],
            insights: vec![],
            sessions: vec![],
            data_hash: "hash".into(),
            ai_analysis: None,
            ai_analysis_stale: false,
        };
        let payload = report_prompt(&report, "zh-CN").unwrap();
        assert!(payload.contains("totalFocusSeconds"));
        assert!(!payload.contains("sessions"));
        assert!(!payload.contains("startedAtMs"));
        assert!(!payload.contains("dataHash"));
    }

    #[tokio::test]
    async fn ai_request_reports_unauthorized_and_invalid_responses() {
        let mut settings = AppSettings::default();
        settings.ai_model = "test-model".into();

        settings.ai_base_url = mock_server("401 Unauthorized", "denied", Duration::ZERO);
        let unauthorized = request_text(&settings, "test").await.unwrap_err();
        assert!(unauthorized.contains("401"));

        settings.ai_base_url = mock_server("200 OK", "{}", Duration::ZERO);
        let invalid = request_text(&settings, "test").await.unwrap_err();
        assert!(invalid.contains("unsupported response"));
    }

    #[tokio::test]
    async fn ai_request_times_out_without_retrying() {
        let mut settings = AppSettings::default();
        settings.ai_model = "test-model".into();
        settings.ai_base_url = mock_server("200 OK", "{}", Duration::from_millis(150));
        let error = request_text_with_timeout(&settings, "test", Duration::from_millis(20))
            .await
            .unwrap_err();
        assert!(error.contains("timed out"));
    }

    fn mock_server(status: &'static str, body: &'static str, delay: Duration) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 8_192];
            let _ = stream.read(&mut request);
            thread::sleep(delay);
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        });
        format!("http://{address}/v1")
    }
}
