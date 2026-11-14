// Mirrors src-tauri/src/core/model.rs

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
