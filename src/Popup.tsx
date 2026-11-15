import { ActivityDot, healthLabel } from "./components/status";
import { refresh, showDashboard } from "./lib/api";
import { formatPercent, severity, tightestWindow, type Severity } from "./lib/format";
import { PROVIDER_NAMES, type ProviderSnapshot } from "./lib/types";
import { useSettings } from "./lib/useSettings";
import { useSnapshots } from "./lib/useSnapshots";

const PERCENT_COLOR: Record<Severity, string> = {
  normal: "text-slate-700 dark:text-slate-200",
  warning: "text-amber-700 dark:text-amber-400",
  danger: "text-red-700 dark:text-red-400",
};

const SEVERITY_MARK: Record<Severity, string> = { normal: "", warning: "▲ ", danger: "⛔ " };

export function PopupRow({ snapshot }: { snapshot: ProviderSnapshot }) {
  const tightest = tightestWindow(snapshot);
  const level = severity(tightest?.used_percent ?? null);
  const problem = healthLabel(snapshot.health);

  return (
    <li className="flex items-center gap-2 px-3 py-2" data-testid={`row-${snapshot.provider}`}>
      <ActivityDot activity={snapshot.activity} showLabel={false} />
      <div className="min-w-0 flex-1">
        <div className="truncate text-sm font-medium">{PROVIDER_NAMES[snapshot.provider]}</div>
        <div className="truncate text-xs text-slate-500 dark:text-slate-400">
          {problem ?? tightest?.label ?? "No quota data"}
        </div>
      </div>
      <div className={`shrink-0 text-sm font-semibold tabular-nums ${PERCENT_COLOR[level]}`}>
        {SEVERITY_MARK[level]}
        {formatPercent(tightest?.used_percent ?? null)}
      </div>
    </li>
  );
}

export default function Popup() {
  const { snapshots: all } = useSnapshots();
  const [settings] = useSettings();
  const snapshots = all.filter(
    (snapshot) => settings?.providers[snapshot.provider]?.enabled ?? true,
  );

  return (
    <main className="flex h-screen flex-col overflow-hidden border border-slate-300 bg-white text-slate-900 dark:border-slate-600 dark:bg-slate-800 dark:text-slate-100">
      <ul className="flex-1 divide-y divide-slate-200 overflow-auto dark:divide-slate-700">
        {snapshots.map((snapshot) => (
          <PopupRow key={snapshot.provider} snapshot={snapshot} />
        ))}
        {snapshots.length === 0 && (
          <li className="px-3 py-2 text-sm text-slate-500 dark:text-slate-400">Loading…</li>
        )}
      </ul>
      <footer className="flex justify-between border-t border-slate-200 px-3 py-2 text-xs dark:border-slate-700">
        <button type="button" className="hover:underline" onClick={() => void showDashboard()}>
          Open dashboard
        </button>
        <button type="button" className="hover:underline" onClick={() => void refresh()}>
          Refresh
        </button>
      </footer>
    </main>
  );
}
