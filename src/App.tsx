import { refresh } from "./lib/api";
import { useSnapshots } from "./lib/useSnapshots";

export default function App() {
  const { snapshots, error } = useSnapshots();

  return (
    <main className="min-h-screen bg-slate-50 p-6 text-slate-900 dark:bg-slate-900 dark:text-slate-100">
      <header className="flex items-center justify-between">
        <h1 className="text-xl font-semibold">AI Usage Monitor</h1>
        <button
          type="button"
          className="rounded border border-slate-300 px-3 py-1 text-sm dark:border-slate-600"
          onClick={() => void refresh()}
        >
          Refresh
        </button>
      </header>
      {error && (
        <p role="alert" className="mt-2 text-sm text-red-600">
          {error}
        </p>
      )}
      <pre className="mt-4 overflow-auto rounded bg-slate-100 p-3 text-xs dark:bg-slate-800">
        {JSON.stringify(snapshots, null, 2)}
      </pre>
    </main>
  );
}
