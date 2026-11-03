// The comparison's figures: the final measurement of 1 November 2026 (19:07 to 19:58), all
// three apps on the same PC, the same song on repeat, window minimised, each the only player
// running. Every process of each app is counted. Memory is private memory, averaged, in MB
// of 1,048,576 bytes; CPU is the share of the whole machine (CPU seconds over wall seconds
// over 16 logical cores). Every figure names the file it came from, by its path inside the
// measurement write-up, which is kept outside the repo; the Method page shows the file names.

export type Metric = "memoryMB" | "memoryPausedMB" | "cpuPlayingPct" | "cpuPausedPct" | "processes" | "downloadMB";
/** The measures that are a single number (processes are a range). */
export type NumericMetric = Exclude<Metric, "processes">;

export const metrics: Metric[] = ["memoryMB", "memoryPausedMB", "cpuPlayingPct", "cpuPausedPct", "processes", "downloadMB"];

export type App = "featherplay" | "Pear Desktop" | "YouTube Music in Edge";

export type Figure = {
  app: App;
  version: string;
  /** Private memory, all processes, a song playing, window minimised (5 minutes). */
  memoryMB: number;
  /** The same with the song paused (3 minutes). */
  memoryPausedMB: number;
  cpuPlayingPct: number;
  cpuPausedPct: number;
  /** How many processes it ran, fewest and most seen. */
  processes: { min: number; max: number };
  /** What you download to install it; 0 for a browser that comes with Windows. */
  downloadMB: number;
  /** What the download is, in a few words. */
  download: string;
  sources: Record<Metric, string>;
};

import { release } from "./release";

const M = "measurements/";
const FINAL = `${M}final-2026-11-01.md`;
const runs = (playing: string, paused: string): Record<Metric, string> => ({
  memoryMB: `${M}${playing}`,
  memoryPausedMB: `${M}${paused}`,
  cpuPlayingPct: `${M}${playing}`,
  cpuPausedPct: `${M}${paused}`,
  processes: `${M}${playing}`,
  downloadMB: FINAL,
});

export const figures: Figure[] = [
  {
    app: "featherplay",
    version: release.version,
    memoryMB: 81,
    memoryPausedMB: 80,
    cpuPlayingPct: 0.022,
    cpuPausedPct: 0.005,
    processes: { min: 1, max: 1 },
    // The installer (release.ts); the portable exe is the other option.
    downloadMB: release.installer!.sizeMB!,
    download: "the installer",
    sources: runs("featherplay_final-playing2_2026-11-01_1907.csv", "featherplay_final-paused_2026-11-01_1913.csv"),
  },
  {
    app: "Pear Desktop",
    version: "3.12.0",
    memoryMB: 798,
    memoryPausedMB: 782,
    cpuPlayingPct: 0.17,
    cpuPausedPct: 0.05,
    processes: { min: 6, max: 7 },
    downloadMB: 274,
    download: "the installer",
    sources: runs("pear_final-playing-2026-11-01_2026-11-01_1918.csv", "pear_final-paused-2026-11-01_2026-11-01_1924.csv"),
  },
  {
    app: "YouTube Music in Edge",
    version: "156.0.4314.58",
    memoryMB: 988,
    memoryPausedMB: 973,
    cpuPlayingPct: 0.14,
    cpuPausedPct: 0.018,
    processes: { min: 9, max: 11 },
    downloadMB: 0,
    download: "nothing, Edge comes with Windows",
    sources: runs("edge_final-playing3_2026-11-01_1946.csv", "edge_final-paused_2026-11-01_1952.csv"),
  },
];

export const figureFor = (app: App) => figures.find((f) => f.app === app)!;

/** A figure as one number: a range of processes by its upper end. */
export const valueOf = (f: Figure, m: Metric): number => (m === "processes" ? f.processes.max : f[m]);

/** How many times less featherplay uses than `app`, to the nearest whole time. */
export const timesLess = (m: NumericMetric, app: App): number =>
  Math.round(figureFor(app)[m] / figureFor("featherplay")[m]);

/** What each group of Pear's entries adds up to: how many, and how many bytes. */
export function leftOutTotals(group: LeftOut): { entries: number; bytes: number } {
  const list = installFolders.pear.entries.filter((e) => e.group === group);
  return { entries: list.length, bytes: list.reduce((n, e) => n + e.bytes, 0) };
}

// The two install folders ("What featherplay leaves out"): Pear Desktop 3.12.0.0 as it sits
// on the measuring PC, and featherplay's portable exe. Sizes in bytes.
// Installed (%LOCALAPPDATA%/Programs/featherplay), featherplay is the same exe plus Inno
// Setup's uninstaller, unins000.exe (about 4.4 million bytes, measured; Inno 6.7.4's
// Setup.e32 it's made from is 4,427,264) and its small unins000.dat log: not listed byte for
// byte, so the installed total is shown as "about".
// Each of Pear's entries is tagged with what it's there for, where that's clear from the
// file: Chromium (the browser engine and its pieces), Electron (the app shell and the web
// app it runs) or video (media decoding and the software graphics fallback).
export type LeftOut = "chromium" | "electron" | "video";
export type FolderEntry = { name: string; bytes: number; files?: number; group?: LeftOut };
export const installFolders = {
  source: "site/pear-install-folder-2026-11-01.md",
  pear: {
    path: ["Programs", "youtube-music"],
    files: 105,
    bytes: 415_193_750,
    // As Explorer sorts them: folders first, then files by name.
    entries: [
      { name: "locales", bytes: 49_471_498, files: 55, group: "chromium" },
      { name: "resources", bytes: 43_837_504, files: 31, group: "electron" },
      { name: "chrome_100_percent.pak", bytes: 119_889, group: "chromium" },
      { name: "chrome_200_percent.pak", bytes: 197_073, group: "chromium" },
      { name: "d3dcompiler_47.dll", bytes: 4_741_488, group: "chromium" },
      { name: "dxcompiler.dll", bytes: 25_634_304, group: "chromium" },
      { name: "dxil.dll", bytes: 1_509_760, group: "chromium" },
      { name: "ffmpeg.dll", bytes: 3_057_152, group: "video" },
      { name: "icudtl.dat", bytes: 10_876_560, group: "chromium" },
      { name: "libEGL.dll", bytes: 478_208, group: "chromium" },
      { name: "libGLESv2.dll", bytes: 7_998_464, group: "chromium" },
      { name: "LICENSE.electron.txt", bytes: 1_096, group: "electron" },
      { name: "LICENSES.chromium.html", bytes: 20_367_095, group: "chromium" },
      { name: "resources.pak", bytes: 6_876_382, group: "chromium" },
      { name: "snapshot_blob.bin", bytes: 346_432, group: "chromium" },
      { name: "Uninstall YouTube Music.exe", bytes: 162_251 },
      { name: "v8_context_snapshot.bin", bytes: 721_176, group: "chromium" },
      { name: "vk_swiftshader.dll", bytes: 5_520_384, group: "video" },
      { name: "vk_swiftshader_icd.json", bytes: 106, group: "video" },
      { name: "vulkan-1.dll", bytes: 925_696, group: "video" },
      { name: "YouTube Music.exe", bytes: 232_351_232, group: "electron" },
    ] as FolderEntry[],
  },
  featherplay: {
    path: ["Downloads"],
    files: 1,
    bytes: release.portable?.bytes ?? 0,
    entries: [{ name: "featherplay.exe", bytes: release.portable?.bytes ?? 0 }] as FolderEntry[],
    /** The installed folder: the exe and the uninstaller (approximate). */
    installed: { uninstallerBytes: 4_427_264, source: "installer.md" },
  },
};
