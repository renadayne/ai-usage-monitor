// Mirrors src-tauri/src/core/model.rs and settings.rs

export type ProviderId = "claude" | "codex" | "antigravity" | "copilot";

export const PROVIDER_IDS: ProviderId[] = ["claude", "codex", "antigravity", "copilot"];

export const PROVIDER_NAMES: Record<ProviderId, string> = {
  claude: "Claude",
  codex: "Codex",
  antigravity: "Antigravity",
  copilot: "GitHub Copilot",
};

export type Activity = "active" | "idle" | "not_running" | "unknown";

export type Health =
  | { kind: "ok" }
  | { kind: "stale"; since: string }
  | { kind: "auth_required"; hint: string }
  | { kind: "not_installed" }
  | { kind: "rate_limited"; retry_at: string }
  | { kind: "error"; message: string };

export interface QuotaWindow {
  id: string;
  label: string;
  used_percent: number | null;
  remaining: number | null;
  limit: number | null;
  unit: string | null;
  resets_at: string | null;
  estimated: boolean;
}

export interface ProviderSnapshot {
  provider: ProviderId;
  activity: Activity;
  health: Health;
  plan: string | null;
  current_model: string | null;
  last_activity_at: string | null;
  windows: QuotaWindow[];
  note: string | null;
  fetched_at: string;
}

export interface ProviderPrefs {
  enabled: boolean;
  /** `null` uses the provider's default interval. */
  poll_interval_secs: number | null;
}

export interface Settings {
  providers: Record<ProviderId, ProviderPrefs>;
  notifications_enabled: boolean;
  warn_threshold: number;
  critical_threshold: number;
  autostart: boolean;
  claude_dir: string | null;
  codex_dir: string | null;
}

/** Fastest allowed polling per provider (seconds); the backend enforces the same floors. */
export const MIN_POLL_SECS: Record<ProviderId, number> = {
  claude: 300,
  codex: 60,
  antigravity: 30,
  copilot: 300,
};

/** Interval used when the user has not chosen one (seconds). */
export const DEFAULT_POLL_SECS: Record<ProviderId, number> = {
  claude: 300,
  codex: 60,
  antigravity: 60,
  copilot: 600,
};

export interface HistoryPoint {
  window_id: string;
  label: string;
  used_percent: number;
  recorded_at: string;
}

export interface DeviceCode {
  user_code: string;
  verification_uri: string;
  expires_in: number;
  interval: number;
}

export interface CopilotAuthEvent {
  state: "connected" | "failed";
  message: string | null;
}
