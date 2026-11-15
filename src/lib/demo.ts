import type { ProviderSnapshot, QuotaWindow } from "./types";

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
