import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { songs } from "../../src/data/songs";
import { figures, metrics, timesLess } from "../../src/data/numbers";
import { figureText } from "../../src/data/format";
import { release } from "../../src/data/release";
import { contrast, lyricInks, mix } from "../../src/data/colour";

const publicDir = new URL("../../public/", import.meta.url);
const banned = /youtube|ytm|google/i;

describe("songs", () => {
  it("has the six free-licence tracks, in order, with unique ids", () => {
    expect(songs.map((s) => s.id)).toEqual([
      "california-lullabye",
      "invisible-light",
      "she-dreams-in-blue",
      "watching-seabirds-soar",
      "origami",
      "in-this-moment",
    ]);
    expect(new Set(songs.map((s) => s.id)).size).toBe(6);
  });

  it("every song has ascending lyric times inside its duration", () => {
    for (const s of songs) {
      let last = -1;
      for (const l of s.lyrics) {
        expect(l.atMs, `${s.title}: "${l.line}"`).toBeGreaterThan(last);
        expect(l.atMs, `${s.title}: "${l.line}"`).toBeLessThan(s.durationMs);
        expect(l.line.trim()).not.toBe("");
        last = l.atMs;
      }
    }
  });

  it("the three Josh Woodward songs have synced lyrics (7 to 16 lines); the rest show the no-lyrics layout", () => {
    const withLyrics = songs.filter((s) => s.lyrics.length > 0).map((s) => s.id);
    expect(withLyrics).toEqual(["california-lullabye", "invisible-light", "she-dreams-in-blue"]);
    for (const s of songs.filter((s) => s.lyrics.length > 0)) {
      expect(s.artist).toBe("Josh Woodward");
      expect(s.lyrics.length, s.title).toBeGreaterThanOrEqual(7);
      expect(s.lyrics.length, s.title).toBeLessThanOrEqual(16);
    }
  });

  it("every song has a clip in both encodings under public/music/", () => {
    for (const s of songs) {
      expect(s.clip, s.title).toBeDefined();
      expect(s.clip!.opus).toBe(`/music/${s.id}.webm`);
      expect(s.clip!.m4a).toBe(`/music/${s.id}.m4a`);
      for (const f of [s.clip!.opus, s.clip!.m4a]) expect(existsSync(new URL(f.slice(1), publicDir)), f).toBe(true);
    }
  });

  it("every song is credited under CC BY (commercial use, no NC, ND or SA), with links", () => {
    for (const s of songs) {
      const c = s.credit!;
      expect(c, s.title).toBeDefined();
      expect(c.licence).toMatch(/^CC BY [34]\.0$/);
      expect(c.licenceUrl).toMatch(/^https:\/\/creativecommons\.org\/licenses\/by\/[34]\.0\/$/);
      expect(c.sourceUrl).toMatch(/^https:\/\//);
      expect(c.site.url).toMatch(/^https:\/\//);
      expect(c.site.label).not.toBe("");
    }
  });

  it("public/ holds only the web clips: no raw downloads, no unused lyrics", () => {
    const all = readdirSync(publicDir, { recursive: true }).map(String);
    expect(all.filter((f) => /raw|unused|novox|\.mp3$|\.flac$|\.wav$/i.test(f))).toEqual([]);
    const music = readdirSync(new URL("music/", publicDir)).sort();
    expect(music).toEqual(songs.flatMap((s) => [`${s.id}.m4a`, `${s.id}.webm`]).sort());
  });

  it("every cover colour gives AA contrast under the glow in both themes", () => {
    for (const s of songs) {
      // White text over the colour mixed 60 % into the dark ground...
      expect(contrast("#FFFFFF", mix(s.colour, "#0F0F0F", 0.6)), `${s.title} dark`).toBeGreaterThanOrEqual(4.5);
      // ...and the dark ink over the colour mixed 60 % into white.
      expect(contrast("#0F0F0F", mix(s.colour, "#FFFFFF", 0.6)), `${s.title} light`).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("the colours are distinct from each other", () => {
    const hex = songs.map((s) => s.colour.toLowerCase());
    expect(new Set(hex).size).toBe(hex.length);
  });

  it("every cover is a webp in public/, at most 25 KB", () => {
    for (const s of songs) {
      expect(s.cover).toMatch(/^\/covers\/[\w-]+\.webp$/);
      const file = new URL(s.cover.slice(1), publicDir);
      expect(existsSync(file), s.cover).toBe(true);
      expect(statSync(file).size, s.cover).toBeLessThanOrEqual(25 * 1024);
      const head = readFileSync(file).subarray(0, 12).toString("latin1");
      expect(head.startsWith("RIFF") && head.endsWith("WEBP"), s.cover).toBe(true);
    }
  });

  it('no title, artist or line contains "YouTube", "YTM" or "Google"', () => {
    for (const s of songs) {
      expect(s.title).not.toMatch(banned);
      expect(s.artist).not.toMatch(banned);
      for (const l of s.lyrics) expect(l.line).not.toMatch(banned);
    }
  });
});

describe("figures", () => {
  it("covers featherplay, Pear Desktop and YouTube Music in Edge", () => {
    expect(figures.map((f) => f.app)).toEqual(["featherplay", "Pear Desktop", "YouTube Music in Edge"]);
  });

  it("holds featherplay 0.3.0's measurement of 13 November 2026 and the others' of 1 November, nothing provisional", () => {
    const by = Object.fromEntries(figures.map((f) => [f.app, f]));
    expect(by["featherplay"]).toMatchObject({
      version: "0.3.0",
      memoryMB: 42,
      memoryPausedMB: 41,
      cpuPlayingPct: 0.061,
      cpuPausedPct: 0.003,
      processes: { min: 1, max: 1 },
      startToSoundS: 0.228,
      clickToSoundS: 0.024,
      downloadMB: 10.78,
    });
    expect(by["Pear Desktop"]).toMatchObject({
      version: "3.12.0",
      memoryMB: 798,
      memoryPausedMB: 782,
      cpuPlayingPct: 0.17,
      cpuPausedPct: 0.05,
      processes: { min: 6, max: 7 },
      downloadMB: 274,
    });
    expect(by["YouTube Music in Edge"]).toMatchObject({
      version: "156.0.4314.58",
      memoryMB: 988,
      memoryPausedMB: 973,
      cpuPlayingPct: 0.14,
      cpuPausedPct: 0.018,
      processes: { min: 9, max: 11 },
      downloadMB: 0,
    });
    // The two timings are featherplay's alone; everything else is there for all three.
    for (const f of figures) {
      expect(f, f.app).not.toHaveProperty("provisional");
      for (const m of metrics) {
        if (f.app !== "featherplay" && (m === "startToSoundS" || m === "clickToSoundS")) expect(f[m], `${f.app} ${m}`).toBeNull();
        else expect(f[m], `${f.app} ${m}`).not.toBeNull();
      }
    }
  });

  it("every figure points at the files it was measured in, or at none where it wasn't measured", () => {
    for (const f of figures) {
      for (const m of metrics) {
        if (f[m as keyof typeof f] === null) expect(f.sources[m], `${f.app} ${m}`).toBeNull();
        else expect(f.sources[m], `${f.app} ${m}`).toMatch(/^measurements\/[^/]*final/);
      }
    }
    expect(figures[0].sources.memoryMB).toMatch(/final-0\.3\.0\/csv\/default-/);
    expect(figures[0].sources.clickToSoundS).toMatch(/click-to-sound/);
    expect(figures[2].sources.cpuPlayingPct).toMatch(/edge_final-playing3_/);
  });

  it("featherplay uses less memory and CPU than either", () => {
    const [fp, ...others] = figures;
    for (const o of others) {
      expect(fp.memoryMB).toBeLessThan(o.memoryMB);
      expect(fp.cpuPlayingPct).toBeLessThan(o.cpuPlayingPct);
      expect(fp.cpuPausedPct).toBeLessThan(o.cpuPausedPct);
    }
  });

  it("works out how many times less featherplay uses, from the figures", () => {
    expect(timesLess("memoryMB", "Pear Desktop")).toBe(19);
    expect(timesLess("memoryMB", "YouTube Music in Edge")).toBe(24);
    expect(timesLess("cpuPlayingPct", "Pear Desktop")).toBe(3);
    expect(timesLess("cpuPlayingPct", "YouTube Music in Edge")).toBe(2);
    expect(timesLess("downloadMB", "Pear Desktop")).toBe(25);
  });

  it("reads each figure the way the page shows it", () => {
    const [fp, pear, edge] = figures;
    expect(figureText(fp, "memoryMB")).toBe("42 MB");
    expect(figureText(fp, "cpuPlayingPct")).toBe("0.061%");
    expect(figureText(fp, "cpuPausedPct")).toBe("0.003%");
    expect(figureText(pear, "cpuPausedPct")).toBe("0.05%");
    expect(figureText(fp, "startToSoundS")).toBe("0.23 s");
    expect(figureText(fp, "clickToSoundS")).toBe("0.024 s");
    expect(figureText(pear, "startToSoundS")).toBe("—");
    expect(figureText(edge, "clickToSoundS")).toBe("—");
    expect(figureText(fp, "downloadMB")).toBe("10.78 MB");
    expect(figureText(pear, "downloadMB")).toBe("274 MB");
    expect(figureText(edge, "downloadMB")).toBe("None");
    expect(figureText(fp, "processes")).toBe("1");
    expect(figureText(pear, "processes")).toBe("6 to 7");
    expect(figureText(edge, "processes")).toBe("9 to 11");
  });
});

describe("release", () => {
  it("has a version, a size and a link", () => {
    expect(release.version).toMatch(/^\d+\.\d+\.\d+/);
    expect(release.sizeMB).toBeGreaterThan(0);
    expect(release.url).toMatch(/^https:\/\//);
  });
  it("lists the four Linux files release.yml builds, at this version, the x86_64 AppImage first", () => {
    const linux = release.linux!;
    expect(linux.files.map((f) => f.name)).toEqual([
      "featherplay-x86_64.AppImage",
      "featherplay-x86_64.tar.gz",
      "featherplay-aarch64.AppImage",
      "featherplay-aarch64.tar.gz",
    ]);
    expect(linux.appImage).toBe(linux.files[0]);
    for (const f of linux.files)
      expect(f.url).toBe(`https://github.com/candygoogy/featherplay/releases/download/v${release.version}/${f.name}`);
    expect(linux.page).toBe(`https://github.com/candygoogy/featherplay/releases/tag/v${release.version}`);
  });
});

describe("colour helpers", () => {
  it("gives every song's lyrics panel the app's three levels: playing white, to come 4.5:1, sung 3:1", () => {
    for (const s of songs) {
      const ink = lyricInks(s.colour);
      // As the app's colour::lyric_panel: deep enough for white to keep 8:1.
      expect(contrast("#FFFFFF", ink.ground), s.title).toBeGreaterThanOrEqual(8);
      const next = contrast(ink.next, ink.ground);
      const past = contrast(ink.past, ink.ground);
      expect(next, s.title).toBeGreaterThanOrEqual(4.5);
      expect(next, s.title).toBeLessThan(5.2);
      expect(past, s.title).toBeGreaterThanOrEqual(3);
      expect(past, s.title).toBeLessThan(3.6);
      // Drawn as white at that alpha over the panel.
      for (const [css, drawn] of [[ink.nextCss, ink.next], [ink.pastCss, ink.past]]) {
        const a = Number(css.match(/^rgb\(255 255 255 \/ ([\d.]+)\)$/)![1]);
        expect(mix("#FFFFFF", ink.ground, a), s.title).toBe(drawn);
      }
      // Still the cover's colour: the usual 60 % mix when that's already deep enough.
      if (contrast("#FFFFFF", mix(s.colour, "#0F0F0F", 0.6)) >= 8) expect(ink.ground).toBe(mix(s.colour, "#0F0F0F", 0.6));
    }
  });
  it("measures contrast like WCAG", () => {
    expect(contrast("#FFFFFF", "#000000")).toBeCloseTo(21, 1);
    expect(contrast("#777777", "#FFFFFF")).toBeCloseTo(4.48, 1);
  });
  it("mixes in sRGB like color-mix()", () => {
    expect(mix("#FF0000", "#0000FF", 0.5)).toBe("#800080");
  });
});

import { zoomTo } from "../../src/demo/features";
describe("the features' zoom", () => {
  const parse = (t: string) => t.match(/-?[\d.]+/g)!.map(Number);
  it("brings a point to the middle when it can", () => {
    const [tx, ty, z] = parse(zoomTo(1000, 600, 500, 300, 1.5));
    expect(tx + z * 500).toBeCloseTo(500, 0);
    expect(ty + z * 300).toBeCloseTo(300, 0);
  });
  it("never shows past the laptop's edges, even for a corner", () => {
    const [tx, ty, z] = parse(zoomTo(1000, 600, 980, 590, 2));
    expect(tx).toBeGreaterThanOrEqual(1000 * (1 - z) - 0.1);
    expect(ty).toBeGreaterThanOrEqual(600 * (1 - z) - 0.1);
    expect(tx).toBeLessThanOrEqual(0);
  });
});

import { downloadOptions } from "../../src/data/release";
describe("the download's options", () => {
  const file = { name: "f", bytes: 10_485_760, sizeMB: 10, url: "https://example.org/" };
  const base = { version: "1.0.0", url: "https://example.org/", sizeMB: 10, linux: null };
  it("recommends the installer when there is one, with the exe beside it", () => {
    const o = downloadOptions({ ...base, installer: file, portable: file });
    expect(o.map((x) => [x.id, x.primary])).toEqual([
      ["installer", true],
      ["portable", false],
    ]);
  });
  it("makes the exe the one button when it's the only file", () => {
    const o = downloadOptions({ ...base, installer: null, portable: file });
    expect(o).toHaveLength(1);
    expect(o[0]).toMatchObject({ id: "portable", primary: true, action: "Download for Windows" });
  });
  it("offers the per-user installer and the exe by name, with the v0.3.0 release's sizes and direct links", async () => {
    const { release, sizeLabel } = await import("../../src/data/release");
    expect(release.installer?.name).toBe(`featherplay-setup-${release.version}.exe`);
    expect(release.version).toBe("0.3.0");
    expect(release.installer?.bytes).toBe(11_303_044);
    expect(release.portable?.bytes).toBe(30_142_976);
    expect(release.installer?.url).toBe(
      "https://github.com/candygoogy/featherplay/releases/download/v0.3.0/featherplay-setup-0.3.0.exe",
    );
    expect(release.portable?.url).toBe("https://github.com/candygoogy/featherplay/releases/download/v0.3.0/featherplay.exe");
    // MB as Windows counts them (1,048,576 bytes).
    expect(release.installer && sizeLabel(release.installer)).toBe("10.78 MB");
    expect(release.portable && sizeLabel(release.portable)).toBe("28.75 MB");
    // Search engines get the first size that's known: the installer's.
    expect(release.sizeMB).toBe(10.78);
  });
  it("shows a dash for a file not measured yet", async () => {
    const { sizeLabel } = await import("../../src/data/release");
    expect(sizeLabel({ name: "f", bytes: null, sizeMB: null, url: "https://example.org/" })).toBe("\u2013");
  });
  it("the comparison's download figure is the installer's size, read from the release", async () => {
    const { release } = await import("../../src/data/release");
    expect(figures[0].downloadMB).toBe(release.installer!.sizeMB);
  });
  it("can offer the installer alone", () => {
    expect(downloadOptions({ ...base, installer: file, portable: null }).map((x) => x.id)).toEqual(["installer"]);
  });
});

describe("what featherplay leaves out", () => {
  it("ties Pear's entries to Chromium, Electron or video, and the groups add up", async () => {
    const { installFolders, leftOutTotals } = await import("../../src/data/numbers");
    const by = (n: string) => installFolders.pear.entries.find((e) => e.name === n)!;
    expect(by("chrome_100_percent.pak").group).toBe("chromium");
    expect(by("libGLESv2.dll").group).toBe("chromium");
    expect(by("resources").group).toBe("electron");
    expect(by("LICENSE.electron.txt").group).toBe("electron");
    expect(by("ffmpeg.dll").group).toBe("video");
    expect(by("vk_swiftshader.dll").group).toBe("video");
    expect(by("Uninstall YouTube Music.exe").group).toBeUndefined();
    const sum = (["chromium", "electron", "video"] as const).reduce((n, g) => n + leftOutTotals(g).bytes, 0);
    expect(sum + by("Uninstall YouTube Music.exe").bytes).toBe(installFolders.pear.bytes);
  });
});
