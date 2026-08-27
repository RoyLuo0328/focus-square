export type TimerMode = "focus" | "shortBreak" | "longBreak";
export type TimerStatus = "idle" | "running" | "paused" | "completed";

export interface TimerSettings {
  focusMinutes: number;
  shortBreakMinutes: number;
  longBreakMinutes: number;
  sessionsPerCycle: number;
}

export interface AppSettings {
  timer: TimerSettings;
  locale: "system" | "zh-CN" | "en";
  alwaysOnTop: boolean;
  soundEnabled: boolean;
  notificationsEnabled: boolean;
  aiBaseUrl: string;
  aiModel: string;
}

export interface TimerSnapshot {
  mode: TimerMode;
  status: TimerStatus;
  remainingMs: number;
  totalMs: number;
  endAtEpochMs: number | null;
  completedFocuses: number;
  settings: AppSettings;
}

export interface ReportPeriod {
  startDate: string;
  endDate: string;
}

export interface ReportMetrics {
  totalFocusSeconds: number;
  activeDays: number;
  attemptedSessions: number;
  completedSessions: number;
  completionRate: number;
  averageSessionSeconds: number;
  longestStreakDays: number;
  peakHour: number | null;
  lateNightRatio: number;
}

export interface FocusSession {
  id: string;
  startedAtMs: number;
  endedAtMs: number;
  plannedSeconds: number;
  activeSeconds: number;
  outcome: "completed" | "completed_after_pause" | "interrupted" | "running";
}

export interface ReportData {
  period: ReportPeriod;
  previousPeriod: ReportPeriod;
  metrics: ReportMetrics;
  previousMetrics: ReportMetrics;
  comparison: {
    focusSecondsChangeRatio: number | null;
    completionRateChange: number;
    activeDaysChange: number;
  };
  daily: Array<{ date: string; focusSeconds: number }>;
  hourly: Array<{ hour: number; focusSeconds: number }>;
  insights: Array<{ kind: string; title: string; detail: string }>;
  sessions: FocusSession[];
  dataHash: string;
  aiAnalysis: string | null;
  aiAnalysisStale: boolean;
}

export interface AiConfig {
  baseUrl: string;
  model: string;
  hasApiKey: boolean;
}

export interface AiAnalysis {
  content: string;
  model: string;
  generatedAtMs: number;
  dataHash: string;
}
