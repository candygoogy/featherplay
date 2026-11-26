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

  it("holds featherplay 0.7.0's candidate's measurement of 26 November 2026 and the others' of 1 November, nothing provisional", () => {
    const by = Object.fromEntries(figures.map((f) => [f.app, f]));
    expect(by["featherplay"]).toMatchObject({
      version: "0.7.0",
      memoryMB: 29,
      memoryPausedMB: 29,
      cpuPlayingPct: 0.058,
      cpuPausedPct: 0.002,
      processes: { min: 1, max: 1 },
      startToSoundS: 0.234,
      clickToSoundS: 0.071,
      downloadMB: 10.92,
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
        // featherplay's download is the current release's file, not a measurement.
        else if (f.app === "featherplay" && m === "downloadMB") expect(f.sources[m]).toBe("The v0.6.2 release's files");
        else expect(f.sources[m], `${f.app} ${m}`).toMatch(/^measurements\/[^/]*final/);
      }
    }
    expect(figures[0].sources.memoryMB).toMatch(/final-rc-1126\/csv\/default-/);
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
    expect(timesLess("memoryMB", "Pear Desktop")).toBe(28);
    expect(timesLess("memoryMB", "YouTube Music in Edge")).toBe(34);
    expect(timesLess("cpuPlayingPct", "Pear Desktop")).toBe(3);
    expect(timesLess("cpuPlayingPct", "YouTube Music in Edge")).toBe(2);
    expect(timesLess("downloadMB", "Pear Desktop")).toBe(25);
  });

  it("reads each figure the way the page shows it", () => {
    const [fp, pear, edge] = figures;
    expect(figureText(fp, "memoryMB")).toBe("29 MB");
    expect(figureText(fp, "cpuPlayingPct")).toBe("0.058%");
    expect(figureText(fp, "cpuPausedPct")).toBe("0.002%");
    expect(figureText(pear, "cpuPausedPct")).toBe("0.05%");
    expect(figureText(fp, "startToSoundS")).toBe("0.23 s");
    expect(figureText(fp, "clickToSoundS")).toBe("0.071 s");
    expect(figureText(pear, "startToSoundS")).toBe("—");
    expect(figureText(edge, "clickToSoundS")).toBe("—");
    expect(figureText(fp, "downloadMB")).toBe("10.92 MB");
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
    expect(linux.appImageArm).toBe(linux.files[2]);
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

import { downloadOptions, heroPicks } from "../../src/data/release";
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
    expect(o[0]).toMatchObject({ id: "portable", primary: true, action: "Download the exe" });
  });
  it("offers the per-user installer and the exe by name, with the v0.6.2 release's sizes and direct links", async () => {
    const { release, sizeLabel } = await import("../../src/data/release");
    expect(release.installer?.name).toBe(`featherplay-setup-${release.version}.exe`);
    expect(release.version).toBe("0.6.2");
    expect(release.installer?.bytes).toBe(11_448_302);
    expect(release.portable?.bytes).toBe(31_322_624);
    expect(release.installer?.url).toBe(
      "https://github.com/candygoogy/featherplay/releases/download/v0.6.2/featherplay-setup-0.6.2.exe",
    );
    expect(release.portable?.url).toBe("https://github.com/candygoogy/featherplay/releases/download/v0.6.2/featherplay.exe");
    // MB as Windows counts them (1,048,576 bytes).
    expect(release.installer && sizeLabel(release.installer)).toBe("10.92 MB");
    expect(release.portable && sizeLabel(release.portable)).toBe("29.87 MB");
    // Search engines get the first size that's known: the installer's.
    expect(release.sizeMB).toBe(10.92);
  });
  it("shows a dash for a file not measured yet", async () => {
    const { sizeLabel } = await import("../../src/data/release");
    expect(sizeLabel({ name: "f", bytes: null, sizeMB: null, url: "https://example.org/" })).toBe("\u2013");
  });
  it("the comparison's download figure is the installer's size, read from the release", async () => {
    const { release } = await import("../../src/data/release");
    expect(figures[0].downloadMB).toBe(release.installer!.sizeMB);
  });
  it("the first screen's Download: the installer on Windows, each machine's AppImage on Linux, each naming the other system", () => {
    const linux = { appImage: { ...file, name: "x64" }, appImageArm: { ...file, name: "arm" }, files: [], page: "https://example.org/" };
    const picks = heroPicks({ ...base, installer: { ...file, name: "setup" }, portable: file, linux });
    expect(picks.map((p) => [p.os, p.file.name, p.also])).toEqual([
      ["windows", "setup", "Linux"],
      ["linux", "x64", "Windows"],
      ["linux-arm", "arm", "Windows"],
    ]);
    // No aarch64 build: an Arm machine goes to the section; no Linux build: Windows alone.
    expect(heroPicks({ ...base, installer: null, portable: file, linux: { ...linux, appImageArm: null } }).map((p) => p.os)).toEqual(["windows", "linux"]);
    expect(heroPicks({ ...base, installer: null, portable: file })).toEqual([{ os: "windows", file, also: null }]);
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

describe("Now playing's fields (src/styles/fields.css, scripts/fields.mjs)", () => {
  const css = readFileSync(new URL("../../src/styles/fields.css", import.meta.url), "utf8");
  const rule = (id: string) => {
    const body = css.split(`[data-song="${id}"] {`)[1]?.split("}")[0] ?? "";
    return Object.fromEntries([...body.matchAll(/--(f[dl]-[a-z0-9]+): (#[0-9a-f]{6})/g)].map((m) => [m[1], m[2]]));
  };
  it("has every song's, and its lyrics' inks keep the app's floors on each band, both themes", () => {
    for (const s of songs) {
      const f = rule(s.id);
      for (const t of ["fd", "fl"]) {
        for (const band of ["top", "mid", "low"]) {
          const ground = f[`${t}-${band}`];
          expect(ground, `${s.id} ${t}-${band}`).toMatch(/^#/);
          expect(contrast(f[`${t}-ink1`], ground), `${s.id} ${t} playing on ${band}`).toBeGreaterThanOrEqual(7);
          expect(contrast(f[`${t}-ink2`], ground), `${s.id} ${t} to come on ${band}`).toBeGreaterThanOrEqual(4.5);
          expect(contrast(f[`${t}-ink3`], ground), `${s.id} ${t} sung on ${band}`).toBeGreaterThanOrEqual(3);
        }
      }
    }
  });
  it("keeps Home's card readable: its colour has the app's floor, and its text keeps AA from the art's edge on", () => {
    // replica.css .band: the field's slice from 37 % to 63 % down, the card's colour (--f?-band,
    // the field's mean held to its floor) over it at 49 % where the text starts (28 px past the
    // art's edge) and whole from 476 px. Text white or #FFFFFFBF (dark), the first ink or
    // #000000B3 (light); a hovered line is white 8 % or black 5 % over it.
    const themes = [
      ["fd", "#ffffff", "#ffffff", 0xbf / 255, 0x14 / 255],
      ["fl", "", "#000000", 0xb3 / 255, 0x0d / 255],
    ] as const;
    for (const s of songs) {
      const f = rule(s.id);
      for (const [t, first, second, alpha, hover] of themes) {
        const band = f[`${t}-band`];
        expect(band, `${s.id} ${t}-band`).toMatch(/^#/);
        // A step above the Shortcut tiles (#212121 dark, #F2F2F2 light): lighter in dark, and
        // never the tiles' grey in light.
        expect(t === "fd" ? contrast(band, "#000000") > contrast("#212121", "#000000") : band !== "#f2f2f2", `${s.id} ${t} floor`).toBe(true);
        const ink1 = first || f["fl-ink1"];
        const stop = (b: string, share: number) => mix(f[`${t}-mid`], f[`${t}-${b}`], share);
        for (const field of [stop("top", 0.67), stop("mid", 1), stop("low", 0.82)]) {
          for (const ground of [mix(band, field, 0.49), band]) {
            const ink2 = mix(second, ground, alpha);
            const at = `${s.id} ${t} on ${ground}`;
            expect(contrast(ink1, ground), at).toBeGreaterThanOrEqual(7);
            expect(contrast(ink2, ground), at).toBeGreaterThanOrEqual(4.5);
            const hovered = mix(second, ground, hover);
            expect(contrast(mix(second, hovered, alpha), hovered), `${at}, hovered`).toBeGreaterThanOrEqual(4.5);
          }
        }
      }
    }
  });
  // replica.css --float-dim and --float-lift: the bar's and Now playing's Up next card's
  // field, dimmed toward black (dark) or lifted toward white (light).
  const float = [
    ["fd", "#000000", 0.3],
    ["fl", "#ffffff", 0.5],
  ] as const;
  /** The field at `y` (0 the window's top, 1 its foot), as replica.css paints it: three bands,
   *  the mid one 55 % down; then dimmed or lifted. */
  const fieldAt = (f: Record<string, string>, t: string, veil: string, by: number, y: number) => {
    const ground = y <= 0.55 ? mix(f[`${t}-top`], f[`${t}-mid`], 1 - y / 0.55) : mix(f[`${t}-mid`], f[`${t}-low`], 1 - (y - 0.55) / 0.45);
    return mix(ground, veil, 1 - by);
  };
  it("keeps the player bar on the field readable: text 4.5 hovered or not, icons 3 pressed, rails 3", () => {
    // replica.css .bar: the field's foot under a bar 10 px from the foot of a window 600 to
    // 852 px tall (from 0.8 down). The title white (dark) or ink1, the artist, times and icons
    // ink2, the rails ink3; hovered and pressed are the theme's overlays (tokens.css) over it.
    const overlays = { fd: ["#ffffff", 0x17 / 255, 0x33 / 255], fl: ["#000000", 0x12 / 255, 0x2b / 255] } as const;
    for (const s of songs) {
      const f = rule(s.id);
      for (const [t, veil, by] of float) {
        const [over, hover, press] = overlays[t];
        const title = t === "fd" ? "#ffffff" : f["fl-ink1"];
        for (let y = 0.8; y <= 1.0001; y += 0.02) {
          const ground = fieldAt(f, t, veil, by, y);
          const at = `${s.id} ${t} on ${ground}`;
          for (const [ink, what] of [
            [title, "title"],
            [f[`${t}-ink2`], "artist and times"],
          ]) {
            expect(contrast(ink, ground), `${at}: ${what}`).toBeGreaterThanOrEqual(4.5);
            expect(contrast(ink, mix(over, ground, hover)), `${at}: ${what}, hovered`).toBeGreaterThanOrEqual(4.5);
            expect(contrast(ink, mix(over, ground, press)), `${at}: ${what} as an icon, pressed`).toBeGreaterThanOrEqual(3);
          }
          expect(contrast(f[`${t}-ink3`], ground), `${at}: rails`).toBeGreaterThanOrEqual(3);
        }
      }
    }
  });
  it("keeps Now playing's Up next card on the field readable: its two inks 4.5 on a row, hovered and playing", () => {
    // replica.css .queue on Now playing: the field from under the title bar's row to above the
    // bar. A hovered row is --f-hover over it, the playing row --playing-row under that.
    const overlays = { fd: ["#ffffff", 0x1f / 255, 0x0f / 255], fl: ["#000000", 0x12 / 255, 0x0a / 255] } as const;
    for (const s of songs) {
      const f = rule(s.id);
      for (const [t, veil, by] of float) {
        const [over, hover, playing] = overlays[t];
        const title = t === "fd" ? "#ffffff" : f["fl-ink1"];
        for (let y = 0.05; y <= 0.9; y += 0.05) {
          const ground = fieldAt(f, t, veil, by, y);
          const at = `${s.id} ${t} on ${ground}`;
          for (const ink of [title, f[`${t}-ink2`]]) {
            expect(contrast(ink, ground), at).toBeGreaterThanOrEqual(4.5);
            expect(contrast(ink, mix(over, ground, hover)), `${at}, hovered`).toBeGreaterThanOrEqual(4.5);
            expect(contrast(ink, mix(over, mix(over, ground, playing), hover)), `${at}, playing and hovered`).toBeGreaterThanOrEqual(4.5);
          }
        }
      }
    }
  });
});
