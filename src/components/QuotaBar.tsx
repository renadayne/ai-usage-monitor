import {
  formatAmount,
  formatCountdown,
  formatPercent,
  severity,
  type Severity,
} from "../lib/format";
import type { QuotaWindow } from "../lib/types";

const BAR_COLOR: Record<Severity, string> = {
  normal: "bg-emerald-500",
  warning: "bg-amber-500",
  danger: "bg-red-500",
};

const TEXT_COLOR: Record<Severity, string> = {
  normal: "text-slate-600 dark:text-slate-300",
  warning: "text-amber-700 dark:text-amber-400",
  danger: "text-red-700 dark:text-red-400",
};

// Severity is never conveyed by colour alone.
const SEVERITY_LABEL: Record<Severity, string | null> = {
  normal: null,
  warning: "▲ High",
  danger: "⛔ Critical",
};

export function QuotaBar({ window, now }: { window: QuotaWindow; now: number }) {
  const level = severity(window.used_percent);
  const countdown = formatCountdown(window.resets_at, now);
  const width = Math.min(100, Math.max(0, window.used_percent ?? 0));
  const amount =
    window.remaining !== null
      ? `${formatAmount(window.remaining)}${window.limit !== null ? ` / ${formatAmount(window.limit)}` : ""} ${window.unit ?? ""} left`.trim()
      : null;

  return (
    <div data-testid={`window-${window.id}`} data-severity={level}>
      <div className="flex items-baseline justify-between gap-2 text-sm">
        <span className="truncate font-medium">{window.label}</span>
        <span className={`shrink-0 tabular-nums ${TEXT_COLOR[level]}`}>
          {SEVERITY_LABEL[level] && <span className="mr-1.5 text-xs">{SEVERITY_LABEL[level]}</span>}
          {formatPercent(window.used_percent)}
          {window.estimated && <span className="ml-1 text-xs">(estimated)</span>}
        </span>
      </div>
      <div
        role="progressbar"
        aria-label={`${window.label} used`}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(width)}
        className="mt-1 h-2 overflow-hidden rounded-full bg-slate-200 dark:bg-slate-700"
      >
        <div className={`h-full rounded-full ${BAR_COLOR[level]}`} style={{ width: `${width}%` }} />
      </div>
      {(countdown || amount) && (
        <div className="mt-1 flex justify-between gap-2 text-xs text-slate-500 dark:text-slate-400">
          <span>{amount}</span>
          <span>{countdown}</span>
        </div>
      )}
    </div>
  );
}
