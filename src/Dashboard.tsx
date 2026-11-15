import { useState } from "react";
import { ProviderCard } from "./components/ProviderCard";
import History from "./History";
import { refresh } from "./lib/api";
import type { ProviderId } from "./lib/types";
import { useNow } from "./lib/useNow";
import { useSettings } from "./lib/useSettings";
import { useSnapshots } from "./lib/useSnapshots";
import Settings from "./Settings";

type Tab = "overview" | "history" | "settings";

const TABS: { id: Tab; label: string }[] = [
  { id: "overview", label: "Overview" },
  { id: "history", label: "History" },
  { id: "settings", label: "Settings" },
];

export default function Dashboard() {
  const { snapshots, error } = useSnapshots();
  const [settings, setSettings] = useSettings();
  const [tab, setTab] = useState<Tab>("overview");
  const now = useNow();

  const enabled = (provider: ProviderId) => settings?.providers[provider]?.enabled ?? true;
  const visible = snapshots.filter((snapshot) => enabled(snapshot.provider));

  return (
    <main className="min-h-screen bg-slate-50 p-5 text-slate-900 dark:bg-slate-900 dark:text-slate-100">
      <header className="mb-4 flex flex-wrap items-center justify-between gap-3">
        <h1 className="text-xl font-semibold">AI Usage Monitor</h1>
        <nav aria-label="Sections" className="flex gap-1">
          {TABS.map(({ id, label }) => (
            <button
              key={id}
              type="button"
              aria-current={tab === id ? "page" : undefined}
              onClick={() => setTab(id)}
              className={`rounded-md px-3 py-1.5 text-sm ${
                tab === id
                  ? "bg-slate-900 text-white dark:bg-slate-100 dark:text-slate-900"
                  : "hover:bg-slate-200 dark:hover:bg-slate-800"
              }`}
            >
              {label}
            </button>
          ))}
        </nav>
        <button
          type="button"
          className="rounded-md border border-slate-300 px-3 py-1.5 text-sm hover:bg-slate-100 dark:border-slate-600 dark:hover:bg-slate-800"
          onClick={() => void refresh()}
        >
          Refresh all
        </button>
      </header>
      {error && (
        <p role="alert" className="mb-3 text-sm text-red-600 dark:text-red-400">
          {error}
        </p>
      )}

      {tab === "overview" &&
        (snapshots.length === 0 && !error ? (
          <p className="text-sm text-slate-500 dark:text-slate-400">Loading…</p>
        ) : visible.length === 0 ? (
          <p className="text-sm text-slate-500 dark:text-slate-400">
            Every provider is turned off. Enable one in Settings.
          </p>
        ) : (
          <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
            {visible.map((snapshot) => (
              <ProviderCard
                key={snapshot.provider}
                snapshot={snapshot}
                now={now}
                onRefresh={() => void refresh(snapshot.provider)}
              />
            ))}
          </div>
        ))}

      {tab === "history" && <History providers={visible.map((snapshot) => snapshot.provider)} />}

      {tab === "settings" &&
        (settings ? (
          <Settings settings={settings} onSaved={setSettings} />
        ) : (
          <p className="text-sm text-slate-500 dark:text-slate-400">Loading…</p>
        ))}
    </main>
  );
}
