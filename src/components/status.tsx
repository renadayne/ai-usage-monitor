import type { Activity, Health } from "../lib/types";

const ACTIVITY: Record<Activity, { label: string; dot: string }> = {
  active: { label: "Active", dot: "bg-emerald-500" },
  idle: { label: "Idle", dot: "bg-amber-400" },
  not_running: { label: "Not running", dot: "bg-slate-400" },
  unknown: { label: "Unknown", dot: "bg-slate-300 dark:bg-slate-600" },
};

export function activityLabel(activity: Activity): string {
  return ACTIVITY[activity].label;
}

export function ActivityDot({
  activity,
  showLabel = true,
}: {
  activity: Activity;
  showLabel?: boolean;
}) {
  const { label, dot } = ACTIVITY[activity];
  return (
    <span className="inline-flex items-center gap-1.5 text-xs text-slate-600 dark:text-slate-300">
      <span
        aria-hidden={showLabel}
        aria-label={showLabel ? undefined : label}
        title={label}
        className={`inline-block h-2.5 w-2.5 rounded-full ${dot} ${activity === "active" ? "animate-pulse" : ""}`}
      />
      {showLabel && label}
    </span>
  );
}

const BADGE: Record<Exclude<Health["kind"], "ok">, { label: string; style: string }> = {
  stale: {
    label: "Stale",
    style: "bg-slate-200 text-slate-700 dark:bg-slate-700 dark:text-slate-200",
  },
  auth_required: {
    label: "Sign-in required",
    style: "bg-amber-100 text-amber-800 dark:bg-amber-900/50 dark:text-amber-200",
  },
  not_installed: {
    label: "Not installed",
    style: "bg-slate-200 text-slate-700 dark:bg-slate-700 dark:text-slate-200",
  },
  rate_limited: {
    label: "Rate limited",
    style: "bg-orange-100 text-orange-800 dark:bg-orange-900/50 dark:text-orange-200",
  },
  error: { label: "Error", style: "bg-red-100 text-red-800 dark:bg-red-900/50 dark:text-red-200" },
};

export function healthLabel(health: Health): string | null {
  return health.kind === "ok" ? null : BADGE[health.kind].label;
}

export function HealthBadge({ health }: { health: Health }) {
  if (health.kind === "ok") return null;
  const { label, style } = BADGE[health.kind];
  return <span className={`rounded px-1.5 py-0.5 text-xs font-medium ${style}`}>{label}</span>;
}
