import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { isPermissionGranted, requestPermission } from "@tauri-apps/plugin-notification";
import { modeLabel, resolveLocale, statusLabel, translator, type Translator } from "./i18n";
import type { AppSettings, TimerSnapshot } from "./types";

const circumference = 2 * Math.PI * 58;
let timer: TimerSnapshot;
let settingsOpen = false;
let busy = false;
let message = "";

function text(t: Translator, key: Parameters<Translator>[0]): string {
  return String(t(key));
}

function remainingMs(): number {
  if (timer.status === "running" && timer.endAtEpochMs) {
    return Math.max(0, timer.endAtEpochMs - Date.now());
  }
  return timer.remainingMs;
}

function formatTimer(milliseconds: number): string {
  const seconds = Math.ceil(milliseconds / 1000);
  const minutes = Math.floor(seconds / 60);
  return `${String(minutes).padStart(2, "0")}:${String(seconds % 60).padStart(2, "0")}`;
}

function phaseTitle(t: Translator): string {
  if (timer.mode !== "focus") return modeLabel(timer.mode, t);
  const total = timer.settings.timer.sessionsPerCycle;
  const current = (timer.completedFocuses % total) + 1;
  const cycle = t("cycle") as (current: number, total: number) => string;
  return cycle(current, total);
}

function icon(name: "play" | "pause" | "reset" | "settings" | "report"): string {
  const paths = {
    play: '<path d="M9 6.5v11l9-5.5z"/>',
    pause: '<path d="M8 6h3v12H8zm5 0h3v12h-3z"/>',
    reset: '<path d="M6.2 8.2A7 7 0 1 1 5 12h2a5 5 0 1 0 1-3l2 2H4V5z"/>',
    settings: '<path d="M19 13.5v-3l-2-.5-.6-1.4 1-1.8-2.2-2.1-1.7 1-1.5-.6-.5-2.1h-3L8 5.1l-1.5.6-1.7-1-2.2 2.1 1 1.8L3 10v3l2 .5.6 1.4-1 1.8 2.2 2.1 1.7-1 1.5.6.5 2.1h3l.5-2.1 1.5-.6 1.7 1 2.2-2.1-1-1.8zM12 15.5a3.5 3.5 0 1 1 0-7 3.5 3.5 0 0 1 0 7"/>',
    report: '<path d="M5 19V9h3v10zm5 0V5h3v14zm5 0v-7h3v7z"/>',
  };
  return `<svg viewBox="0 0 24 24" aria-hidden="true">${paths[name]}</svg>`;
}

function render(): void {
  const app = document.querySelector<HTMLDivElement>("#app")!;
  const locale = resolveLocale(timer.settings);
  const t = translator(locale);
  document.documentElement.lang = locale;
  app.className = "timer-app";

  if (timer.status === "completed") {
    const focusDone = timer.mode === "focus";
    app.innerHTML = `
      <main class="timer-shell completion-shell ${focusDone ? "focus-complete" : "break-complete"}">
        <div class="drag-zone" data-tauri-drag-region></div>
        <div class="completion-glow" aria-hidden="true"></div>
        <div class="completion-mark" aria-hidden="true">${focusDone ? "✓" : "↗"}</div>
        <p class="eyebrow">${focusDone ? text(t, "focusDone") : text(t, "breakDone")}</p>
        <h1>${focusDone ? text(t, "startBreak") : text(t, "startFocus")}</h1>
        <p class="completion-note">${focusDone ? text(t, "focusDoneNote") : text(t, "breakDoneNote")}</p>
        <div class="completion-actions">
          <button class="primary wide" id="advance" ${busy ? "disabled" : ""}>${focusDone ? text(t, "startBreak") : text(t, "startFocus")}</button>
          <button class="quiet" id="defer" ${busy ? "disabled" : ""}>${text(t, "later")}</button>
        </div>
        ${message ? `<p class="inline-message" role="status">${message}</p>` : ""}
      </main>`;
    bind("advance", () => command("advance_timer"));
    bind("defer", () => command("defer_timer"));
    return;
  }

  if (settingsOpen) {
    const value = timer.settings;
    app.innerHTML = `
      <main class="timer-shell settings-shell">
        <div class="drag-zone" data-tauri-drag-region></div>
        <header class="settings-heading">
          <h1>${text(t, "timerSettings")}</h1>
          <button class="close-button" id="cancel-settings" aria-label="${text(t, "cancel")}">×</button>
        </header>
        <form id="settings-form" class="settings-form">
          <div class="number-grid">
            <label>${text(t, "focusMinutes")}<input name="focusMinutes" type="number" min="1" max="180" value="${value.timer.focusMinutes}" required></label>
            <label>${text(t, "shortMinutes")}<input name="shortBreakMinutes" type="number" min="1" max="60" value="${value.timer.shortBreakMinutes}" required></label>
            <label>${text(t, "longMinutes")}<input name="longBreakMinutes" type="number" min="1" max="60" value="${value.timer.longBreakMinutes}" required></label>
            <label>${text(t, "cycleCount")}<input name="sessionsPerCycle" type="number" min="1" max="12" value="${value.timer.sessionsPerCycle}" required></label>
          </div>
          <div class="toggle-list">
            ${toggle("alwaysOnTop", text(t, "alwaysOnTop"), value.alwaysOnTop)}
            ${toggle("soundEnabled", text(t, "sound"), value.soundEnabled)}
            ${toggle("notificationsEnabled", text(t, "notifications"), value.notificationsEnabled)}
          </div>
          <label class="select-label">${text(t, "language")}
            <select name="locale">
              <option value="system" ${value.locale === "system" ? "selected" : ""}>${text(t, "system")}</option>
              <option value="zh-CN" ${value.locale === "zh-CN" ? "selected" : ""}>中文</option>
              <option value="en" ${value.locale === "en" ? "selected" : ""}>English</option>
            </select>
          </label>
          <button class="primary save-settings" type="submit" ${busy ? "disabled" : ""}>${busy ? text(t, "working") : text(t, "save")}</button>
          ${message ? `<p class="inline-message" role="status">${message}</p>` : ""}
        </form>
      </main>`;
    bind("cancel-settings", () => {
      settingsOpen = false;
      message = "";
      render();
    });
    document.querySelector<HTMLFormElement>("#settings-form")!.addEventListener("submit", saveSettings);
    return;
  }

  app.innerHTML = `
    <main class="timer-shell ${timer.mode}">
      <div class="drag-zone" data-tauri-drag-region></div>
      <div class="timer-header">
        <p class="eyebrow">${phaseTitle(t)}</p>
        <span class="status-dot ${timer.status}" aria-hidden="true"></span>
      </div>
      <div class="clock-wrap">
        <svg class="progress-ring" viewBox="0 0 140 140" aria-hidden="true">
          <circle class="ring-track" cx="70" cy="70" r="58"></circle>
          <circle class="ring-value" cx="70" cy="70" r="58"></circle>
        </svg>
        <div class="clock-copy">
          <strong id="time-value">${formatTimer(remainingMs())}</strong>
          <span>${statusLabel(timer.status, t)}</span>
        </div>
      </div>
      <nav class="timer-controls" aria-label="Timer controls">
        <button id="reset" class="icon-button" title="${text(t, "reset")}" ${busy ? "disabled" : ""}>${icon("reset")}</button>
        <button id="toggle" class="icon-button main-action" title="${timer.status === "running" ? text(t, "pause") : text(t, "start")}" ${busy ? "disabled" : ""}>${icon(timer.status === "running" ? "pause" : "play")}</button>
        <button id="settings" class="icon-button" title="${text(t, "settings")}">${icon("settings")}</button>
        <button id="reports" class="icon-button" title="${text(t, "reports")}">${icon("report")}</button>
      </nav>
      ${message ? `<p class="timer-message" role="status">${message}</p>` : ""}
    </main>`;
  bind("toggle", async () => {
    if (timer.status !== "running" && timer.settings.notificationsEnabled) await ensureNotifications();
    await command(timer.status === "running" ? "pause_timer" : "start_timer");
  });
  bind("reset", () => command("reset_timer"));
  bind("settings", () => {
    settingsOpen = true;
    message = "";
    render();
  });
  bind("reports", () => command("open_analytics", false));
  updateClock();
}

function toggle(name: string, label: string, checked: boolean): string {
  return `<label class="toggle-row"><span>${label}</span><input name="${name}" type="checkbox" ${checked ? "checked" : ""}><i aria-hidden="true"></i></label>`;
}

function bind(id: string, action: () => void | Promise<void>): void {
  document.querySelector(`#${id}`)?.addEventListener("click", () => void action());
}

async function command(name: string, returnsTimer = true): Promise<void> {
  busy = true;
  message = "";
  render();
  try {
    const value = await invoke<TimerSnapshot | void>(name);
    if (returnsTimer && value) timer = value;
  } catch (error) {
    message = String(error);
  } finally {
    busy = false;
    render();
  }
}

async function saveSettings(event: SubmitEvent): Promise<void> {
  event.preventDefault();
  const form = event.currentTarget as HTMLFormElement;
  const data = new FormData(form);
  const next: AppSettings = {
    ...timer.settings,
    timer: {
      focusMinutes: Number(data.get("focusMinutes")),
      shortBreakMinutes: Number(data.get("shortBreakMinutes")),
      longBreakMinutes: Number(data.get("longBreakMinutes")),
      sessionsPerCycle: Number(data.get("sessionsPerCycle")),
    },
    alwaysOnTop: data.get("alwaysOnTop") === "on",
    soundEnabled: data.get("soundEnabled") === "on",
    notificationsEnabled: data.get("notificationsEnabled") === "on",
    locale: data.get("locale") as AppSettings["locale"],
  };
  busy = true;
  render();
  try {
    timer = await invoke<TimerSnapshot>("update_settings", { settings: next });
    if (next.notificationsEnabled) await ensureNotifications();
    settingsOpen = false;
  } catch (error) {
    message = String(error);
  } finally {
    busy = false;
    render();
  }
}

async function ensureNotifications(): Promise<void> {
  try {
    if (!(await isPermissionGranted())) await requestPermission();
  } catch {
    // The timer remains fully usable if notification permission is unavailable.
  }
}

function updateClock(): void {
  const value = remainingMs();
  const time = document.querySelector<HTMLElement>("#time-value");
  const ring = document.querySelector<SVGCircleElement>(".ring-value");
  if (time) time.textContent = formatTimer(value);
  if (ring) {
    const progress = timer.totalMs ? 1 - value / timer.totalMs : 1;
    ring.style.strokeDasharray = String(circumference);
    ring.style.strokeDashoffset = String(circumference * (1 - Math.min(1, Math.max(0, progress))));
  }
}

function playChime(): void {
  const AudioContextClass = window.AudioContext;
  const context = new AudioContextClass();
  const now = context.currentTime;
  [523.25, 659.25].forEach((frequency, index) => {
    const oscillator = context.createOscillator();
    const gain = context.createGain();
    oscillator.frequency.value = frequency;
    gain.gain.setValueAtTime(0, now);
    gain.gain.linearRampToValueAtTime(0.12, now + 0.02 + index * 0.08);
    gain.gain.exponentialRampToValueAtTime(0.001, now + 0.45 + index * 0.08);
    oscillator.connect(gain).connect(context.destination);
    oscillator.start(now + index * 0.08);
    oscillator.stop(now + 0.5 + index * 0.08);
  });
  window.setTimeout(() => void context.close(), 800);
}

export async function mountTimer(): Promise<void> {
  timer = await invoke<TimerSnapshot>("get_timer");
  await listen<TimerSnapshot>("timer://state", (event) => {
    timer = event.payload;
    render();
  });
  await listen("timer://sound", playChime);
  await listen<string>("app://error", (event) => {
    message = event.payload;
    render();
  });
  window.setInterval(updateClock, 250);
  render();
}
