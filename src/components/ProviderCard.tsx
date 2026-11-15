import { formatAgo, formatDuration } from "../lib/format";
import { PROVIDER_NAMES, type Health, type ProviderSnapshot } from "../lib/types";
import { QuotaBar } from "./QuotaBar";
import { ActivityDot, HealthBadge } from "./status";

function healthMessage(health: Health, now: number): string | null {
  switch (health.kind) {
    case "ok":
      return null;
    case "stale":
      return `Showing data from ${formatAgo(health.since, now) ?? "earlier"}.`;
    case "auth_required":
      return health.hint;
    case "not_installed":
      return "This tool was not found on this computer.";
    case "rate_limited": {
      const wait = Date.parse(health.retry_at) - now;
      return wait > 0
        ? `The provider is rate limiting requests. Retrying in ${formatDuration(wait)}.`
        : "The provider is rate limiting requests. Retrying soon.";
    }
    case "error":
      return health.message;
  }
}

export interface ProviderCardProps {
  snapshot: ProviderSnapshot;
  now: number;
  onRefresh?: () => void;
}

export function ProviderCard({ snapshot, now, onRefresh }: ProviderCardProps) {
  const name = PROVIDER_NAMES[snapshot.provider];
  const message = healthMessage(snapshot.health, now);
  const updated = formatAgo(snapshot.fetched_at, now);
  const hasData = snapshot.windows.length > 0;

  return (
    <section
      aria-label={name}
      className="flex flex-col gap-3 rounded-xl border border-slate-200 bg-white p-4 shadow-sm dark:border-slate-700 dark:bg-slate-800"
    >
      <header className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="text-base font-semibold">{name}</h2>
            {snapshot.plan && (
              <span className="rounded bg-slate-100 px-1.5 py-0.5 text-xs text-slate-600 dark:bg-slate-700 dark:text-slate-300">
                {snapshot.plan}
              </span>
            )}
            <HealthBadge health={snapshot.health} />
          </div>
          <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1">
            <ActivityDot activity={snapshot.activity} />
            <span className="truncate text-xs text-slate-500 dark:text-slate-400">
              Model: {snapshot.current_model ?? "unknown"}
            </span>
          </div>
        </div>
        {onRefresh && (
          <button
            type="button"
            onClick={onRefresh}
            aria-label={`Refresh ${name}`}
            className="shrink-0 rounded-md border border-slate-300 px-2 py-1 text-xs hover:bg-slate-100 dark:border-slate-600 dark:hover:bg-slate-700"
          >
            Refresh
          </button>
        )}
      </header>

      {message && (
        <p role="status" className="text-sm text-slate-600 dark:text-slate-300">
          {message}
        </p>
      )}

      {hasData && (
        <div className="flex flex-col gap-3">
          {snapshot.windows.map((window) => (
            <QuotaBar key={window.id} window={window} now={now} />
          ))}
        </div>
      )}
      {!hasData && snapshot.health.kind === "ok" && !snapshot.note && (
        <p className="text-sm text-slate-500 dark:text-slate-400">No quota data yet.</p>
      )}

      {snapshot.note && (
        <p className="text-xs text-slate-500 dark:text-slate-400">{snapshot.note}</p>
      )}

      {hasData && updated && (
        <footer className="mt-auto text-xs text-slate-400 dark:text-slate-500">
          Updated {updated}
        </footer>
      )}
    </section>
  );
}
