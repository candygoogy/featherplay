// The download: version 0.4.0, the files on the v0.4.0 release (built by release.yml on
// 14 November 2026). Each file links straight to its download; the release page has the
// checksums and notes. featherplay's memory, CPU and timing figures (numbers.ts) were
// measured on 14 November, on a release build of the same code made on the measuring PC.
//
// Two ways to get it: the per-user installer (recommended: no admin prompt, a Start menu
// entry, an uninstaller that removes everything) and the portable exe. A file's bytes are
// null until it's measured, shown as a dash. Set a file to null to leave it out; the
// download card (Download.astro) and every size on the page read from here.
//
// The experimental Linux build (from 0.3.0): an AppImage and a .tar.gz for x86_64 and for
// aarch64 (release.yml's build-linux). The card offers the x86_64 AppImage and points to the
// release page for the rest. Its links read `version` as the Windows files' do.

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
  /** Every Linux file on the release, the AppImage first. */
  files: ReleaseFile[];
  /** This version's release page, with the .tar.gz and the aarch64 files. */
  page: string;
};

const releases = links.releases;
const version = "0.4.0";
/** Bytes as MB the way Windows counts them (1,048,576 bytes), to two decimals. */
export const toMB = (bytes: number): number => Math.round((bytes / 1_048_576) * 100) / 100;
const file = (name: string, bytes: number | null): ReleaseFile => ({
  name,
  bytes,
  sizeMB: bytes == null ? null : toMB(bytes),
  url: `${releases}/download/v${version}/${name}`,
});
const installer: ReleaseFile | null = file(`featherplay-setup-${version}.exe`, 11_659_916);
const portable: ReleaseFile | null = file("featherplay.exe", 31_679_488);
const linuxFiles = [
  file("featherplay-x86_64.AppImage", 15_260_152),
  file("featherplay-x86_64.tar.gz", 15_356_446),
  file("featherplay-aarch64.AppImage", 15_227_400),
  file("featherplay-aarch64.tar.gz", 15_102_411),
];
const linux: LinuxBuild | null = { appImage: linuxFiles[0], files: linuxFiles, page: `${releases}/tag/v${version}` };

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
      action: out.length ? "Download the exe" : "Download for Windows",
      file: r.portable,
      primary: out.length === 0,
    });
  return out;
}
