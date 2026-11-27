// The comparison's figures, from two measurements on the same PC, each app playing a song on
// repeat with its window minimised, the only player running. Every process of each app is
// counted. Memory is private memory, averaged, in MB of 1,048,576 bytes; CPU is the share of
// the whole machine (CPU seconds over wall seconds over 16 logical cores); featherplay's are
// its processor cycles as CPU seconds, as Windows' charged time no longer followed its work.
// The PC: 16 logical processors, a 3840 x 2160 display at 150 % scaling (240 Hz). With the
// graphics card, featherplay's memory depends on the window's size (its screen buffers alone are
// about 32 MB each at 4K), so a smaller display uses less.
// - featherplay: 0.8.0 on 27 November 2026 (15:42 to 16:51), a release build of its code with
//   the build keys, built on the measuring PC, not the release's own exe; three runs, the
//   median, by 0.4.0's method, in its default mode (drawing with the graphics card). Its two
//   timings too (start to music playing, click to sound), which only featherplay has: Pear
//   Desktop and Edge weren't part of that measurement. The processor mode, measured beside it,
//   is `processorMode`, beside the table. The one run with the sound features on
//   (withProcessing) is 0.2.0's, of 9 November, and wasn't repeated.
// - Pear Desktop and Edge: the final measurement of 1 November 2026 (19:07 to 19:58), not
//   repeated.
// Every figure names the files it came from, by their path inside the measurement write-ups,
// which are kept outside the repo; the Method page shows them.

export type Metric =
  | "memoryMB"
  | "memoryPausedMB"
  | "cpuPlayingPct"
  | "cpuPausedPct"
  | "processes"
  | "startToSoundS"
  | "clickToSoundS"
  | "downloadMB";
/** The measures that are a single number (processes are a range). */
export type NumericMetric = Exclude<Metric, "processes">;
/** The measures only featherplay has: null for the others. */
export type Timing = "startToSoundS" | "clickToSoundS";

export const metrics: Metric[] = [
  "memoryMB",
  "memoryPausedMB",
  "cpuPlayingPct",
  "cpuPausedPct",
  "processes",
  "startToSoundS",
  "clickToSoundS",
  "downloadMB",
];

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
  /** Seconds from the process starting to the first sound of the song it resumes (median);
   *  null where it wasn't part of the measurement. */
  startToSoundS: number | null;
  /** Seconds from a click on a song's play button to its first sound (median); null where it
   *  wasn't part of the measurement. */
  clickToSoundS: number | null;
  /** What you download to install it; 0 for a browser that comes with Windows. */
  downloadMB: number;
  /** What the download is, in a few words. */
  download: string;
  sources: Record<Metric, string | null>;
};

import { release } from "./release";

const M = "measurements/";
const FINAL = `${M}final-2026-11-01.md`;
const runs = (playing: string, paused: string): Record<Metric, string | null> => ({
  memoryMB: `${M}${playing}`,
  memoryPausedMB: `${M}${paused}`,
  cpuPlayingPct: `${M}${playing}`,
  cpuPausedPct: `${M}${paused}`,
  processes: `${M}${playing}`,
  startToSoundS: null,
  clickToSoundS: null,
  downloadMB: FINAL,
});
// The write-ups: their samples in csv/ (the median of the runs is used). 0.8.0's set of
// 27 November holds the figures: gpu-1, -2 and -3 (the default mode), interleaved with cpu-1,
// -2 and -3 (the processor mode) and two runs of 0.7.0 for comparison, which aren't used.
// 0.2.0's set has the one run with the sound features on.
const V080 = `${M}final-1127b/`;
const RUNS = "gpu-{1,2,3}";
const V020 = `${M}final-0.2.0/`;

/** featherplay's two timings in detail: the median, the range, and how many. */
export const timings = {
  startToSoundS: { median: 0.312, min: 0.308, max: 0.33, n: 6 },
  clickToSoundS: { median: 0.026, min: 0.001, max: 0.371, n: 12 },
  /** Clicks on a song the pointer rested on first, and clicks straight away (6 each). */
  hovered: { median: 0.005, min: 0.001, max: 0.01, n: 6 },
  notHovered: { median: 0.25, min: 0.043, max: 0.371, n: 6 },
  /** The same 12 clicks by what was ready before them: the song prepared ahead while another
   *  played, only the hover's own work, or nothing (cold). */
  preparedAhead: { median: 0.026, min: 0.001, max: 0.371, n: 10 },
  hoveredOnly: { median: 0.01, min: 0.01, max: 0.01, n: 1 },
  cold: { median: 0.27, min: 0.27, max: 0.27, n: 1 },
};

/** featherplay's processor mode (Settings › Window › Performance), measured the same afternoon
 *  in the same way, three runs interleaved with the default mode's: beside the table, not in
 *  it. Memory, and CPU in cycles, while playing and paused; the shown window's figures are
 *  `shownWindow`'s. */
export const processorMode = {
  memoryMB: 30,
  memoryPausedMB: 30,
  cpuPlayingPct: 0.048,
  cpuPausedPct: 0.002,
  source: `${V080}csv/cpu-{1,2,3}-playing.csv`,
};

/** Memory while playing from 40 s after minimising, once the default mode has let go of the
 *  graphics card's memory (30 s after): the median of the same three runs. */
export const settledMemoryMB = { gpu: 44.6, processor: 29.7 };

/** The window shown on Home, playing, for 60 s (three runs each, the median): not part of the
 *  comparison, which is minimised. Task Manager's memory is the private working set. */
export const shownWindow = {
  gpu: { taskManagerMB: 154, gpuMB: 117 },
  processor: { taskManagerMB: 57, gpuMB: 0 },
  source: `${V080}csv/shown-{gpu,cpu}-{1,2,3}.csv`,
};

/** One extra run of 0.2.0 with the sound features on (Equalizer Bass, Even out loudness Light,
 *  Skip silence Start and end, Crossfade 5 s): beside the table, not in it, and not repeated
 *  for later versions. */
export const withProcessing = {
  memoryMB: 56,
  cpuPlayingPct: 0.087,
  source: `${V020}csv/processing-1-playing.csv`,
};

export const figures: Figure[] = [
  {
    app: "featherplay",
    // The version measured (the README's table says the same), drawing with the graphics card,
    // its default. Only the download is release.ts's.
    version: "0.8.0",
    memoryMB: 56,
    memoryPausedMB: 45,
    // In processor cycles (0.0485 to 0.0492%); the charged CPU time read 0.031%, too noisy.
    cpuPlayingPct: 0.049,
    cpuPausedPct: 0.002,
    processes: { min: 1, max: 1 },
    startToSoundS: timings.startToSoundS.median,
    clickToSoundS: timings.clickToSoundS.median,
    // The installer (release.ts, the current release's); the portable exe is the other option.
    downloadMB: release.installer!.sizeMB!,
    download: "the installer",
    sources: {
      memoryMB: `${V080}csv/${RUNS}-playing.csv`,
      memoryPausedMB: `${V080}csv/${RUNS}-paused.csv`,
      cpuPlayingPct: `${V080}csv/${RUNS}-playing.csv`,
      cpuPausedPct: `${V080}csv/${RUNS}-paused.csv`,
      processes: `${V080}csv/${RUNS}-playing.csv`,
      startToSoundS: `${V080}csv/start-gpu-to-sound.csv`,
      clickToSoundS: `${V080}csv/clicks-gpu-{A,B}.csv`,
      downloadMB: `The v${release.version} release's files`,
    },
  },
  {
    app: "Pear Desktop",
    version: "3.12.0",
    memoryMB: 798,
    memoryPausedMB: 782,
    cpuPlayingPct: 0.17,
    cpuPausedPct: 0.05,
    processes: { min: 6, max: 7 },
    startToSoundS: null,
    clickToSoundS: null,
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
    startToSoundS: null,
    clickToSoundS: null,
    downloadMB: 0,
    download: "nothing, Edge comes with Windows",
    sources: runs("edge_final-playing3_2026-11-01_1946.csv", "edge_final-paused_2026-11-01_1952.csv"),
  },
];

export const figureFor = (app: App) => figures.find((f) => f.app === app)!;

/** A figure as one number: a range of processes by its upper end; null where it wasn't
 *  part of the measurement. */
export const valueOf = (f: Figure, m: Metric): number | null => (m === "processes" ? f.processes.max : f[m]);

/** How many times less featherplay uses than `app`, to the nearest whole time. */
export const timesLess = (m: Exclude<NumericMetric, Timing>, app: App): number =>
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
// Setup.e32 it's made from is 4,427,264) and its small unins000.dat log, and since 0.1.2 the
// Visual C++ runtime the exe links (packaging/windows/build-installer.sh: msvcp140.dll,
// vcruntime140.dll and vcruntime140_1.dll: 872,240 bytes from MSVC 14.51's redistributable,
// recorded in installer.md; the release's own come from the build machine's tools and differ
// a little). Not listed byte for byte, so the installed total is shown as "about".
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
    /** The installed folder: the exe, the uninstaller and the Visual C++ runtime (approximate). */
    installed: { uninstallerBytes: 4_427_264, runtimeBytes: 872_240, source: "installer.md" },
  },
};
