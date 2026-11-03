// "Lyrics, up close": a large lyrics view on the same player as the hero's demo. The lines
// glide so the one being sung stays in place; a click on a line seeks there; the Show lyrics
// switch turns lyrics off here and in the demo, as the app's setting does.
//
// This section always shows lyrics. It offers only the songs that have them; while the demo
// plays one without (an instrumental, picked in the hero), it keeps the last song with
// lyrics on show, paused where it was, and its Play (or a click on a line) plays that one.

import type { Song } from "../data/songs";
import { currentLineIndex, formatTime, lyricsSource, type Player, type PlayerState } from "./player";
import { followLyrics } from "./follow";
import { lyricInks } from "../data/colour";

export function bindShowcase(root: HTMLElement, player: Player): void {
  const q = <T extends HTMLElement = HTMLElement>(sel: string) => root.querySelector<T>(sel)!;
  const panel = q("[data-showcase-panel]");
  const lines = q<HTMLOListElement>("[data-showcase-lines]");
  const calm = q("[data-showcase-calm]");
  const off = q("[data-showcase-off]");
  const title = q("[data-showcase-title]");
  const artist = q("[data-showcase-artist]");
  const cover = q<HTMLImageElement>("[data-showcase-cover]");
  const time = q("[data-showcase-time]");
  const duration = q("[data-showcase-duration]");
  const play = q<HTMLButtonElement>("[data-showcase-play]");
  const toggle = q<HTMLButtonElement>("[data-showcase-switch]");
  const source = root.querySelector<HTMLElement>("[data-showcase-source]");
  const picks = [...root.querySelectorAll<HTMLButtonElement>("[data-showcase-pick]")];
  // The credit here names the song on show (which may not be the one the demo is playing).
  const credit = root.querySelector<HTMLElement>("[data-credit]");
  credit?.setAttribute("data-credit-own", "");

  const withLyrics = player.songs.filter((s) => s.lyrics.length > 0);
  /** The song with lyrics last seen here, and where it was. */
  let kept: { song: Song; at: number } | null = null;

  let song: string | null = null;
  let line = -2;
  let second = -1;
  let playing: boolean | null = null;
  let lyricsOn: boolean | null = null;
  let live: boolean | null = null;

  const build = (s: Song) =>
    s.lyrics.map((l, i) => {
      const li = document.createElement("li");
      if (i > 0 && l.atMs - s.lyrics[i - 1].atMs > 5200) li.className = "verse";
      const b = document.createElement("button");
      b.type = "button";
      b.className = "line";
      b.dataset.at = String(l.atMs);
      b.textContent = l.line;
      li.append(b);
      return li;
    });

  // The lines follow the song, and the wheel or a finger can move them for a while.
  const follower = followLyrics(panel, lines);
  const glide = (index: number, instant = false) => {
    const at = index >= 0 ? (lines.children[index] as HTMLElement | undefined) : undefined;
    follower.follow(at ?? null, instant);
  };

  /** What this section shows: the playing song if it has lyrics, else the one kept. */
  const shown = (s: PlayerState): { song: Song; at: number; live: boolean } => {
    if (s.song && s.song.lyrics.length) return { song: s.song, at: s.positionMs, live: true };
    const k = kept ?? { song: withLyrics[0], at: 0 };
    return { ...k, live: false };
  };

  /** Plays the shown song from `at` (it may not be the demo's current song). */
  const playShown = (at?: number) => {
    const view = shown(player.state);
    if (view.live) {
      if (at !== undefined) player.seek(at);
      else player.toggle();
      return;
    }
    player.play(view.song.id);
    const from = at ?? view.at;
    if (from > 0) player.seek(from);
  };

  lines.addEventListener("click", (e) => {
    const b = (e.target as Element).closest<HTMLElement>(".line");
    if (!b) return;
    follower.release();
    playShown(Number(b.dataset.at));
    // A seek on the playing song keeps its play state; from a kept song, it plays.
  });
  play.addEventListener("click", () => playShown());
  toggle.addEventListener("click", () => player.setLyrics(!player.state.lyricsOn));
  for (const b of picks) {
    b.addEventListener("click", () => {
      const id = b.dataset.showcasePick!;
      if (player.state.song?.id === id) {
        if (!player.state.playing) player.resume();
        return;
      }
      player.play(id);
    });
  }

  const render = (s: PlayerState) => {
    const view = shown(s);
    if (view.live) kept = { song: view.song, at: view.at };
    const cur = view.song;
    if (cur.id !== song) {
      song = cur.id;
      root.style.setProperty("--song", cur.colour);
      const ink = lyricInks(cur.colour);
      root.style.setProperty("--lyric-ground", ink.ground);
      root.style.setProperty("--lyric-next", ink.nextCss);
      root.style.setProperty("--lyric-past", ink.pastCss);
      title.textContent = cur.title;
      artist.textContent = cur.artist;
      cover.src = cur.cover;
      duration.textContent = formatTime(cur.durationMs);
      if (source) source.textContent = lyricsSource(cur);
      if (credit && cur.credit) {
        const site = credit.querySelector<HTMLAnchorElement>("[data-credit-site]");
        const licence = credit.querySelector<HTMLAnchorElement>("[data-credit-licence]");
        const text = credit.querySelector("[data-credit-text]");
        if (text) text.textContent = `“${cur.title}” by ${cur.artist}`;
        if (site) {
          site.href = cur.credit.site.url;
          site.textContent = cur.credit.site.label;
        }
        if (licence) {
          licence.href = cur.credit.licenceUrl;
          licence.textContent = cur.credit.licence;
        }
      }
      for (const b of picks) b.setAttribute("aria-pressed", String(b.dataset.showcasePick === cur.id));
      lines.replaceChildren(...build(cur));
      follower.release();
      glide(-1, true);
      line = -2;
      second = -1;
    }
    if (s.lyricsOn !== lyricsOn) {
      lyricsOn = s.lyricsOn;
      toggle.setAttribute("aria-checked", String(s.lyricsOn));
      line = -2;
    }
    // Off: the lines fade out inside the panel and the note fades in, in the same place; the
    // panel and its caption keep their size (CSS, LyricsShowcase.astro).
    calm.hidden = true;
    off.hidden = false;
    panel.dataset.state = !s.lyricsOn ? "off" : "lines";
    if (source) source.dataset.off = String(!s.lyricsOn);

    const isPlaying = view.live && s.playing;
    if (isPlaying !== playing || view.live !== live) {
      playing = isPlaying;
      live = view.live;
      root.dataset.playing = String(isPlaying);
      play.setAttribute("aria-label", isPlaying ? "Pause" : view.live ? "Play" : `Play ${cur.title}`);
    }
    const sec = Math.floor(view.at / 1000);
    if (sec !== second) {
      second = sec;
      time.textContent = formatTime(view.at);
    }
    const index = currentLineIndex(cur, view.at);
    if (index !== line && s.lyricsOn) {
      line = index;
      [...lines.querySelectorAll<HTMLElement>(".line")].forEach((b, i) => {
        b.dataset.state = i < index ? "past" : i === index ? "current" : "next";
        if (i === index) b.setAttribute("aria-current", "true");
        else b.removeAttribute("aria-current");
      });
      glide(index);
    }
  };

  player.subscribe(render);
}
