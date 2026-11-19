// The download: version 0.5.0, the files on the v0.5.0 release (built by release.yml on
// 18 November 2026). Each file links straight to its download; the release page has the
// checksums and notes. featherplay's memory, CPU and timing figures (numbers.ts) are 0.4.0's,
// measured on 14 November on a release build of 0.4.0's code made on the measuring PC (0.4.1,
// 0.4.2 and 0.5.0 weren't measured again).
//
// Two ways to get it: the per-user installer (recommended: no admin prompt, a Start menu
// entry, an uninstaller that removes everything) and the portable exe. A file's bytes are
// null until it's measured, shown as a dash. Set a file to null to leave it out; the
// download card (Download.astro) and every size on the page read from here.
//
// The experimental Linux build (from 0.3.0): an AppImage and a .tar.gz for x86_64 and for
// aarch64 (release.yml's build-linux). The card offers the x86_64 AppImage and points to the
// release page for the rest; the first screen's Download gets the aarch64 AppImage where the
// page can tell it runs on one. Its links read `version` as the Windows files' do.

import { links } from "./links";

export type ReleaseFile = {
  /** The file's name on the release page. */
  name: string;
  /** Null until the file exists and is measured. */
  bytes: number | null;
  /** The same in MB as Windows counts them (1,048,576 bytes), to two decimals. */
  sizeMB: number | null;
  url: string;
};

export type Release = {
  version: string;
  /** The release page, for every file and the checksums. */
  url: string;
  installer: ReleaseFile | null;
  portable: ReleaseFile | null;
  /** The experimental Linux build, or null to leave it out. */
  linux: LinuxBuild | null;
  /** The size of the first option offered whose size is known (for search engines). */
  sizeMB: number;
};

export type LinuxBuild = {
  /** The one the card offers directly: the x86_64 AppImage. */
  appImage: ReleaseFile;
  /** The aarch64 AppImage, or null if there is none. */
  appImageArm: ReleaseFile | null;
  /** Every Linux file on the release, the AppImage first. */
  files: ReleaseFile[];
  /** This version's release page, with the .tar.gz and the aarch64 files. */
  page: string;
};

const releases = links.releases;
const version = "0.5.0";
/** Bytes as MB the way Windows counts them (1,048,576 bytes), to two decimals. */
export const toMB = (bytes: number): number => Math.round((bytes / 1_048_576) * 100) / 100;
const file = (name: string, bytes: number | null): ReleaseFile => ({
  name,
  bytes,
  sizeMB: bytes == null ? null : toMB(bytes),
  url: `${releases}/download/v${version}/${name}`,
});
const installer: ReleaseFile | null = file(`featherplay-setup-${version}.exe`, 11_356_089);
const portable: ReleaseFile | null = file("featherplay.exe", 30_928_384);
const linuxFiles = [
  file("featherplay-x86_64.AppImage", 15_444_472),
  file("featherplay-x86_64.tar.gz", 15_569_340),
  file("featherplay-aarch64.AppImage", 15_403_528),
  file("featherplay-aarch64.tar.gz", 15_292_877),
];
const linux: LinuxBuild | null = { appImage: linuxFiles[0], appImageArm: linuxFiles[2], files: linuxFiles, page: `${releases}/tag/v${version}` };

export const release: Release = {
  version,
  url: releases,
  installer,
  portable,
  linux,
  sizeMB: [installer, portable].find((f) => f?.sizeMB != null)?.sizeMB ?? 0,
};

/** A file's size as the card shows it: "10.13 MB", or a dash while it isn't known. */
export const sizeLabel = (f: ReleaseFile): string => (f.sizeMB == null ? "\u2013" : `${f.sizeMB.toFixed(2)} MB`);

export type DownloadOption = {
  id: "installer" | "portable";
  title: string;
  note: string;
  action: string;
  file: ReleaseFile;
  /** The one to press: the installer when there is one. */
  primary: boolean;
};

/** What the download card offers, the recommended one first. */
export function downloadOptions(r: Release = release): DownloadOption[] {
  const out: DownloadOption[] = [];
  if (r.installer)
    out.push({
      id: "installer",
      title: "Installer",
      note: "No admin prompt, and it brings the Visual C++ runtime it needs. Adds featherplay to Start, and its uninstaller leaves nothing behind.",
      action: "Download the installer",
      file: r.installer,
      primary: true,
    });
  if (r.portable)
    out.push({
      id: "portable",
      title: "Portable exe",
      note: "One file, nothing to install. Keep it anywhere and run it. It needs the Visual C++ runtime, which most PCs have.",
      action: "Download the exe",
      file: r.portable,
      primary: out.length === 0,
    });
  return out;
}

/** A system the first screen's Download can tell (src/scripts/boot.js sets it on the root as
 *  data-os before the first paint). Anything else (a Mac, a phone, no script) is sent to the
 *  Download section. */
export type HeroOs = "windows" | "linux" | "linux-arm";

export type HeroPick = {
  os: HeroOs;
  file: ReleaseFile;
  /** The other system there is a build for, named under the button ("Also for Linux"). */
  also: "Windows" | "Linux" | null;
};

/** What the first screen's Download gets on each system it can tell: the installer (else the
 *  exe) on Windows, the AppImage for the machine on Linux. A system with no file isn't listed. */
export function heroPicks(r: Release = release): HeroPick[] {
  const win = r.installer ?? r.portable;
  const out: HeroPick[] = [];
  if (win) out.push({ os: "windows", file: win, also: r.linux ? "Linux" : null });
  if (r.linux) out.push({ os: "linux", file: r.linux.appImage, also: win ? "Windows" : null });
  if (r.linux?.appImageArm) out.push({ os: "linux-arm", file: r.linux.appImageArm, also: win ? "Windows" : null });
  return out;
}
