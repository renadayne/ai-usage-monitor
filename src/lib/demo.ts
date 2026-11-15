import type { HistoryPoint, ProviderId, ProviderSnapshot, QuotaWindow, Settings } from "./types";

// Sample data covering every card state. Used by tests and by `pnpm dev`
// when the page is opened in a plain browser (no Tauri backend).

const HOUR = 3_600_000;

function window(
  id: string,
  label: string,
  used: number | null,
  resetInHours: number | null,
  extra: Partial<QuotaWindow> = {},
): QuotaWindow {
  return {
    id,
    label,
    used_percent: used,
    remaining: null,
    limit: null,
    unit: null,
    resets_at:
      resetInHours === null ? null : new Date(Date.now() + resetInHours * HOUR).toISOString(),
    estimated: false,
    ...extra,
  };
}

export function demoSnapshots(): ProviderSnapshot[] {
  const now = Date.now();
  const iso = (offsetMs: number) => new Date(now + offsetMs).toISOString();
  return [
    {
      provider: "claude",
      activity: "active",
      health: { kind: "ok" },
      plan: "max",
      current_model: "claude-fable-5-2",
      last_activity_at: iso(-20_000),
      windows: [
        window("five_hour", "5 hours", 37, 2.4),
        window("weekly", "Weekly", 78, 96),
        window("weekly:opus", "Weekly · Opus", 94, 96),
      ],
      note: null,
      fetched_at: iso(-3 * 60_000),
    },
    {
      provider: "codex",
      activity: "idle",
      health: { kind: "rate_limited", retry_at: iso(8 * 60_000) },
      plan: "plus",
      current_model: "gpt-6.1-sol",
      last_activity_at: iso(-2 * HOUR),
      windows: [
        window("five_hour", "5 hours", 0, null, { estimated: true }),
        window("weekly", "Weekly", 12, 110),
      ],
      note: null,
      fetched_at: iso(-14 * 60_000),
    },
    {
      provider: "antigravity",
      activity: "not_running",
      health: { kind: "stale", since: iso(-5 * HOUR) },
      plan: "Google AI Pro",
      current_model: null,
      last_activity_at: null,
      windows: [
        window("pool:gemini-models:5h", "Gemini Models · 5 hours", 55, 1.2),
        window("pool:gemini-models:weekly", "Gemini Models · Weekly", 21, 140),
      ],
      note: "Antigravity is not running. Showing the last known numbers.",
      fetched_at: iso(-5 * HOUR),
    },
    {
      provider: "copilot",
      activity: "unknown",
      health: {
        kind: "auth_required",
        hint: "No GitHub token found. Connect GitHub in Settings, or install GitHub CLI and run `gh auth login`.",
      },
      plan: null,
      current_model: null,
      last_activity_at: null,
      windows: [],
      note: null,
      fetched_at: iso(0),
    },
  ];
}

export function demoSettings(): Settings {
  return {
    providers: {
      claude: { enabled: true, poll_interval_secs: null },
      codex: { enabled: true, poll_interval_secs: null },
      antigravity: { enabled: true, poll_interval_secs: 120 },
      copilot: { enabled: true, poll_interval_secs: null },
    },
    notifications_enabled: true,
    warn_threshold: 80,
    critical_threshold: 95,
    autostart: false,
    claude_dir: null,
    codex_dir: null,
  };
}

/** A sawtooth 5-hour window and a slowly climbing weekly window. */
export function demoHistory(provider: ProviderId, days: number): HistoryPoint[] {
  if (provider === "copilot") return [];
  const points: HistoryPoint[] = [];
  const now = Date.now();
  const steps = days * 12;
  for (let step = 0; step <= steps; step++) {
    const recorded_at = new Date(now - (steps - step) * 2 * HOUR).toISOString();
    points.push({
      window_id: "five_hour",
      label: "5 hours",
      used_percent: ((step * 17) % 60) + (step % 3) * 5,
      recorded_at,
    });
    points.push({
      window_id: "weekly",
      label: "Weekly",
      used_percent: Math.min(95, (step / steps) * 90),
      recorded_at,
    });
  }
  return points;
}
