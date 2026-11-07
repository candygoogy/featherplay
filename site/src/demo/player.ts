// The demo's one player store. Every place on the page that shows featherplay (the hero's
// laptop, the lyrics showcase, the customise section, the tour) reads and drives this.
// Time is a clock that runs on animation frames while a song plays and the page is
// visible, and stands still otherwise. Sound, where a song has a clip, is a separate
// listener (src/demo/audio.ts) that follows this clock; the store itself is silent.

import type { Song } from "../data/songs";

/** Now playing (cover or lyrics), or one of the pages the sidebar and the search open. */
export type View = "home" | "now" | "lyrics" | "search" | "library" | "explore" | "history";
export type Theme = "dark" | "light";
export type Repeat = "off" | "all" | "one";
export type QueueTab = "next" | "related" | "lyrics";

export const isNowView = (v: View) => v === "now" || v === "lyrics";

export type Toast = { id: number; text: string; action?: "undo" };

export type PlayerState = {
  song: Song | null;
  playing: boolean;
  positionMs: number;
  view: View;
  minimised: boolean;
  lyricsOn: boolean;
  theme: Theme;
  /** False until the visitor picks a theme for the app ("Make it yours"); till then it follows the page. */
  themeSet: boolean;
  /** The search field's text and what it found (Search view). */
  query: string;
  result: Song | null;
  /** Songs liked, and marked "Not for me", by id. */
  liked: string[];
  disliked: string[];
  shuffle: boolean;
  repeat: Repeat;
  /** 0..1, as the app's volume slider; `muted` is the mute button (and the page's sound switch). */
  volume: number;
  muted: boolean;
  /** The Up next panel beside a browse page (Now playing always has it), and its tab. */
  queueOpen: boolean;
  queueTab: QueueTab;
  /**
   * The queue: the demo's songs up to this index (exclusive). Up next shows the song playing
   * and the ones after it, up to here; Clear brings it to the song playing, as the app's
   * Clear drops every song after it. Playing a song from the demo's list queues all of it.
   */
  queueEnd: number;
  /** Now playing's side panel (Up next, Related, Lyrics): the bar's Up next button hides and
   *  shows it there, and the page keeps the choice. */
  sideOpen: boolean;
  /** Songs played, the latest first (the History page). */
  history: string[];
  /** The toast over the bar ("Marked as not for me", Undo), or none. */
  toast: Toast | null;
  /**
   * Counts the times the position jumped (a seek, another song, a restart), not the clock's
   * ticks: the sound follows the clock and re-syncs only when this changes.
   */
  jumps: number;
};

export type Player = {
  readonly state: PlayerState;
  readonly songs: Song[];
  play(id: string): void;
  pause(): void;
  resume(): void;
  toggle(): void;
  next(): void;
  previous(): void;
  seek(ms: number): void;
  /** Sets the song and the position, paused, without playing it (the tour's "Start over"). */
  cue(id: string, positionMs: number): void;
  show(view: View): void;
  /** Back from Now playing to the page it was opened from. */
  back(): void;
  setLyrics(on: boolean): void;
  /** The app's own theme; "page" goes back to following the page's. */
  setTheme(theme: Theme | "page"): void;
  minimise(): void;
  restore(): void;
  search(text: string): Song | null;
  toggleLike(id?: string): void;
  /** Rates the song "Not for me" and skips it; the toast offers Undo. */
  notForMe(): void;
  undo(): void;
  dismissToast(): void;
  toggleShuffle(): void;
  cycleRepeat(): void;
  setVolume(v: number): void;
  toggleMute(): void;
  toggleQueue(open?: boolean): void;
  setQueueTab(tab: QueueTab): void;
  /** Up next's Clear: no songs after the one playing. */
  clearQueue(): void;
  /** Hides or shows Now playing's side panel. */
  toggleSide(open?: boolean): void;
  /** The songs Related shows for the current one. */
  related(): Song[];
  subscribe(fn: (s: PlayerState) => void): () => void;
  /** Call when the page's visibility changes (the default wiring does it for you). */
  visibilityChanged(): void;
  /**
   * What the visitor actually hears: the clip's position in ms, or null when nothing is
   * sounding (not loaded yet, paused, seeking, refused). While it answers, the clock keeps
   * to it, so the lyrics and the bar never run ahead of a clip that started late or stalled.
   */
  followSound(source: (() => number | null) | null): void;
};

/** How far the clock may drift from the sound before it's pulled back to it. */
export const SOUND_DRIFT_MS = 120;

export type PlayerOptions = {
  now?: () => number;
  raf?: (cb: (t: number) => void) => number;
  caf?: (id: number) => void;
  visible?: () => boolean;
  random?: () => number;
  initial?: { songId?: string; positionMs?: number; view?: View; playing?: boolean };
};

export const DEMO_LINK = /music\.example\/watch\?v=([\w-]+)/i;
export const NOT_FOR_ME_TOAST = "Marked as not for me";

const fold = (s: string) =>
  s
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/&/g, "and")
    .replace(/[^a-z0-9]+/g, " ")
    .trim();

export function createPlayer(songs: Song[], options: PlayerOptions = {}): Player {
  const hasDom = typeof window !== "undefined" && typeof document !== "undefined";
  const now = options.now ?? (() => performance.now());
  const raf = options.raf ?? ((cb) => requestAnimationFrame(cb));
  const caf = options.caf ?? ((id) => cancelAnimationFrame(id));
  const visible = options.visible ?? (() => !hasDom || document.visibilityState === "visible");
  const random = options.random ?? Math.random;

  const init = options.initial ?? {};
  const first = init.songId ? (songs.find((s) => s.id === init.songId) ?? null) : null;
  const state: PlayerState = {
    song: first,
    playing: false,
    positionMs: first ? Math.max(0, init.positionMs ?? 0) : 0,
    view: init.view ?? "home",
    minimised: false,
    lyricsOn: true,
    theme: "dark",
    themeSet: false,
    query: "",
    result: null,
    liked: [],
    disliked: [],
    shuffle: false,
    repeat: "off",
    volume: 0.7,
    muted: false,
    queueOpen: false,
    queueTab: "next",
    queueEnd: songs.length,
    sideOpen: true,
    history: first ? [first.id] : [],
    toast: null,
    jumps: 0,
  };

  const listeners = new Set<(s: PlayerState) => void>();
  const emit = () => {
    for (const fn of listeners) fn(state);
  };

  // The clock: while running, position = base + (now - since).
  let frame: number | null = null;
  let since = 0;
  let base = 0;
  let browseView: View = isNowView(state.view) ? "home" : state.view;
  let toastId = 0;
  let undoTo: { song: Song; positionMs: number } | null = null;

  let sound: (() => number | null) | null = null;

  const running = () => state.playing && state.song !== null && visible();

  const stopClock = () => {
    if (frame !== null) caf(frame);
    frame = null;
  };

  const step = () => {
    frame = null;
    if (!running() || !state.song) return;
    state.positionMs = base + (now() - since);
    // Keep to the sound: a clip that started late, stalled or was held up runs the show.
    const heard = sound?.();
    if (heard != null && Math.abs(heard - state.positionMs) > SOUND_DRIFT_MS) {
      base = heard;
      since = now();
      state.positionMs = heard;
    }
    if (state.positionMs >= state.song.durationMs) {
      // The song ended: again (repeat one), or on to the next, as the app's queue would.
      if (state.repeat === "one") {
        state.positionMs = 0;
        state.jumps++;
        startClock();
        emit();
        return;
      }
      // The end of the queue, with repeat off: it stops there, as the app's queue does.
      // Shuffle plays on through the whole list; after a Clear the queue ends here all the same.
      const last = indexOf(state.song) === state.queueEnd - 1;
      const cleared = state.queueEnd < songs.length;
      if (last && state.repeat === "off" && (!state.shuffle || cleared)) {
        state.positionMs = state.song.durationMs;
        state.playing = false;
        emit();
        return;
      }
      goTo(nextIndex());
      return;
    }
    emit();
    frame = raf(step);
  };

  function startClock() {
    stopClock();
    if (!running()) return;
    since = now();
    base = state.positionMs;
    frame = raf(step);
  }

  const indexOf = (song: Song) => songs.findIndex((s) => s.id === song.id);
  const nextIndex = () => {
    if (!state.song) return 0;
    const at = indexOf(state.song);
    const end = state.queueEnd;
    if (state.shuffle && end > 1) {
      const pick = Math.floor(random() * (end - 1));
      return pick >= at ? pick + 1 : pick;
    }
    // Nothing after it: back to the queue's first song.
    return at + 1 < end ? at + 1 : 0;
  };

  function goTo(index: number) {
    const i = ((index % songs.length) + songs.length) % songs.length;
    const song = songs[i];
    state.song = song;
    state.queueEnd = Math.max(state.queueEnd, i + 1);
    state.positionMs = 0;
    state.playing = true;
    state.jumps++;
    state.history = [song.id, ...state.history.filter((id) => id !== song.id)];
    startClock();
    emit();
  }

  const toggleIn = (list: string[], id: string, on: boolean) =>
    on ? (list.includes(id) ? list : [...list, id]) : list.filter((x) => x !== id);

  const player: Player = {
    state,
    songs,
    play(id) {
      const song = songs.find((s) => s.id === id);
      if (!song) return;
      if (state.song?.id === id && !state.playing) return player.resume();
      // From the demo's list: all of it is the queue again.
      state.queueEnd = songs.length;
      goTo(indexOf(song));
    },
    pause() {
      if (!state.playing) return;
      if (frame !== null && state.song) state.positionMs = Math.min(base + (now() - since), state.song.durationMs - 1);
      stopClock();
      state.playing = false;
      emit();
    },
    resume() {
      if (!state.song || state.playing) return;
      // Ended (the list stopped there): play starts the song over.
      if (state.positionMs >= state.song.durationMs) {
        state.positionMs = 0;
        state.jumps++;
      }
      state.playing = true;
      startClock();
      emit();
    },
    toggle() {
      if (!state.song) return goTo(0);
      if (state.playing) player.pause();
      else player.resume();
    },
    next() {
      goTo(nextIndex());
    },
    previous() {
      if (state.song && state.positionMs > 3000) return player.seek(0);
      goTo(state.song ? indexOf(state.song) - 1 : 0);
    },
    seek(ms) {
      if (!state.song) return;
      state.positionMs = Math.max(0, Math.min(Math.round(ms), state.song.durationMs - 1));
      state.jumps++;
      startClock();
      emit();
    },
    cue(id, positionMs) {
      const song = songs.find((s) => s.id === id);
      if (!song) return;
      stopClock();
      state.song = song;
      state.queueEnd = songs.length;
      state.playing = false;
      state.positionMs = Math.max(0, Math.min(Math.round(positionMs), song.durationMs - 1));
      state.jumps++;
      emit();
    },
    show(view) {
      const next: View = view === "lyrics" && !state.lyricsOn ? "now" : view;
      if (next === state.view) return;
      if (!isNowView(next)) browseView = next;
      state.view = next;
      emit();
    },
    back() {
      player.show(isNowView(state.view) ? browseView : "home");
    },
    setLyrics(on) {
      state.lyricsOn = on;
      if (!on && state.view === "lyrics") state.view = "now";
      emit();
    },
    setTheme(theme) {
      if (theme === "page") {
        if (!state.themeSet) return;
        state.themeSet = false;
      } else {
        if (state.theme === theme && state.themeSet) return;
        state.theme = theme;
        state.themeSet = true;
      }
      emit();
    },
    minimise() {
      if (state.minimised) return;
      state.minimised = true;
      emit();
    },
    restore() {
      if (!state.minimised) return;
      state.minimised = false;
      emit();
    },
    search(text) {
      const query = text.trim();
      state.query = query;
      state.view = "search";
      browseView = "search";
      const link = query.match(DEMO_LINK);
      let found: Song | null = null;
      if (link) {
        found = songs.find((s) => s.id === link[1]) ?? null;
      } else if (fold(query)) {
        const q = fold(query);
        found = songs.find((s) => fold(s.title).includes(q) || fold(s.artist).includes(q)) ?? null;
      }
      state.result = found;
      emit();
      return found;
    },
    toggleLike(id) {
      const target = id ?? state.song?.id;
      if (!target) return;
      const on = !state.liked.includes(target);
      state.liked = toggleIn(state.liked, target, on);
      if (on) state.disliked = toggleIn(state.disliked, target, false);
      emit();
    },
    notForMe() {
      const song = state.song;
      if (!song) return;
      undoTo = { song, positionMs: state.positionMs };
      state.disliked = toggleIn(state.disliked, song.id, true);
      state.liked = toggleIn(state.liked, song.id, false);
      state.toast = { id: ++toastId, text: NOT_FOR_ME_TOAST, action: "undo" };
      goTo(nextIndex());
    },
    undo() {
      if (!undoTo) return;
      const { song, positionMs } = undoTo;
      undoTo = null;
      state.disliked = toggleIn(state.disliked, song.id, false);
      state.toast = null;
      state.song = song;
      state.queueEnd = Math.max(state.queueEnd, indexOf(song) + 1);
      state.positionMs = positionMs;
      state.playing = true;
      state.jumps++;
      startClock();
      emit();
    },
    dismissToast() {
      if (!state.toast) return;
      state.toast = null;
      undoTo = null;
      emit();
    },
    toggleShuffle() {
      state.shuffle = !state.shuffle;
      emit();
    },
    cycleRepeat() {
      state.repeat = state.repeat === "off" ? "all" : state.repeat === "all" ? "one" : "off";
      emit();
    },
    setVolume(v) {
      const volume = Math.max(0, Math.min(1, v));
      if (volume === state.volume && !(volume > 0 && state.muted)) return;
      state.volume = volume;
      if (volume > 0) state.muted = false;
      emit();
    },
    toggleMute() {
      state.muted = !state.muted;
      emit();
    },
    toggleQueue(open) {
      const next = open ?? !state.queueOpen;
      if (next === state.queueOpen) return;
      state.queueOpen = next;
      emit();
    },
    setQueueTab(tab) {
      if (tab === state.queueTab) return;
      state.queueTab = tab;
      emit();
    },
    clearQueue() {
      if (!state.song) return;
      const end = indexOf(state.song) + 1;
      if (end === state.queueEnd) return;
      state.queueEnd = end;
      emit();
    },
    toggleSide(open) {
      const next = open ?? !state.sideOpen;
      if (next === state.sideOpen) return;
      state.sideOpen = next;
      emit();
    },
    related() {
      // "Similar to" the current song: the others, the nearest in colour first (a demo stand-in
      // for the service's suggestions, stable for each song).
      const cur = state.song;
      if (!cur) return [];
      const hue = (hex: string) => {
        const n = parseInt(hex.slice(1), 16);
        const [r, g, b] = [(n >> 16) & 255, (n >> 8) & 255, n & 255].map((v) => v / 255);
        const max = Math.max(r, g, b);
        const min = Math.min(r, g, b);
        if (max === min) return 0;
        const d = max - min;
        const h = max === r ? ((g - b) / d) % 6 : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
        return (h * 60 + 360) % 360;
      };
      const h0 = hue(cur.colour);
      const dist = (s: Song) => {
        const d = Math.abs(hue(s.colour) - h0);
        return Math.min(d, 360 - d);
      };
      return songs.filter((s) => s.id !== cur.id).sort((a, b) => dist(a) - dist(b));
    },
    subscribe(fn) {
      listeners.add(fn);
      fn(state);
      return () => listeners.delete(fn);
    },
    followSound(source) {
      sound = source;
    },
    visibilityChanged() {
      if (!state.playing) return;
      if (visible()) {
        startClock();
      } else {
        if (frame !== null && state.song) state.positionMs = base + (now() - since);
        stopClock();
      }
    },
  };

  if (init.playing && state.song) player.resume();
  if (hasDom && !options.visible) {
    document.addEventListener("visibilitychange", () => player.visibilityChanged());
  }
  return player;
}

/** The line being sung at `ms`, or -1 before the first (and for songs without lyrics). */
export function currentLineIndex(song: Song, ms: number): number {
  let index = -1;
  for (let i = 0; i < song.lyrics.length; i++) {
    if (song.lyrics[i].atMs <= ms) index = i;
    else break;
  }
  return index;
}

/** The line under a song's lyrics: whose they are, and the licence they're shown under. */
export function lyricsSource(song: Song): string {
  return song.credit ? `Lyrics: ${song.artist}, ${song.credit.licence}` : `Lyrics: ${song.artist}`;
}

/** Now playing's line under the artist: "album · year", or "" for a track without an album. */
export function albumLine(song: Song): string {
  return song.album ? `${song.album.title} · ${song.album.year}` : "";
}

/** 0:00, as the app's bar shows it. */
export function formatTime(ms: number): string {
  const total = Math.floor(Math.max(0, ms) / 1000);
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}
