import { invoke } from "@tauri-apps/api/core";
import { resolveLocale, translator, type Locale, type Translator } from "./i18n";
import type { AiAnalysis, AiConfig, AppSettings, ReportData, ReportPeriod, TimerSnapshot } from "./types";

type RangeKind = "today" | "week" | "custom";

let timer: TimerSnapshot;
let report: ReportData;
let aiConfig: AiConfig;
let rangeKind: RangeKind = "week";
let period: ReportPeriod;
let busy = "";
let message = "";

function e(value: unknown): string {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function text(t: Translator, key: Parameters<Translator>[0]): string {
  return String(t(key));
}

function dateValue(date: Date): string {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function todayPeriod(): ReportPeriod {
  const today = dateValue(new Date());
  return { startDate: today, endDate: today };
}

function weekPeriod(): ReportPeriod {
  const end = new Date();
  const start = new Date(end);
  start.setDate(end.getDate() - ((end.getDay() + 6) % 7));
  return { startDate: dateValue(start), endDate: dateValue(end) };
}

function periodDays(value: ReportPeriod): number {
  const start = Date.parse(`${value.startDate}T00:00:00Z`);
  const end = Date.parse(`${value.endDate}T00:00:00Z`);
  return Math.max(1, Math.round((end - start) / 86_400_000) + 1);
}

function formatDuration(seconds: number, locale: Locale): string {
  const roundedMinutes = Math.round(seconds / 60);
  const hours = Math.floor(roundedMinutes / 60);
  const minutes = roundedMinutes % 60;
  if (locale === "zh-CN") {
    if (hours === 0) return `${minutes} 分钟`;
    return minutes ? `${hours} 小时 ${minutes} 分` : `${hours} 小时`;
  }
  if (hours === 0) return `${minutes} min`;
  return minutes ? `${hours}h ${minutes}m` : `${hours}h`;
}

function signedPercent(value: number | null, digits = 0): string {
  if (value === null) return "—";
  const percentage = value * 100;
  return `${percentage > 0 ? "+" : ""}${percentage.toFixed(digits)}%`;
}

function signedNumber(value: number): string {
  return `${value > 0 ? "+" : ""}${value}`;
}

function render(): void {
  const root = document.querySelector<HTMLDivElement>("#app")!;
  const locale = resolveLocale(timer.settings);
  const t = translator(locale);
  document.documentElement.lang = locale;
  root.className = "analytics-app";
  const noPrevious = report.previousMetrics.totalFocusSeconds === 0;

  root.innerHTML = `
    <div class="analytics-layout">
      <aside class="report-sidebar">
        <div class="brand-mark"><span></span><strong>Focus Square</strong></div>
        <div class="report-intro">
          <p class="eyebrow">LOCAL · PRIVATE</p>
          <h1>${text(t, "reportTitle")}</h1>
          <p>${text(t, "reportSubtitle")}</p>
        </div>
        <nav class="range-tabs" aria-label="Report range">
          ${rangeButton("today", text(t, "today"))}
          ${rangeButton("week", text(t, "thisWeek"))}
          ${rangeButton("custom", text(t, "custom"))}
        </nav>
        <form class="period-form ${rangeKind === "custom" ? "is-custom" : ""}" id="period-form">
          <label><span>Start</span><input id="start-date" type="date" value="${period.startDate}" ${rangeKind === "custom" ? "" : "disabled"}></label>
          <label><span>End</span><input id="end-date" type="date" value="${period.endDate}" ${rangeKind === "custom" ? "" : "disabled"}></label>
          ${rangeKind === "custom" ? `<button class="secondary" type="submit">${text(t, "apply")}</button>` : ""}
        </form>
        <div class="privacy-note">
          <span class="privacy-dot"></span>
          <p>${locale === "zh-CN" ? "专注记录仅保存在本机" : "Focus records stay on this device"}</p>
        </div>
      </aside>

      <main class="report-main">
        <header class="report-toolbar">
          <div><span>${e(report.period.startDate)}</span><i>—</i><span>${e(report.period.endDate)}</span></div>
          <button class="ghost-button" id="refresh" ${busy ? "disabled" : ""}>${busy ? text(t, "working") : "↻"}</button>
        </header>
        ${message ? `<div class="report-message" role="status">${e(message)}</div>` : ""}

        <section class="metric-grid">
          ${metricCard(text(t, "totalFocus"), formatDuration(report.metrics.totalFocusSeconds, locale), "accent")}
          ${metricCard(text(t, "dailyAverage"), formatDuration(report.metrics.totalFocusSeconds / periodDays(report.period), locale))}
          ${metricCard(text(t, "activeDays"), `${report.metrics.activeDays} ${text(t, "days")}`)}
          ${metricCard(text(t, "completion"), `${report.metrics.completedSessions} / ${report.metrics.attemptedSessions}`)}
          ${metricCard(text(t, "completionRate"), `${Math.round(report.metrics.completionRate * 100)}%`)}
          ${metricCard(text(t, "averageSession"), formatDuration(report.metrics.averageSessionSeconds, locale))}
        </section>

        ${report.metrics.attemptedSessions === 0 ? `<section class="empty-card"><span>◷</span><p>${text(t, "noData")}</p></section>` : renderReportBody(t, locale, noPrevious)}

        <section class="panel history-panel">
          <div class="panel-heading">
            <div><p class="eyebrow">LOG</p><h2>${text(t, "history")}</h2></div>
            <button class="danger-link" id="clear-history" ${report.sessions.length === 0 || busy ? "disabled" : ""}>${text(t, "clearHistory")}</button>
          </div>
          <div class="history-list">${renderHistory(t, locale)}</div>
        </section>

        <section class="panel ai-panel">
          <div class="panel-heading">
            <div><p class="eyebrow">OPT-IN</p><h2>${text(t, "aiTitle")}</h2></div>
            <span class="key-state ${aiConfig.hasApiKey ? "ready" : ""}">${aiConfig.hasApiKey ? text(t, "keyStored") : text(t, "keyMissing")}</span>
          </div>
          <p class="ai-privacy">${text(t, "aiPrivacy")}</p>
          <form id="ai-config-form" class="ai-config-form">
            <label>${text(t, "serviceUrl")}<input name="baseUrl" type="url" value="${e(timer.settings.aiBaseUrl)}" placeholder="https://api.openai.com/v1"></label>
            <label>${text(t, "model")}<input name="model" value="${e(timer.settings.aiModel)}" placeholder="model-name"></label>
            <button class="secondary" type="submit" ${busy ? "disabled" : ""}>${text(t, "saveConfig")}</button>
          </form>
          <div class="key-form">
            <label>${text(t, "apiKey")}<input id="api-key" type="password" autocomplete="off" placeholder="••••••••••••"></label>
            <button class="secondary" id="save-key" ${busy ? "disabled" : ""}>${text(t, "saveKey")}</button>
            ${aiConfig.hasApiKey ? `<button class="ghost-button text-button" id="remove-key" ${busy ? "disabled" : ""}>${text(t, "removeKey")}</button>` : ""}
          </div>
          <div class="ai-actions">
            <button class="secondary" id="test-ai" ${busy ? "disabled" : ""}>${busy === "test" ? text(t, "working") : text(t, "testConnection")}</button>
            <button class="primary" id="generate-ai" ${busy ? "disabled" : ""}>${busy === "generate" ? text(t, "working") : text(t, "generateAi")}</button>
          </div>
          <div class="ai-result ${report.aiAnalysis ? "has-result" : ""}">
            <div class="ai-result-heading"><strong>${text(t, "generatedAi")}</strong>${report.aiAnalysisStale ? `<span>${text(t, "stale")}</span>` : ""}</div>
            <pre id="ai-content"></pre>
          </div>
        </section>
      </main>
    </div>`;

  const aiContent = document.querySelector<HTMLElement>("#ai-content")!;
  aiContent.textContent = report.aiAnalysis || text(t, "aiEmpty");
  bindEvents(t);
}

function rangeButton(kind: RangeKind, label: string): string {
  return `<button data-range="${kind}" class="${rangeKind === kind ? "active" : ""}">${label}</button>`;
}

function metricCard(label: string, value: string, className = ""): string {
  return `<article class="metric-card ${className}"><span>${e(label)}</span><strong>${e(value)}</strong></article>`;
}

function renderReportBody(t: Translator, locale: Locale, noPrevious: boolean): string {
  const dailyMax = Math.max(1, ...report.daily.map((point) => point.focusSeconds));
  const hourlyMax = Math.max(1, ...report.hourly.map((point) => point.focusSeconds));
  const peak = report.metrics.peakHour;
  const focusChange = report.comparison.focusSecondsChangeRatio;
  return `
    <div class="report-columns">
      <section class="panel daily-panel">
        <div class="panel-heading"><div><p class="eyebrow">RHYTHM</p><h2>${text(t, "dailyTrend")}</h2></div><strong>${formatDuration(report.metrics.totalFocusSeconds, locale)}</strong></div>
        <div class="daily-chart">${report.daily.map((point) => `
          <div class="bar-column" title="${e(point.date)} · ${e(formatDuration(point.focusSeconds, locale))}">
            <span style="height:${Math.max(point.focusSeconds ? 4 : 0, point.focusSeconds / dailyMax * 100)}%"></span>
            <small>${e(point.date.slice(5).replace("-", "/"))}</small>
          </div>`).join("")}</div>
      </section>
      <section class="panel comparison-panel">
        <div class="panel-heading"><div><p class="eyebrow">DELTA</p><h2>${text(t, "comparison")}</h2></div></div>
        ${noPrevious ? `<p class="muted-copy">${text(t, "noPrevious")}</p>` : `
          <div class="comparison-list">
            ${comparisonRow(text(t, "timeChange"), signedPercent(focusChange), focusChange || 0)}
            ${comparisonRow(text(t, "rateChange"), signedPercent(report.comparison.completionRateChange, 1), report.comparison.completionRateChange)}
            ${comparisonRow(text(t, "continuityChange"), signedNumber(report.comparison.activeDaysChange), report.comparison.activeDaysChange)}
          </div>`}
      </section>
    </div>
    <section class="panel hourly-panel">
      <div class="panel-heading"><div><p class="eyebrow">WINDOW</p><h2>${text(t, "hourlyTrend")}</h2></div><strong>${text(t, "peakTime")} · ${peak === null ? "—" : `${String(peak).padStart(2, "0")}:00–${String((peak + 1) % 24).padStart(2, "0")}:00`}</strong></div>
      <div class="hourly-chart">${report.hourly.map((point) => `<span class="${point.hour === peak ? "peak" : ""}" style="height:${Math.max(point.focusSeconds ? 3 : 0, point.focusSeconds / hourlyMax * 100)}%" title="${point.hour}:00 · ${e(formatDuration(point.focusSeconds, locale))}"></span>`).join("")}</div>
      <div class="hour-labels"><span>00</span><span>06</span><span>12</span><span>18</span><span>24</span></div>
    </section>
    <section class="panel insights-panel">
      <div class="panel-heading"><div><p class="eyebrow">PATTERNS</p><h2>${text(t, "insights")}</h2></div><strong>${text(t, "streak")} · ${report.metrics.longestStreakDays} ${text(t, "days")}</strong></div>
      <div class="insight-grid">${report.insights.map((item) => `<article class="insight ${e(item.kind)}"><i></i><div><strong>${e(item.title)}</strong><p>${e(item.detail)}</p></div></article>`).join("")}</div>
    </section>`;
}

function comparisonRow(label: string, value: string, direction: number): string {
  const state = direction > 0 ? "positive" : direction < 0 ? "negative" : "neutral";
  return `<div><span>${e(label)}</span><strong class="${state}">${e(value)}</strong></div>`;
}

function renderHistory(t: Translator, locale: Locale): string {
  if (report.sessions.length === 0) return `<p class="muted-copy">${text(t, "noData")}</p>`;
  const dateLocale = locale === "zh-CN" ? "zh-CN" : "en-US";
  return report.sessions.map((session) => {
    const status = session.outcome === "completed"
      ? text(t, "normalComplete")
      : session.outcome === "completed_after_pause"
        ? text(t, "pausedComplete")
        : session.outcome === "running"
          ? text(t, "runningSession")
          : text(t, "interrupted");
    const date = new Intl.DateTimeFormat(dateLocale, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }).format(session.startedAtMs);
    return `<article class="history-row">
      <span class="outcome-dot ${e(session.outcome)}"></span>
      <div><strong>${e(status)}</strong><small>${e(date)}</small></div>
      <span>${e(formatDuration(session.activeSeconds, locale))}</span>
      <button data-delete="${e(session.id)}" title="${text(t, "delete")}">×</button>
    </article>`;
  }).join("");
}

function bindEvents(t: Translator): void {
  document.querySelectorAll<HTMLButtonElement>("[data-range]").forEach((button) => {
    button.addEventListener("click", () => void changeRange(button.dataset.range as RangeKind));
  });
  document.querySelector<HTMLFormElement>("#period-form")!.addEventListener("submit", (event) => {
    event.preventDefault();
    period = {
      startDate: document.querySelector<HTMLInputElement>("#start-date")!.value,
      endDate: document.querySelector<HTMLInputElement>("#end-date")!.value,
    };
    void loadReport();
  });
  bind("refresh", loadReport);
  document.querySelectorAll<HTMLButtonElement>("[data-delete]").forEach((button) => {
    button.addEventListener("click", async () => {
      if (!window.confirm(text(t, "deleteConfirm"))) return;
      await action("delete", () => invoke("delete_focus_session", { id: button.dataset.delete }));
      await loadReport(false);
    });
  });
  bind("clear-history", async () => {
    if (!window.confirm(text(t, "clearConfirm"))) return;
    await action("clear", () => invoke("clear_focus_sessions"));
    await loadReport(false);
  });
  document.querySelector<HTMLFormElement>("#ai-config-form")!.addEventListener("submit", (event) => {
    event.preventDefault();
    void saveAiConfig();
  });
  bind("save-key", async () => {
    const apiKey = document.querySelector<HTMLInputElement>("#api-key")!.value;
    await action("key", async () => {
      aiConfig = await invoke<AiConfig>("save_ai_key", { apiKey });
    });
  });
  bind("remove-key", async () => {
    await action("key", async () => {
      aiConfig = await invoke<AiConfig>("delete_ai_key");
    });
  });
  bind("test-ai", async () => {
    if (!(await saveAiConfig(false))) return;
    await action("test", async () => {
      await invoke<string>("test_ai_connection");
      message = text(t, "connectionOk");
    });
  });
  bind("generate-ai", async () => {
    if (!(await saveAiConfig(false))) return;
    await action("generate", async () => {
      const analysis = await invoke<AiAnalysis>("generate_ai_analysis", {
        period,
        locale: resolveLocale(timer.settings),
      });
      report.aiAnalysis = analysis.content;
      report.aiAnalysisStale = false;
    });
  });
}

function bind(id: string, callback: () => void | Promise<void>): void {
  document.querySelector(`#${id}`)?.addEventListener("click", () => void callback());
}

async function changeRange(kind: RangeKind): Promise<void> {
  rangeKind = kind;
  if (kind === "today") period = todayPeriod();
  if (kind === "week") period = weekPeriod();
  if (kind !== "custom") await loadReport();
  else render();
}

async function loadReport(showBusy = true): Promise<void> {
  if (showBusy) busy = "report";
  message = "";
  render();
  try {
    report = await invoke<ReportData>("build_report", {
      period,
      locale: resolveLocale(timer.settings),
    });
  } catch (error) {
    message = String(error);
  } finally {
    busy = "";
    render();
  }
}

async function saveAiConfig(showFeedback = true): Promise<boolean> {
  const form = document.querySelector<HTMLFormElement>("#ai-config-form")!;
  const data = new FormData(form);
  const settings: AppSettings = {
    ...timer.settings,
    aiBaseUrl: String(data.get("baseUrl") || "").trim(),
    aiModel: String(data.get("model") || "").trim(),
  };
  try {
    timer = await invoke<TimerSnapshot>("update_settings", { settings });
    aiConfig = await invoke<AiConfig>("get_ai_config");
    if (showFeedback) message = String(translator(resolveLocale(timer.settings))("saved"));
    if (showFeedback) render();
    return true;
  } catch (error) {
    message = String(error);
    render();
    return false;
  }
}

async function action(name: string, callback: () => Promise<unknown>): Promise<void> {
  busy = name;
  message = "";
  render();
  try {
    await callback();
  } catch (error) {
    message = String(error);
  } finally {
    busy = "";
    render();
  }
}

export async function mountAnalytics(): Promise<void> {
  period = weekPeriod();
  timer = await invoke<TimerSnapshot>("get_timer");
  [aiConfig, report] = await Promise.all([
    invoke<AiConfig>("get_ai_config"),
    invoke<ReportData>("build_report", {
      period,
      locale: resolveLocale(timer.settings),
    }),
  ]);
  render();
}
