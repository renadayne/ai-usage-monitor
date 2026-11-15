import { ProviderCard } from "./components/ProviderCard";
import { refresh } from "./lib/api";
import { useNow } from "./lib/useNow";
import { useSnapshots } from "./lib/useSnapshots";

export default function Dashboard() {
  const { snapshots, error } = useSnapshots();
  const now = useNow();

  return (
    <main className="min-h-screen bg-slate-50 p-5 text-slate-900 dark:bg-slate-900 dark:text-slate-100">
      <header className="mb-4 flex items-center justify-between">
        <h1 className="text-xl font-semibold">AI Usage Monitor</h1>
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
      {snapshots.length === 0 && !error ? (
        <p className="text-sm text-slate-500 dark:text-slate-400">Loading…</p>
      ) : (
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
          {snapshots.map((snapshot) => (
            <ProviderCard
              key={snapshot.provider}
              snapshot={snapshot}
              now={now}
              onRefresh={() => void refresh(snapshot.provider)}
            />
          ))}
        </div>
      )}
    </main>
  );
}
