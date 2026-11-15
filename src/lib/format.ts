import type { ProviderSnapshot, QuotaWindow } from "./types";

export type Severity = "normal" | "warning" | "danger";

/** < 70 % normal, 70–90 % warning, > 90 % danger. */
export function severity(usedPercent: number | null): Severity {
  if (usedPercent === null) return "normal";
  if (usedPercent > 90) return "danger";
  if (usedPercent >= 70) return "warning";
  return "normal";
}

export function formatPercent(usedPercent: number | null): string {
  return usedPercent === null ? "–" : `${Math.round(usedPercent)}%`;
}

/** "2d 3h", "4h 12m", "12m", "<1m" */
export function formatDuration(milliseconds: number): string {
  const minutes = Math.floor(milliseconds / 60_000);
  if (minutes < 1) return "<1m";
  const days = Math.floor(minutes / 1440);
  const hours = Math.floor((minutes % 1440) / 60);
  const rest = minutes % 60;
  if (days > 0) return hours > 0 ? `${days}d ${hours}h` : `${days}d`;
  if (hours > 0) return rest > 0 ? `${hours}h ${rest}m` : `${hours}h`;
  return `${rest}m`;
}

export function formatCountdown(resetsAt: string | null, now: number): string | null {
  if (!resetsAt) return null;
  const target = Date.parse(resetsAt);
  if (Number.isNaN(target)) return null;
  const remaining = target - now;
  return remaining <= 0 ? "resetting…" : `resets in ${formatDuration(remaining)}`;
}

export function formatAgo(time: string | null, now: number): string | null {
  if (!time) return null;
  const then = Date.parse(time);
  if (Number.isNaN(then)) return null;
  const minutes = Math.floor((now - then) / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return `${hours} h ago`;
  return `${Math.floor(hours / 24)} d ago`;
}

/** The window closest to its limit, if any reports a percentage. */
export function tightestWindow(snapshot: ProviderSnapshot): QuotaWindow | null {
  let best: QuotaWindow | null = null;
  for (const window of snapshot.windows) {
    if (window.used_percent === null) continue;
    if (best === null || window.used_percent > (best.used_percent ?? -1)) best = window;
  }
  return best;
}

export function formatAmount(value: number): string {
  return Number.isInteger(value) ? value.toLocaleString("en-US") : value.toFixed(1);
}
