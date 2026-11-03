// How each figure reads on the page (and while it counts up).
import type { Figure, Metric, NumericMetric } from "./numbers";

export const decimals: Record<NumericMetric, number> = {
  memoryMB: 0,
  memoryPausedMB: 0,
  cpuPlayingPct: 3,
  cpuPausedPct: 3,
  downloadMB: 2,
};

const isCpu = (m: NumericMetric) => m === "cpuPlayingPct" || m === "cpuPausedPct";

export function formatFigure(metric: NumericMetric, value: number): string {
  // A download of 100 MB or more reads whole (274 MB), smaller ones to two decimals.
  const places = metric === "downloadMB" && value >= 100 ? 0 : decimals[metric];
  const n = value.toFixed(places);
  // CPU shares drop trailing zeros (0.05%, not 0.050%).
  return isCpu(metric) ? `${Number(n)}%` : `${n} MB`;
}

/** A figure as the page shows it: "81 MB", "0.022%", "6 to 7" processes, "None" to download. */
export function figureText(f: Figure, metric: Metric): string {
  if (metric === "processes") {
    const { min, max } = f.processes;
    return min === max ? `${min}` : `${min} to ${max}`;
  }
  if (metric === "downloadMB" && f.downloadMB === 0) return "None";
  return formatFigure(metric, f[metric]);
}

/** Bytes as Windows shows them: MB of 1,048,576 bytes, whole above 100 MB. */
export function formatBytes(bytes: number, decimals = 1): string {
  const mb = bytes / 1_048_576;
  return `${mb >= 100 ? Math.round(mb) : mb.toFixed(decimals)} MB`;
}
