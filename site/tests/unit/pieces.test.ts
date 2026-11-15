import { describe, expect, it } from "vitest";
import { createListen, mediaMetadataFor, scrobbleAfterMs, sponsorPlayhead, trayLines } from "../../src/demo/pieces";
import { songs } from "../../src/data/songs";

const song = songs[0]; // California Lullabye, 54.5 s

describe("the tray's tooltip", () => {
  it("names the app and nothing playing before a song is loaded", () => {
    expect(trayLines({ song: null, playing: false, positionMs: 0 })).toEqual(["featherplay", "Nothing playing"]);
  });
  it("gives the title, then the artist and where the song is", () => {
    expect(trayLines({ song, playing: true, positionMs: 20_600 })).toEqual(["California Lullabye", "Josh Woodward, 0:20 of 0:54"]);
  });
  it("says where a paused song waits", () => {
    expect(trayLines({ song, playing: false, positionMs: 20_600 })).toEqual(["California Lullabye", "Josh Woodward, paused at 0:20"]);
  });
});

describe("scrobbling", () => {
  it("counts half the song, four minutes at most", () => {
    expect(scrobbleAfterMs(54_500)).toBe(27_250);
    expect(scrobbleAfterMs(600_000)).toBe(240_000);
  });

  const run = (listen: ReturnType<typeof createListen>, from: number, to: number, playing = true, s = song) => {
    let last = listen.step({ song: s, playing, positionMs: from });
    for (let t = from + 250; t <= to; t += 250) last = listen.step({ song: s, playing, positionMs: t });
    return last;
  };

  it("counts only time heard while playing, not where the song is", () => {
    const listen = createListen();
    const r = run(listen, 20_000, 30_000);
    expect(r.heardMs).toBe(10_000);
    expect(r.progress).toBeCloseTo(10_000 / 27_250, 3);
    expect(r.scrobbled).toBe(false);
  });
  it("doesn't count a seek forward, nor time paused", () => {
    const listen = createListen();
    run(listen, 0, 5_000);
    listen.step({ song, playing: true, positionMs: 40_000 }); // a seek
    const r = run(listen, 40_000, 40_000, false);
    expect(r.heardMs).toBe(5_000);
  });
  it("scrobbles once, at half the song heard, and says so once", () => {
    const listen = createListen();
    const steps: boolean[] = [];
    let t = 0;
    listen.step({ song, playing: true, positionMs: 0 });
    while (t < 40_000) {
      t += 250;
      steps.push(listen.step({ song, playing: true, positionMs: t }).justScrobbled);
    }
    expect(steps.filter(Boolean)).toHaveLength(1);
    expect(listen.step({ song, playing: true, positionMs: t }).scrobbled).toBe(true);
  });
  it("starts again on another song, and hands back the one that went before if it was scrobbled", () => {
    const listen = createListen();
    run(listen, 0, 30_000);
    const r = listen.step({ song: songs[1], playing: true, positionMs: 0 });
    expect(r.scrobbled).toBe(false);
    expect(r.heardMs).toBe(0);
    expect(r.finished?.id).toBe(song.id);
    expect(listen.step({ song: songs[1], playing: true, positionMs: 250 }).finished).toBeNull();
  });
});

describe("SponsorBlock", () => {
  it("maps the song onto a video whose first 7 % and last 12 % are talking", () => {
    expect(sponsorPlayhead(0)).toBe(7);
    expect(sponsorPlayhead(1)).toBe(88);
    expect(sponsorPlayhead(0.5)).toBeCloseTo(47.5, 5);
  });
});

describe("the media session", () => {
  it("gives the song's title, artist, album and its cover from the site", () => {
    const m = mediaMetadataFor(song, "https://featherplay.candygoogy.dev");
    expect(m).toEqual({
      title: "California Lullabye",
      artist: "Josh Woodward",
      album: "The Beautiful Machine",
      artwork: [{ src: "https://featherplay.candygoogy.dev/covers/california-lullabye.webp", sizes: "600x600", type: "image/webp" }],
    });
  });
  it("leaves the album empty for a track without one", () => {
    expect(mediaMetadataFor(songs.find((s) => !s.album)!, "https://x").album).toBe("");
  });
});
