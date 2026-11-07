import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPlayer, currentLineIndex, formatTime, lyricsSource, SOUND_DRIFT_MS } from "../../src/demo/player";
import { songs } from "../../src/data/songs";

// A frame clock the tests drive: rAF becomes a 16 ms timer on vitest's fake clock.
function frames() {
  return {
    now: () => Date.now(),
    raf: (cb: (t: number) => void) => setTimeout(() => cb(Date.now()), 16) as unknown as number,
    caf: (id: number) => clearTimeout(id),
    visible: () => true,
  };
}

const withLyrics = songs.find((s) => s.lyrics.length > 0)!;
const instrumental = songs.find((s) => s.lyrics.length === 0)!;

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-10-31T21:00:00Z"));
});
afterEach(() => vi.useRealTimers());

describe("player", () => {
  it("starts with nothing playing on Home", () => {
    const p = createPlayer(songs, frames());
    expect(p.state.song).toBeNull();
    expect(p.state.playing).toBe(false);
    expect(p.state.view).toBe("home");
  });

  it("play sets the song and playing", () => {
    const p = createPlayer(songs, frames());
    p.play(withLyrics.id);
    expect(p.state.song?.id).toBe(withLyrics.id);
    expect(p.state.playing).toBe(true);
    expect(p.state.positionMs).toBe(0);
  });

  it("positionMs advances only while playing", () => {
    const p = createPlayer(songs, frames());
    p.play(withLyrics.id);
    vi.advanceTimersByTime(1000);
    p.pause();
    const after1s = p.state.positionMs;
    expect(after1s).toBeGreaterThan(900);
    expect(after1s).toBeLessThan(1100);
    vi.advanceTimersByTime(2000);
    expect(p.state.positionMs).toBe(after1s);
    p.resume();
    vi.advanceTimersByTime(500);
    expect(p.state.positionMs).toBeGreaterThan(after1s + 400);
  });

  it("the clock keeps to the sound while a clip is sounding, and runs alone when it isn't", () => {
    const p = createPlayer(songs, frames());
    let heard: number | null = null;
    p.followSound(() => heard);
    p.play(withLyrics.id);
    // Nothing sounding yet (the clip is loading): the clock runs on its own.
    vi.advanceTimersByTime(1500);
    expect(p.state.positionMs).toBeGreaterThan(1400);
    // The clip starts late, at 0: the clock goes back to it, and the line is the clip's.
    heard = 0;
    vi.advanceTimersByTime(20);
    expect(p.state.positionMs).toBeLessThan(SOUND_DRIFT_MS);
    // Small differences (frame timing) are left alone.
    heard = p.state.positionMs + SOUND_DRIFT_MS / 2;
    const before = p.state.positionMs;
    vi.advanceTimersByTime(16);
    expect(p.state.positionMs - before).toBeLessThan(40);
    // The sound stops answering: the clock carries on from where it was.
    heard = null;
    const at = p.state.positionMs;
    vi.advanceTimersByTime(1000);
    expect(p.state.positionMs).toBeGreaterThan(at + 900);
    // And following the sound never counts as a jump (the clip isn't sent back to itself).
    expect(p.state.jumps).toBe(1);
  });

  it("time stands still while the page is hidden", () => {
    let visible = true;
    const p = createPlayer(songs, { ...frames(), visible: () => visible });
    p.play(withLyrics.id);
    vi.advanceTimersByTime(500);
    visible = false;
    p.visibilityChanged();
    const before = p.state.positionMs;
    vi.advanceTimersByTime(3000);
    expect(p.state.positionMs).toBe(before);
    visible = true;
    p.visibilityChanged();
    vi.advanceTimersByTime(500);
    expect(p.state.positionMs).toBeGreaterThan(before + 400);
    expect(p.state.positionMs).toBeLessThan(before + 600);
  });

  it("seek moves the position, within the song", () => {
    const p = createPlayer(songs, frames());
    p.play(withLyrics.id);
    p.seek(30_000);
    expect(p.state.positionMs).toBe(30_000);
    p.seek(-5);
    expect(p.state.positionMs).toBe(0);
    p.seek(withLyrics.durationMs + 10_000);
    expect(p.state.positionMs).toBeLessThan(withLyrics.durationMs);
  });

  it("goes on to the next song at the end of one", () => {
    const p = createPlayer(songs, frames());
    p.play(songs[0].id);
    p.seek(songs[0].durationMs - 100);
    vi.advanceTimersByTime(300);
    expect(p.state.song?.id).toBe(songs[1].id);
    expect(p.state.playing).toBe(true);
  });

  it("next and previous move through the list", () => {
    const p = createPlayer(songs, frames());
    p.play(songs[0].id);
    p.next();
    expect(p.state.song?.id).toBe(songs[1].id);
    p.previous();
    expect(p.state.song?.id).toBe(songs[0].id);
  });

  it("search finds a song by its demo link and by title", () => {
    const p = createPlayer(songs, frames());
    expect(p.search(`https://music.example/watch?v=${withLyrics.id}`)?.id).toBe(withLyrics.id);
    expect(p.search("  seabirds ")?.title).toBe("Watching Seabirds Soar (Harmony)");
    expect(p.search("Buckley")?.title).toBe("Origami");
    expect(p.state.view).toBe("search");
    expect(p.search("no such song")).toBeNull();
    expect(p.state.query).toBe("no such song");
  });

  it("setLyrics(false) leaves the lyrics view", () => {
    const p = createPlayer(songs, frames());
    p.play(withLyrics.id);
    p.show("lyrics");
    expect(p.state.view).toBe("lyrics");
    p.setLyrics(false);
    expect(p.state.lyricsOn).toBe(false);
    expect(p.state.view).toBe("now");
    p.show("lyrics");
    expect(p.state.view).toBe("now");
  });

  it("minimise and restore keep the song", () => {
    const p = createPlayer(songs, frames());
    p.play(withLyrics.id);
    vi.advanceTimersByTime(1000);
    p.minimise();
    expect(p.state.minimised).toBe(true);
    expect(p.state.song?.id).toBe(withLyrics.id);
    expect(p.state.playing).toBe(true);
    p.restore();
    expect(p.state.minimised).toBe(false);
    expect(p.state.song?.id).toBe(withLyrics.id);
  });

  it("follows the page's theme until a theme is picked", () => {
    const p = createPlayer(songs, frames());
    expect(p.state.themeSet).toBe(false);
    p.setTheme("dark");
    expect(p.state.themeSet).toBe(true);
    expect(p.state.theme).toBe("dark");
    p.setTheme("light");
    expect(p.state.theme).toBe("light");
  });

  it("goes back to following the page with setTheme('page')", () => {
    const p = createPlayer(songs, frames());
    const seen: boolean[] = [];
    p.subscribe((s) => seen.push(s.themeSet));
    p.setTheme("light");
    p.setTheme("page");
    expect(p.state.themeSet).toBe(false);
    p.setTheme("page");
    expect(seen).toEqual([false, true, false]);
  });

  it("tells subscribers about changes, until they unsubscribe", () => {
    const p = createPlayer(songs, frames());
    const seen: string[] = [];
    const off = p.subscribe((s) => seen.push(s.view));
    p.show("now");
    off();
    p.show("home");
    expect(seen).toEqual(["home", "now"]);
  });

  it("can start from a given state (the static hero)", () => {
    const p = createPlayer(songs, { ...frames(), initial: { songId: withLyrics.id, positionMs: 40_000, view: "lyrics", playing: false } });
    expect(p.state.song?.id).toBe(withLyrics.id);
    expect(p.state.positionMs).toBe(40_000);
    expect(p.state.view).toBe("lyrics");
    expect(p.state.playing).toBe(false);
  });
});

describe("helpers", () => {
  it("finds the current lyric line", () => {
    const l = withLyrics.lyrics;
    expect(currentLineIndex(withLyrics, 0)).toBe(-1);
    expect(currentLineIndex(withLyrics, l[0].atMs)).toBe(0);
    expect(currentLineIndex(withLyrics, l[2].atMs + 10)).toBe(2);
    expect(currentLineIndex(withLyrics, withLyrics.durationMs - 1)).toBe(l.length - 1);
    expect(currentLineIndex(instrumental, 30_000)).toBe(-1);
  });

  it("formats times like the app", () => {
    expect(formatTime(0)).toBe("0:00");
    expect(formatTime(61_500)).toBe("1:01");
    expect(formatTime(221_000)).toBe("3:41");
  });
});

describe("the app's controls in the store", () => {
  it("like toggles for the current song and clears a Not for me", () => {
    const p = createPlayer(songs, frames());
    p.play(songs[0].id);
    p.toggleLike();
    expect(p.state.liked).toEqual([songs[0].id]);
    p.toggleLike();
    expect(p.state.liked).toEqual([]);
  });

  it("Not for me marks the song, skips it, and Undo brings it back where it was", () => {
    const p = createPlayer(songs, frames());
    p.play(songs[0].id);
    p.seek(30_000);
    p.toggleLike();
    p.notForMe();
    expect(p.state.disliked).toEqual([songs[0].id]);
    expect(p.state.liked).toEqual([]);
    expect(p.state.song?.id).toBe(songs[1].id);
    expect(p.state.toast?.text).toBe("Marked as not for me");
    p.undo();
    expect(p.state.song?.id).toBe(songs[0].id);
    expect(p.state.positionMs).toBe(30_000);
    expect(p.state.disliked).toEqual([]);
    expect(p.state.toast).toBeNull();
  });

  it("repeat cycles off, all, one; repeat one plays the song again at its end", () => {
    const p = createPlayer(songs, frames());
    expect(p.state.repeat).toBe("off");
    p.cycleRepeat();
    expect(p.state.repeat).toBe("all");
    p.cycleRepeat();
    expect(p.state.repeat).toBe("one");
    p.play(songs[2].id);
    p.seek(songs[2].durationMs - 100);
    vi.advanceTimersByTime(300);
    expect(p.state.song?.id).toBe(songs[2].id);
    expect(p.state.positionMs).toBeLessThan(1000);
    p.cycleRepeat();
    expect(p.state.repeat).toBe("off");
  });

  it("shuffle picks another song for next", () => {
    const p = createPlayer(songs, { ...frames(), random: () => 0.99 });
    p.play(songs[0].id);
    p.toggleShuffle();
    p.next();
    expect(p.state.song?.id).toBe(songs[songs.length - 1].id);
  });

  it("volume and mute: a volume above zero unmutes", () => {
    const p = createPlayer(songs, frames());
    expect(p.state.volume).toBe(0.7);
    p.toggleMute();
    expect(p.state.muted).toBe(true);
    p.setVolume(0.4);
    expect(p.state.muted).toBe(false);
    p.setVolume(2);
    expect(p.state.volume).toBe(1);
  });

  it("back from Now playing returns to the page it came from", () => {
    const p = createPlayer(songs, frames());
    p.show("library");
    p.show("now");
    p.back();
    expect(p.state.view).toBe("library");
  });

  it("history keeps the songs played, the latest first, once each", () => {
    const p = createPlayer(songs, frames());
    p.play(songs[1].id);
    p.play(songs[3].id);
    p.play(songs[1].id);
    expect(p.state.history.slice(0, 2)).toEqual([songs[1].id, songs[3].id]);
  });

  it("related lists the other songs, not the current one", () => {
    const p = createPlayer(songs, frames());
    p.play(songs[0].id);
    const r = p.related();
    expect(r).toHaveLength(songs.length - 1);
    expect(r.map((s) => s.id)).not.toContain(songs[0].id);
  });

  it("the Up next panel opens and switches tabs", () => {
    const p = createPlayer(songs, frames());
    p.toggleQueue();
    expect(p.state.queueOpen).toBe(true);
    p.setQueueTab("related");
    expect(p.state.queueTab).toBe("related");
  });
  it("Now playing's side panel starts shown and keeps the visitor's choice across views", () => {
    const p = createPlayer(songs, frames());
    expect(p.state.sideOpen).toBe(true);
    p.toggleSide();
    p.show("home");
    p.show("now");
    expect(p.state.sideOpen).toBe(false);
    p.toggleSide(true);
    expect(p.state.sideOpen).toBe(true);
  });
  it("with repeat off, the last song pauses at its end instead of starting the list again", () => {
    const p = createPlayer(songs, frames());
    const last = songs[songs.length - 1];
    p.play(last.id);
    p.seek(last.durationMs - 100);
    vi.advanceTimersByTime(300);
    expect(p.state.song?.id).toBe(last.id);
    expect(p.state.playing).toBe(false);
    // Play again starts it over.
    p.toggle();
    expect(p.state.playing).toBe(true);
    expect(p.state.positionMs).toBeLessThan(1000);
  });

  it("with repeat all, the last song goes on to the first", () => {
    const p = createPlayer(songs, frames());
    p.cycleRepeat();
    const last = songs[songs.length - 1];
    p.play(last.id);
    p.seek(last.durationMs - 100);
    vi.advanceTimersByTime(300);
    expect(p.state.song?.id).toBe(songs[0].id);
    expect(p.state.playing).toBe(true);
  });

  it("cue sets the song and the position, paused, without ever playing", () => {
    const p = createPlayer(songs, frames());
    p.play(songs[2].id);
    const seen: boolean[] = [];
    p.subscribe((s) => seen.push(s.playing));
    seen.length = 0;
    p.cue(songs[0].id, 20_600);
    expect(p.state.song?.id).toBe(songs[0].id);
    expect(p.state.positionMs).toBe(20_600);
    expect(p.state.playing).toBe(false);
    expect(seen).not.toContain(true);
  });

  it("jumps counts the position's jumps (seek, another song, a restart), not the clock's ticks", () => {
    const p = createPlayer(songs, frames());
    p.play(songs[0].id);
    const a = p.state.jumps;
    vi.advanceTimersByTime(2000);
    expect(p.state.jumps).toBe(a);
    p.seek(10_000);
    expect(p.state.jumps).toBe(a + 1);
    p.next();
    expect(p.state.jumps).toBe(a + 2);
    p.cycleRepeat();
    p.cycleRepeat(); // repeat one
    p.seek(songs[1].durationMs - 100);
    const b = p.state.jumps;
    vi.advanceTimersByTime(300);
    expect(p.state.jumps).toBe(b + 1);
  });

  it("names where the lyrics come from: the artist, under the song's licence", () => {
    const s = songs.find((x) => x.lyrics.length > 0)!;
    expect(lyricsSource(s)).toBe(`Lyrics: ${s.artist}, ${s.credit!.licence}`);
  });
});
