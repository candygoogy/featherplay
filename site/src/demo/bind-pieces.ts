// Draws every small live piece on the page from the one player store (the replica's,
// src/demo/replica.ts): the media controls, the tray's tooltip, the Discord status, scrobbling,
// SponsorBlock, the lyric card, the first screen's play button, and the system's own media
// controls (Media Session). One sound, one clock: none of them keeps time of its own.
//
// The store speaks on every frame while a song plays; each piece works only when something it
// shows has changed (the song, play or pause, a jump, the second, the line). Progress lines run
// on the compositor: one linear animation to the song's end, restarted on a change.

import type { Song } from "../data/songs";
import { currentLineIndex, formatTime, lyricsSource, type Player, type PlayerState } from "./player";
import { createListen, mediaMetadataFor, scrobbleAfterMs, sponsorPlayhead, trayLines } from "./pieces";
import { followLyrics } from "./follow";
import { lineItems, markLines } from "./lyric-lines";
import { runLine } from "./lines";

const $$ = <T extends Element = HTMLElement>(sel: string, root: ParentNode = document) => [...root.querySelectorAll<T>(sel)];
const reduced = () => matchMedia("(prefers-reduced-motion: reduce)").matches;

/** Rendered now: not in a closed popover, a hidden page or a section that waits to render. */
export const rendered = (el: Element) => !el.checkVisibility || el.checkVisibility({ contentVisibilityAuto: true });

/** Runs a progress line from where the song is to its end, on the compositor. One that isn't
 *  rendered just takes its place: an animation there can't run on the compositor, and the
 *  main thread would tick it every frame for nothing; it starts when it shows. */
/** Pieces out of view (above or below the screen, though their section is rendered): their
 *  lines hold still until they come back (src/demo/bind-pieces.ts, an observer). */
const away = new WeakSet<Element>();
/** A piece's progress line, restarted on the next frame with the others (src/demo/lines.ts). */
function pieceLine(el: HTMLElement, ends: () => [string, string], ms: number, playing: boolean) {
  const piece = el.closest("[data-piece]");
  runLine({ el, prop: "transform", ends, ms, run: playing && !(piece && away.has(piece)) });
}

/** A small cover changing song: the new one fades over the old (no hard cut). */
function swapCover(img: HTMLImageElement, src: string) {
  if (img.getAttribute("src") === src) return;
  const piece = img.closest("[data-piece]");
  // Out of view, simply the new cover (an unseen fade runs on the page's own thread). Nothing is
  // read from layout here: it runs in the middle of a song change's writes.
  if (reduced() || !img.isConnected || (piece && away.has(piece))) {
    img.src = src;
    return;
  }
  img.src = src;
  img.animate([{ opacity: 0.2 }, { opacity: 1 }], { duration: 200, easing: "ease-out" });
}

let bound = false;

export function bindPieces(player: Player): void {
  if (bound || typeof document === "undefined") return;
  bound = true;
  const doc = document.documentElement;
  // The pieces with controls are inert until their script is here (MediaFlyout, LyricCard).
  for (const p of $$("[data-piece][inert]")) p.inert = false;

  // ---- Buttons anywhere on the page that act on the player ----
  document.addEventListener("click", (e) => {
    const b = (e.target as Element).closest<HTMLElement>("[data-do]");
    if (!b) return;
    const act = b.dataset.do;
    if (act === "toggle") player.toggle();
    else if (act === "previous") player.previous();
    else if (act === "next") player.next();
    else if (act === "play" && b.dataset.song) {
      if (player.state.song?.id === b.dataset.song) player.toggle();
      else player.play(b.dataset.song);
    }
  });

  // ---- The lyric cards: lines that follow the song; a click on one seeks there ----
  const cards = $$("[data-piece='lyrics']").map((card) => {
    const view = card.querySelector<HTMLElement>("[data-lc-view]")!;
    const list = card.querySelector<HTMLElement>("[data-lc-lines]")!;
    const follow = followLyrics(view, list);
    list.addEventListener("click", (e) => {
      const line = (e.target as Element).closest<HTMLElement>(".lc-line");
      if (!line) return;
      follow.release();
      player.seek(Number(line.dataset.at));
      if (!player.state.playing) player.resume();
    });
    const c: { card: HTMLElement; list: HTMLElement; follow: ReturnType<typeof followLyrics>; line: number; instant: boolean; stale: Song | null } = { card, list, follow, line: -2, instant: true, stale: null };
    // In a section that waits to render (content-visibility) the lines have no place to measure:
    // they're placed when it shows.
    card.closest("section")?.addEventListener("contentvisibilityautostatechange", (e) => {
      if ((e as Event & { skipped: boolean }).skipped) return;
      c.line = -2;
      c.instant = true;
      render(player.state);
    });
    return c;
  });
  const buildLines = (song: Song) => lineItems(song, { className: "lc-line" });
  function fillCard(c: (typeof cards)[number], song: Song) {
    c.stale = null;
    const has = song.lyrics.length > 0;
    c.card.dataset.hasLyrics = String(has);
    c.list.replaceChildren(...buildLines(song));
    const src = c.card.querySelector("[data-lc-source]");
    if (src) src.textContent = has ? lyricsSource(song) : "";
    c.follow.release();
    c.line = -2;
    c.instant = true;
  }

  // ---- Scrobbling ----
  const scrobbles = $$("[data-piece='scrobble']");
  const listen = createListen();
  // The ring fills with the time heard, drawn about one device pixel of its round at a time
  // (some 130 steps to the scrobble), only where it shows: a few small restyles a second,
  // not an animation ticking every frame (stroke-dashoffset can't run on the compositor).
  const ring = { heardMs: 0, needMs: Infinity, at: 0, playing: false, scrobbled: false };
  let ringTimer = 0;
  const drawRings = () => {
    clearTimeout(ringTimer);
    const heard = ring.heardMs + (ring.playing ? performance.now() - ring.at : 0);
    const p = ring.scrobbled ? 1 : Math.min(1, heard / ring.needMs);
    let shown = false;
    for (const sc of scrobbles) {
      const el = sc.querySelector<SVGElement>("[data-sc-ring]");
      // In view only (the observer below); nothing read from layout in the middle of a change.
      if (!el || away.has(sc)) continue;
      shown = true;
      el.style.setProperty("--p", p.toFixed(4));
    }
    if (shown && ring.playing && !ring.scrobbled && p < 1) ringTimer = window.setTimeout(drawRings, Math.max(200, ring.needMs / 130));
  };
  const addPast = (song: Song) => {
    for (const s of scrobbles) {
      const past = s.querySelector("[data-sc-past]");
      if (!past) continue;
      const li = document.createElement("li");
      const img = document.createElement("img");
      img.src = song.cover;
      img.width = img.height = 32;
      img.alt = "";
      const b = document.createElement("b");
      b.textContent = song.title;
      const t = document.createElement("span");
      t.textContent = "Scrobbled";
      li.append(img, b, t);
      past.prepend(li);
      while (past.children.length > 2) past.lastElementChild!.remove();
    }
  };

  // ---- SponsorBlock: the hop over the talk as a song starts and ends ----
  const sponsors = $$("[data-piece='sponsor']");
  let hopTimer = 0;
  let hopped = "";
  const hop = (side: "a" | "b") => {
    hopped = side;
    // Only where it is seen: out of view its animations would run on the page's own thread.
    for (const s of sponsors) if (!away.has(s)) s.dataset.hop = side;
    clearTimeout(hopTimer);
    hopTimer = window.setTimeout(() => sponsors.forEach((s) => delete s.dataset.hop), 1900);
  };

  // ---- The first screen's play button: "Play a song", then Pause and Play ----
  const toggles = $$("[data-play-toggle]");
  let started = false;
  for (const t of toggles) t.hidden = false;

  // ---- The song's announcement: polite, once a song and when it pauses ----
  const announcer = document.querySelector<HTMLElement>("[data-announce]");
  let announceTimer = 0;
  let announced = "";

  // ---- Media Session: the page's song in the system's own media controls ----
  const ms = "mediaSession" in navigator ? navigator.mediaSession : null;
  if (ms) {
    const set = (action: MediaSessionAction, fn: MediaSessionActionHandler) => {
      try {
        ms.setActionHandler(action, fn);
      } catch {
        // An action this browser doesn't offer.
      }
    };
    set("play", () => player.resume());
    set("pause", () => player.pause());
    set("previoustrack", () => player.previous());
    set("nexttrack", () => player.next());
    set("seekto", (d) => d.seekTime != null && player.seek(d.seekTime * 1000));
  }

  let last: { song?: string; playing?: boolean; jumps?: number; retimes?: number; second?: number } = {};
  // A part showing (a flyout opening, a section rendering): its lines start from where the
  // song is.
  let resync = false;
  const again = () => {
    resync = true;
    // A card back in view takes the song it missed.
    for (const c of cards) if (c.stale && !away.has(c.card)) fillCard(c, c.stale);
    render(player.state);
  };
  for (const fly of document.querySelectorAll<HTMLElement>("[data-fly]")) fly.addEventListener("toggle", (e) => (e as ToggleEvent).newState === "open" && again());
  // A piece going out of view stops its lines; coming back, they start from where the song is.
  // (Off screen the browser may tick them on the page's own thread.)
  if ("IntersectionObserver" in window) {
    const io = new IntersectionObserver(
      (es) => {
        let changed = false;
        for (const e of es) {
          if (e.isIntersecting === !away.has(e.target)) continue;
          if (e.isIntersecting) away.delete(e.target);
          else away.add(e.target);
          changed = true;
        }
        if (changed) again();
      },
      { rootMargin: "64px" },
    );
    for (const p of document.querySelectorAll("[data-piece]")) if (!p.closest("[data-fly]")) io.observe(p);
  }
  if ("ResizeObserver" in window) {
    const widths = new WeakMap<Element, number>();
    const ro = new ResizeObserver((es) => {
      let changed = false;
      for (const e of es) {
        const w = Math.round(e.contentRect.width);
        if (widths.has(e.target) && widths.get(e.target) !== w) changed = true;
        widths.set(e.target, w);
      }
      if (changed) again();
    });
    for (const el of $$("[data-sp-head]")) if (el.parentElement) ro.observe(el.parentElement);
  }
  for (const s of new Set([...document.querySelectorAll("[data-piece]")].map((p) => p.closest("section")))) {
    s?.addEventListener("contentvisibilityautostatechange", (e) => !(e as Event & { skipped: boolean }).skipped && again());
  }

  const render = (s: PlayerState) => {
    const song = s.song;
    if (!song) return;
    const songChanged = song.id !== last.song;
    const playChanged = s.playing !== last.playing;
    // A jump, or the clock set back to the sound: the progress lines start again from here.
    const jumped = s.jumps !== last.jumps || s.retimes !== last.retimes;
    const second = Math.floor(s.positionMs / 1000);
    const secondChanged = second !== last.second;
    if (s.playing) started = true;

    if (songChanged) {
      for (const el of $$("[data-piece] [data-bind='title']")) el.textContent = song.title;
      for (const el of $$("[data-piece] [data-bind='artist']")) el.textContent = song.artist;
      for (const el of $$("[data-piece] [data-bind='album']")) el.textContent = song.album?.title ?? "";
      for (const el of $$("[data-piece] [data-bind-wrap='album']")) el.hidden = !song.album;
      for (const el of $$("[data-piece] [data-bind='duration']")) el.textContent = formatTime(song.durationMs);
      for (const img of $$<HTMLImageElement>("[data-piece] img[data-bind='cover']")) swapCover(img, song.cover);
      // A card out of view waits for its lines until it comes back (again(), below).
      for (const c of cards) {
        if (away.has(c.card)) c.stale = song;
        else fillCard(c, song);
      }
      if (last.song !== undefined && sponsors.length) setTimeout(() => hop("a"), 520);
      if (ms) {
        try {
          ms.metadata = new MediaMetadata(mediaMetadataFor(song, location.origin));
        } catch {
          // No MediaMetadata here.
        }
      }
    }

    if (playChanged) {
      doc.dataset.playing = String(s.playing);
      for (const el of $$("[data-piece][data-piece='discord']")) el.toggleAttribute("data-on", s.playing);
      for (const el of $$("[data-play-label]")) el.setAttribute("aria-label", s.playing ? "Pause" : "Play");
      if (ms) ms.playbackState = s.playing ? "playing" : "paused";
    }
    if (playChanged || songChanged) {
      const label = s.playing ? "Pause" : started ? "Play" : "Play a song";
      for (const t of toggles) {
        const text = t.querySelector("[data-play-toggle-text]");
        if (text) text.textContent = label;
      }
      clearTimeout(announceTimer);
      if (announcer && started) {
        announceTimer = window.setTimeout(() => {
          const text = s.playing ? `Playing ${song.title} by ${song.artist}` : `Paused: ${song.title}`;
          if (text !== announced) announcer.textContent = announced = text;
        }, 350);
      }
    }

    // Progress lines: from here to the end, restarted when the song, play or the place changes.
    const resynced = resync;
    if (songChanged || playChanged || jumped || resync) {
      resync = false;
      const f = Math.min(1, s.positionMs / song.durationMs);
      const left = Math.max(0, song.durationMs - s.positionMs);
      for (const el of $$("[data-piece] [data-fill]")) pieceLine(el, () => [`scaleX(${f})`, "scaleX(1)"], left, s.playing);
      // SponsorBlock's head in pixels of its rail (an animation in container units can't run on
      // the compositor), the rail read at the frame; the rail changing width starts it again.
      for (const el of $$("[data-sp-head]")) {
        pieceLine(
          el,
          () => {
            const w = (el.parentElement?.clientWidth ?? 0) / 100;
            return [`translateX(${(sponsorPlayhead(f) * w).toFixed(2)}px)`, `translateX(${(sponsorPlayhead(1) * w).toFixed(2)}px)`];
          },
          left,
          s.playing,
        );
      }
    }

    if (songChanged || secondChanged || playChanged) {
      const at = formatTime(s.positionMs);
      for (const el of $$("[data-piece] [data-bind='at']")) el.textContent = at;
      const [a, b] = trayLines(s);
      for (const el of $$("[data-tray-1]")) el.textContent = a;
      for (const el of $$("[data-tray-2]")) el.textContent = b;
      if (ms?.setPositionState) {
        try {
          ms.setPositionState({ duration: song.durationMs / 1000, position: Math.min(s.positionMs, song.durationMs) / 1000, playbackRate: 1 });
        } catch {
          // A position a moment past the end.
        }
      }
      const f = s.positionMs / song.durationMs;
      if (f > 0.985 && hopped !== "b") hop("b");
      else if (f < 0.5 && hopped === "b") hopped = "";
    }

    // Scrobbling counts on every step (time heard, not where the song is).
    const l = listen.step(s);
    if (l.finished) addPast(l.finished);
    if (songChanged || secondChanged || l.justScrobbled || playChanged || resynced) {
      Object.assign(ring, { heardMs: l.heardMs, needMs: scrobbleAfterMs(song.durationMs), at: performance.now(), playing: s.playing, scrobbled: l.scrobbled });
      if (scrobbles.length) drawRings();
      for (const sc of scrobbles) {
        const row = sc.querySelector<HTMLElement>("[data-sc-row]");
        row?.toggleAttribute("data-scrobbled", l.scrobbled);
        const label = sc.querySelector("[data-sc-label]");
        const text = l.scrobbled ? "Scrobbled" : s.playing ? "Scrobbling now" : "Paused";
        if (label && label.textContent !== text) label.textContent = text;
      }
    }

    // The lyric cards' current line (not while their section is skipped: nothing to measure).
    const line = currentLineIndex(song, s.positionMs);
    for (const c of cards) {
      if (line === c.line) continue;
      // Out of view: placed when it comes back (again()).
      if (away.has(c.card)) continue;
      c.line = line;
      markLines(c.list, ".lc-line", line);
      const li = line >= 0 ? (c.list.children[line] as HTMLElement | undefined) : undefined;
      c.follow.follow(li ?? null, c.instant);
      c.instant = false;
    }

    last = { song: song.id, playing: s.playing, jumps: s.jumps, retimes: s.retimes, second };
  };
  player.subscribe(render);
}
