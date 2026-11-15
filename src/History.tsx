import { useEffect, useState } from "react";
import { getHistory } from "./lib/api";
import { buildSeries } from "./lib/chart";
import { formatPercent } from "./lib/format";
import { PROVIDER_NAMES, type HistoryPoint, type ProviderId } from "./lib/types";

const DAYS = 7;
const WIDTH = 600;
const HEIGHT = 160;
const DAY_MS = 86_400_000;

// Colour plus a dash pattern, so series stay distinguishable without colour.
const STYLES = [
  { stroke: "#10b981", dash: undefined },
  { stroke: "#3b82f6", dash: "6 3" },
  { stroke: "#f59e0b", dash: "2 3" },
  { stroke: "#ec4899", dash: "8 3 2 3" },
  { stroke: "#8b5cf6", dash: "1 4" },
  { stroke: "#64748b", dash: "10 4" },
];

export function HistoryChart({
  provider,
  history,
  now,
}: {
  provider: ProviderId;
  history: HistoryPoint[];
  now: number;
}) {
  const name = PROVIDER_NAMES[provider];
  const series = buildSeries(history, {
    width: WIDTH,
    height: HEIGHT,
    from: now - DAYS * DAY_MS,
    to: now,
  });

  return (
    <section
      aria-label={`${name} history`}
      className="rounded-xl border border-slate-200 bg-white p-4 shadow-sm dark:border-slate-700 dark:bg-slate-800"
    >
      <h2 className="text-base font-semibold">{name}</h2>
      {series.length === 0 ? (
        <p className="mt-2 text-sm text-slate-500 dark:text-slate-400">
          No history yet. Points are recorded every time the quota is fetched.
        </p>
      ) : (
        <>
          <svg
            role="img"
            aria-label={`${name} usage over the last ${DAYS} days`}
            viewBox={`-4 -4 ${WIDTH + 8} ${HEIGHT + 8}`}
            className="mt-2 w-full"
          >
            {[0, 50, 100].map((percent) => {
              const y = HEIGHT - (percent / 100) * HEIGHT;
              return (
                <line
                  key={percent}
                  x1={0}
                  x2={WIDTH}
                  y1={y}
                  y2={y}
                  className="stroke-slate-200 dark:stroke-slate-700"
                  strokeWidth={1}
                />
              );
            })}
            {series.map((line, index) => {
              const style = STYLES[index % STYLES.length];
              return (
                <polyline
                  key={line.id}
                  data-testid={`series-${line.id}`}
                  points={line.points}
                  fill="none"
                  stroke={style.stroke}
                  strokeDasharray={style.dash}
                  strokeWidth={2}
                  strokeLinejoin="round"
                />
              );
            })}
          </svg>
          <div className="mt-1 flex justify-between text-xs text-slate-400 dark:text-slate-500">
            <span>{DAYS} days ago</span>
            <span>now</span>
          </div>
          <ul className="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-xs">
            {series.map((line, index) => {
              const style = STYLES[index % STYLES.length];
              return (
                <li key={line.id} className="flex items-center gap-1.5">
                  <svg width="22" height="6" aria-hidden="true">
                    <line
                      x1="0"
                      x2="22"
                      y1="3"
                      y2="3"
                      stroke={style.stroke}
                      strokeDasharray={style.dash}
                      strokeWidth="2"
                    />
                  </svg>
                  {line.label}
                  <span className="text-slate-500 tabular-nums dark:text-slate-400">
                    {formatPercent(line.last)}
                  </span>
                </li>
              );
            })}
          </ul>
        </>
      )}
    </section>
  );
}

export default function History({ providers }: { providers: ProviderId[] }) {
  const [history, setHistory] = useState<Partial<Record<ProviderId, HistoryPoint[]>>>({});
  const [error, setError] = useState<string | null>(null);
  const [now] = useState(() => Date.now());
  const key = providers.join(",");

  useEffect(() => {
    let disposed = false;
    for (const provider of key.split(",").filter(Boolean) as ProviderId[]) {
      getHistory(provider, DAYS)
        .then((points) => {
          if (!disposed) setHistory((current) => ({ ...current, [provider]: points }));
        })
        .catch((cause: unknown) => {
          if (!disposed) setError(String(cause));
        });
    }
    return () => {
      disposed = true;
    };
  }, [key]);

  return (
    <div className="flex flex-col gap-4">
      {error && (
        <p role="alert" className="text-sm text-red-600 dark:text-red-400">
          {error}
        </p>
      )}
      {providers.map((provider) => (
        <HistoryChart
          key={provider}
          provider={provider}
          history={history[provider] ?? []}
          now={now}
        />
      ))}
    </div>
  );
}
