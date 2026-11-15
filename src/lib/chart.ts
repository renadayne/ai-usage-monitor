import type { HistoryPoint } from "./types";

export interface ChartSeries {
  id: string;
  label: string;
  /** SVG polyline `points` attribute. */
  points: string;
  /** Most recent value of the series. */
  last: number;
  count: number;
}

export interface ChartBox {
  width: number;
  height: number;
  /** Start and end of the time axis (ms since epoch). */
  from: number;
  to: number;
}

/** Groups history points by window and projects them into the chart box (0–100 % on y). */
export function buildSeries(history: HistoryPoint[], box: ChartBox): ChartSeries[] {
  const span = Math.max(1, box.to - box.from);
  const groups = new Map<string, { label: string; rows: { at: number; used: number }[] }>();

  for (const point of history) {
    const at = Date.parse(point.recorded_at);
    if (Number.isNaN(at) || at < box.from || at > box.to) continue;
    let group = groups.get(point.window_id);
    if (!group) {
      group = { label: point.label, rows: [] };
      groups.set(point.window_id, group);
    }
    group.label = point.label;
    group.rows.push({ at, used: Math.min(100, Math.max(0, point.used_percent)) });
  }

  return [...groups.entries()].map(([id, group]) => {
    group.rows.sort((a, b) => a.at - b.at);
    const points = group.rows
      .map(({ at, used }) => {
        const x = ((at - box.from) / span) * box.width;
        const y = box.height - (used / 100) * box.height;
        return `${x.toFixed(1)},${y.toFixed(1)}`;
      })
      .join(" ");
    return {
      id,
      label: group.label,
      points,
      last: group.rows[group.rows.length - 1].used,
      count: group.rows.length,
    };
  });
}
