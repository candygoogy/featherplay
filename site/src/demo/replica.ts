// Binds every `[data-replica]` on the page to the one shared player store, and the desktop
// around it (the taskbar button, the tray icon and its hover text) to the same state.

import { songs, type Song } from "../data/songs";
import { createPlayer, currentLineIndex, formatTime, isNowView, lyricsSource, type Player, type PlayerState, type View } from "./player";
import { setGlow } from "./glow";
import { followLyrics } from "./follow";
import { bindAudio } from "./audio";
import { bindCredits } from "./credit";

let shared: Player | null = null;

// The toast: one timer for the page (every player shows the same toast; resting the pointer
// on any of them holds it), and one live region that's always there, so it's announced
// once (a region that appears with its text often isn't read at all).
let toastTimer = 0;
let toastHeld = false;
function armToast(player: Player) {
  clearTimeout(toastTimer);
  if (!toastHeld) toastTimer = window.setTimeout(() => player.dismissToast(), 5000);
}
function holdToast(held: boolean, player: Player) {
  toastHeld = held;
  if (held) clearTimeout(toastTimer);
  else armToast(player);
}
let toastLive: HTMLElement | null = null;
function liveRegion(): HTMLElement {
  if (toastLive) return toastLive;
  toastLive = document.createElement("p");
  toastLive.className = "visually-hidden";
  toastLive.setAttribute("role", "status");
  toastLive.dataset.toastLive = "";
  document.body.append(toastLive);
  return toastLive;
}

/** The page's one player. The first replica's rendered state is where it starts. */
export function sharedPlayer(initial?: Parameters<typeof createPlayer>[1]["initial"]): Player {
  shared ??= createPlayer(songs, { initial });
  return shared;
}

const $ = <T extends Element = HTMLElement>(root: ParentNode, sel: string) => root.querySelector<T>(sel);
const $$ = <T extends Element = HTMLElement>(root: ParentNode, sel: string) => [...root.querySelectorAll<T>(sel)];

function setAll(root: ParentNode, sel: string, fn: (el: HTMLElement) => void) {
  for (const el of $$(root, sel)) fn(el);
}

const reduced = () => matchMedia("(prefers-reduced-motion: reduce)").matches;

function lineItems(song: Song): HTMLLIElement[] {
  const source = document.createElement("li");
  source.className = "source";
  source.textContent = lyricsSource(song);
  const items = song.lyrics.map((l) => {
    const li = document.createElement("li");
    const b = document.createElement("button");
    b.type = "button";
    b.className = "line";
    b.dataset.at = String(l.atMs);
    b.dataset.state = "next";
    b.textContent = l.line;
    li.append(b);
    return li;
  });
  return items.length ? [...items, source] : [];
}

/** A song row as the app's SongRow draws it, cloned from the Up next list's own markup. */
function songRow(template: HTMLElement | null, song: Song, current: boolean, playing: boolean): HTMLElement {
  const row = (template?.cloneNode(true) as HTMLElement | undefined) ?? document.createElement("button");
  row.dataset.play = song.id;
  row.dataset.row = song.id;
  row.removeAttribute("aria-label");
  if (current) row.setAttribute("aria-current", "true");
  else row.removeAttribute("aria-current");
  // Its own verb, not the template's: the current song's row pauses it while it plays.
  const verb = row.querySelector("[data-row-verb]");
  if (verb) verb.textContent = current && playing ? "Pause " : "Play ";
  const img = row.querySelector("img");
  if (img) {
    img.src = song.cover;
    img.loading = "eager";
  }
  const t = row.querySelector(".t");
  const a = row.querySelector(".a");
  const d = row.querySelector(".dur");
  if (t) t.textContent = song.title;
  if (a) a.textContent = song.artist;
  if (d) d.textContent = formatTime(song.durationMs);
  return row;
}

/**
 * Moves `el` from where it was (`from`) to where it is now, as one eased motion (FLIP).
 * The window is drawn scaled (the laptop's screen, Make it yours), so the distances measured
 * on screen are turned back into the element's own pixels before they're applied; otherwise
 * it would start part way and jump.
 */
function glideFrom(el: HTMLElement, from: DOMRect, duration: number) {
  const to = el.getBoundingClientRect();
  const frame = el.closest<HTMLElement>("[data-replica]") ?? el;
  if (!to.width || !from.width || !frame.offsetWidth || reduced()) return;
  // Screen pixels per pixel of the window (measured on the window, never on `el`, which may
  // still be mid-glide from a quick second toggle).
  const k = frame.getBoundingClientRect().width / frame.offsetWidth;
  const sx = from.width / to.width;
  const dx = (from.left - to.left) / k;
  const dy = (from.top - to.top) / k;
  if (Math.abs(dx) < 1 && Math.abs(dy) < 1 && Math.abs(sx - 1) < 0.01) return;
  el.animate(
    [
      { transformOrigin: "0 0", transform: `translate(${dx}px, ${dy}px) scale(${sx})` },
      { transformOrigin: "0 0", transform: "none" },
    ],
    { duration, easing: "cubic-bezier(0.77, 0, 0.175, 1)" },
  );
}

function bindReplica(root: HTMLElement, player: Player) {
  const desktop = root.closest<HTMLElement>("[data-desktop]");
  // Only a window on a desktop with a taskbar to come back from can minimise (the hero's);
  // the illustration (inert) and the bare window ("Make it yours") stay as they are, and
  // the bare window has no minimise button.
  const canMinimise = !!desktop?.querySelector("[data-taskbar-app]") && !root.parentElement?.closest("[inert]");
  if (!desktop) setAll(root, '[data-act="minimise"]', (el) => (el.hidden = true));
  const lines = $<HTMLOListElement>(root, "[data-lines]")!;
  const seek = $<HTMLInputElement>(root, "[data-seek]")!;
  const seekLine = $(root, "[data-seek-line]");
  const seekTip = $(root, "[data-seek-tip]");
  const thumb = $(root, "[data-thumb]");
  const searchInput = $<HTMLInputElement>(root, "[data-search-input]");
  const volume = $<HTMLInputElement>(root, "[data-volume]");
  const volumeWrap = $(root, "[data-volume-wrap]");
  const rowTemplate = $(root, ".queue [data-row]");
  const toast = $(root, "[data-toast]");
  const toastText = $(root, "[data-toast-text]");
  const npCover = $(root, ".np-cover");
  const npText = $(root, ".np-text");

  let last: {
    view?: View;
    song?: string | null;
    playing?: boolean;
    second?: number;
    line?: number;
    minimised?: boolean;
    lyricsOn?: boolean;
    theme?: string;
    query?: string;
    result?: string | null;
    liked?: boolean;
    shuffle?: boolean;
    repeat?: string;
    volume?: number;
    muted?: boolean;
    lyricsKey?: string;
    queueOpen?: boolean;
    sideOpen?: boolean;
    queueTab?: string;
    history?: string;
    likedList?: string;
    toast?: number | null;
  } = {};

  // The lines follow the song; the wheel or a finger over them moves them for a while.
  const lyricsPanel = $(root, "[data-lyrics]");
  const follower = lyricsPanel ? followLyrics(lyricsPanel, lines) : null;

  // --- What the visitor does ---------------------------------------------------------
  root.addEventListener("click", (e) => {
    const target = (e.target as Element).closest<HTMLElement>("[data-play], [data-act], [data-page], [data-tab], [data-chip], .line");
    if (!target || !root.contains(target)) return;
    // A click on a control with a tooltip hides the tooltip until the pointer leaves.
    const tipped = target.closest<HTMLElement>(".fp-tipped");
    if (tipped) {
      tipped.dataset.dismissed = "";
      tipped.addEventListener("pointerleave", () => delete tipped.dataset.dismissed, { once: true });
    }
    if (target.matches(".line")) {
      follower?.release();
      player.seek(Number(target.dataset.at));
      return;
    }
    if (target.dataset.page) {
      player.show(target.dataset.page as View);
      return;
    }
    if (target.dataset.tab) {
      player.setQueueTab(target.dataset.tab as "next" | "related");
      return;
    }
    if (target.dataset.chip) {
      const i = target.dataset.chip;
      setAll(root, "[data-chip]", (c) => c.setAttribute("aria-selected", String(c.dataset.chip === i)));
      setAll(root, "[data-chip-list]", (l) => (l.hidden = l.dataset.chipList !== i));
      return;
    }
    if (target.dataset.play) {
      // The current song's row toggles it, as the app's cover play button does.
      if (player.state.song?.id === target.dataset.play && target.matches(".row")) player.toggle();
      else player.play(target.dataset.play);
      return;
    }
    const s = player.state;
    switch (target.dataset.act) {
      case "home":
        player.show("home");
        break;
      case "back":
        player.back();
        break;
      case "nowplaying":
        if (isNowView(s.view)) player.back();
        else player.show("now");
        break;
      case "toggle":
        player.toggle();
        break;
      case "previous":
        player.previous();
        break;
      case "next":
        player.next();
        break;
      case "notforme":
        player.notForMe();
        break;
      case "undo":
        player.undo();
        break;
      case "lyrics":
        // No lyrics for this song: a press says so under the song (the app's Lyrics tab).
        if (s.song && s.song.lyrics.length === 0) {
          if (!isNowView(s.view)) player.show("now");
          setAll(root, "[data-no-lyrics]", (el) => (el.textContent = el.textContent ? "" : "No lyrics for this song"));
        } else player.show(s.view === "lyrics" ? "now" : "lyrics");
        break;
      case "queue":
        // On Now playing it hides and shows the side panel; over a page, Up next.
        if (isNowView(s.view)) player.toggleSide();
        else player.toggleQueue();
        break;
      case "source":
        // "Playing from": the demo's songs are a list of its own, so it opens Up next.
        player.setQueueTab("next");
        player.toggleSide(true);
        break;
      case "minimise":
        player.minimise();
        break;
      case "shuffle":
        player.toggleShuffle();
        break;
      case "repeat":
        player.cycleRepeat();
        break;
      case "like": {
        const wasLiked = !!s.song && s.liked.includes(s.song.id);
        player.toggleLike();
        // The heart pops when a like is added: the glyph at 0.8, back to 1 over t-fast.
        if (!wasLiked && !reduced()) {
          for (const svg of $$<SVGElement>(root, ".like svg")) {
            svg.animate([{ transform: "scale(0.8)" }, { transform: "scale(1)" }], {
              duration: 140,
              easing: "cubic-bezier(0.23, 1, 0.32, 1)",
            });
          }
        }
        break;
      }
      case "mute":
        // At zero, the button brings the sound back; otherwise it mutes and unmutes.
        if (s.volume === 0) player.setVolume(0.7);
        else player.toggleMute();
        break;
    }
  });

  $(root, "[data-search]")?.addEventListener("submit", (e) => {
    e.preventDefault();
    if (searchInput) player.search(searchInput.value);
  });
  // A pasted link (or anything pasted) searches at once, as the app does.
  searchInput?.addEventListener("paste", () => setTimeout(() => player.search(searchInput.value), 0));

  seek.addEventListener("input", () => player.seek(Number(seek.value)));
  // The time under the pointer, above the seek line.
  seekLine?.addEventListener("pointermove", (e) => {
    const song = player.state.song;
    if (!song || !seekTip) return;
    const r = seekLine.getBoundingClientRect();
    const f = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
    seekTip.textContent = formatTime(f * song.durationMs);
    const w = seekTip.offsetWidth;
    const local = f * seekLine.offsetWidth;
    seekTip.style.transform = `translateX(${Math.round(local - w / 2)}px)`;
  });

  volume?.addEventListener("input", () => player.setVolume(Number(volume.value) / 100));
  // The wheel over the slider: 5 % a notch, as the app's.
  volumeWrap?.addEventListener(
    "wheel",
    (e) => {
      if (!e.deltaY) return;
      e.preventDefault();
      const notch = e.deltaMode === 1 ? e.deltaY / 3 : e.deltaY / 100;
      player.setVolume(player.state.volume - notch * 0.05);
    },
    { passive: false },
  );

  // The toast leaves after a few seconds, unless the pointer rests on it.
  toast?.addEventListener("pointerenter", () => holdToast(true, player));
  toast?.addEventListener("pointerleave", () => holdToast(false, player));

  if (desktop) {
    $(desktop, "[data-taskbar-app]")?.addEventListener("click", () =>
      player.state.minimised ? player.restore() : player.minimise(),
    );
    $(desktop, "[data-tray-icon]")?.addEventListener("click", () => player.restore());
  }

  // --- Drawing the state ----------------------------------------------------------------
  const keepFocus = (fallback: string) => {
    const active = document.activeElement as HTMLElement | null;
    if (active && root.contains(active) && active.offsetParent === null) $(root, fallback)?.focus();
  };

  let instant = true; // the first placement doesn't glide in
  function glide(index: number) {
    const li = index >= 0 ? (lines.children[index] as HTMLElement | undefined) : undefined;
    follower?.follow(li ?? null, instant);
    instant = false;
  }

  function tooltip(s: PlayerState) {
    if (!desktop || !s.song) return;
    const at = formatTime(s.positionMs);
    const line2 = s.playing ? `${s.song.artist}, ${at} of ${formatTime(s.song.durationMs)}` : `${s.song.artist}, paused at ${at}`;
    setAll(desktop, '[data-tray-line="1"]', (el) => (el.textContent = s.song!.title));
    setAll(desktop, '[data-tray-line="2"]', (el) => (el.textContent = line2));
  }

  const fill = (list: HTMLElement | null, ids: string[], current: string | null) => {
    if (!list) return;
    list.replaceChildren(
      ...ids
        .map((id) => songs.find((s) => s.id === id))
        .filter((s): s is Song => !!s)
        .map((s) => songRow(rowTemplate, s, s.id === current, player.state.playing)),
    );
  };

  function render(s: PlayerState) {
    const song = s.song;
    const songChanged = (song?.id ?? null) !== last.song;

    if (s.view !== last.view) {
      // The lyrics toggle glides the cover and its words to their other place (t-lyrics,
      // ease-in-out), while the panel and the lines fade (replica.css).
      const toggling = isNowView(s.view) && last.view !== undefined && isNowView(last.view);
      const fromCover = toggling && npCover ? npCover.getBoundingClientRect() : null;
      const fromText = toggling && npText ? npText.getBoundingClientRect() : null;

      root.dataset.view = s.view;
      const browse = !isNowView(s.view);
      setAll(root, '[data-chrome="browse"]', (el) => (el.hidden = !browse));
      for (const pane of $$(root, "[data-pane]")) {
        const name = pane.dataset.pane;
        pane.hidden = name === "now" ? browse : name !== s.view;
      }
      setAll(root, "[data-page]", (el) =>
        el.dataset.page === s.view && !el.hasAttribute("data-liked-link")
          ? el.setAttribute("aria-current", "page")
          : el.removeAttribute("aria-current"),
      );
      keepFocus(browse ? ".side-item" : ".back");
      last.line = undefined; // re-glide once the lines are laid out again
      last.queueOpen = undefined;

      if (fromCover && npCover) glideFrom(npCover, fromCover, 300);
      if (fromText && npText) glideFrom(npText, fromText, 300);
    }

    // The lyrics toggles: pressed while the lyrics show. A song without lyrics keeps the cover
    // view and the toggles say "No lyrics", never pressed; the note under the song is only for
    // a press on them, and goes with the song.
    const hasLyrics = !!song && song.lyrics.length > 0;
    const lyricsKey = `${s.view}|${hasLyrics}`;
    if (lyricsKey !== last.lyricsKey) {
      const showing = s.view === "lyrics" && hasLyrics;
      setAll(root, "[data-lyrics-toggle]", (el) => el.setAttribute("aria-pressed", String(showing)));
      setAll(root, "[data-lyrics-tip]", (el) => (el.textContent = !hasLyrics ? "No lyrics" : showing ? "Hide lyrics" : "Lyrics"));
      last.lyricsKey = lyricsKey;
    }
    if (songChanged) setAll(root, "[data-no-lyrics]", (el) => (el.textContent = ""));

    if (s.lyricsOn !== last.lyricsOn) root.dataset.lyricsOn = String(s.lyricsOn);

    // The app's own theme ("Make it yours"): until one is picked, the window follows the page.
    const theme = s.themeSet ? s.theme : "";
    if (theme !== last.theme) {
      if (theme) root.dataset.theme = theme;
      else delete root.dataset.theme;
      last.theme = theme;
    }

    if (songChanged && song) {
      root.dataset.song = song.id;
      root.dataset.hasLyrics = String(song.lyrics.length > 0);
      root.style.setProperty("--song", song.colour);
      for (const img of $$<HTMLImageElement>(root, "[data-bar-cover], [data-np-cover], [data-resume-cover]")) {
        img.src = song.cover;
      }
      // Each demo song is a single of its own (as in the app's demo fixtures), so the album
      // line under the artist is its title.
      setAll(root, "[data-bar-title], [data-np-title], [data-resume-title], [data-np-album]", (el) => (el.textContent = song.title));
      setAll(root, "[data-bar-artist], [data-np-artist], [data-resume-artist]", (el) => (el.textContent = song.artist));
      setAll(root, "[data-bar-duration]", (el) => (el.textContent = formatTime(song.durationMs)));
      setAll(root, "[data-similar]", (el) => (el.textContent = `Similar to ${song.title}`));
      setAll(root, "[data-row]", (el) =>
        el.dataset.row === song.id ? el.setAttribute("aria-current", "true") : el.removeAttribute("aria-current"),
      );
      fill($(root, "[data-related-list]"), player.related().map((r) => r.id), song.id);
      seek.max = String(song.durationMs);
      lines.replaceChildren(...lineItems(song));
      follower?.release();
      // Back to the top for the new lines, then the first placement on them is instant (set
      // after, as glide() spends it).
      glide(-1);
      instant = true;
      last.line = undefined;
      last.second = undefined;
      last.liked = undefined;
      setGlow(song.colour);
    }

    if (s.playing !== last.playing || songChanged) {
      // The current row's button pauses it while it plays (as the app's cover button).
      for (const verb of $$(root, "[data-row-verb]")) {
        const row = verb.closest<HTMLElement>("[data-row]");
        verb.textContent = row?.dataset.row === song?.id && s.playing ? "Pause " : "Play ";
      }
    }
    if (s.playing !== last.playing) {
      root.dataset.playing = String(s.playing);
      setAll(root, "[data-play-label]", (el) => el.setAttribute("aria-label", s.playing ? "Pause" : "Play"));
      setAll(root, "[data-play-text]", (el) => (el.textContent = s.playing ? "Pause" : "Play"));
      last.second = undefined;
    }

    if (song) {
      const fraction = Math.min(1, s.positionMs / song.durationMs);
      setAll(root, "[data-progress]", (el) => (el.style.transform = `scaleX(${fraction})`));
      thumb?.style.setProperty("--at", String(fraction));
      const second = Math.floor(s.positionMs / 1000);
      if (second !== last.second) {
        const at = formatTime(s.positionMs);
        setAll(root, "[data-bar-time]", (el) => (el.textContent = at));
        setAll(root, "[data-resume-time]", (el) => (el.textContent = `${at} of ${formatTime(song.durationMs)}`));
        seek.value = String(s.positionMs);
        seek.setAttribute("aria-valuetext", `${at} of ${formatTime(song.durationMs)}`);
        tooltip(s);
        last.second = second;
      }
      const line = currentLineIndex(song, s.positionMs);
      if (line !== last.line) {
        $$<HTMLButtonElement>(lines, ".line").forEach((b, i) => {
          b.dataset.state = i < line ? "past" : i === line ? "current" : "next";
          if (i === line) b.setAttribute("aria-current", "true");
          else b.removeAttribute("aria-current");
        });
        if (s.view === "lyrics") glide(line);
        last.line = line;
      }
      const liked = s.liked.includes(song.id);
      if (liked !== last.liked) {
        setAll(root, '[data-act="like"]', (el) => {
          el.setAttribute("aria-pressed", String(liked));
          el.setAttribute("aria-label", liked ? "Remove like" : "Like");
        });
        last.liked = liked;
      }
    }

    if (s.shuffle !== last.shuffle) {
      setAll(root, '[data-act="shuffle"]', (el) => el.setAttribute("aria-pressed", String(s.shuffle)));
      last.shuffle = s.shuffle;
    }
    if (s.repeat !== last.repeat) {
      setAll(root, '[data-act="repeat"]', (el) => {
        el.dataset.repeat = s.repeat;
        el.setAttribute("aria-pressed", String(s.repeat !== "off"));
        el.setAttribute("aria-label", s.repeat === "one" ? "Repeat one" : s.repeat === "all" ? "Repeat all" : "Repeat off");
      });
      last.repeat = s.repeat;
    }

    if (s.volume !== last.volume || s.muted !== last.muted) {
      const silent = s.muted || s.volume === 0;
      root.dataset.muted = String(silent);
      volumeWrap?.style.setProperty("--vol", String(s.volume));
      if (volume && Number(volume.value) !== Math.round(s.volume * 100)) volume.value = String(Math.round(s.volume * 100));
      volume?.setAttribute("aria-valuetext", silent ? "Muted" : `${Math.round(s.volume * 100)} %`);
      setAll(root, '[data-act="mute"]', (el) => el.setAttribute("aria-label", silent ? "Unmute" : "Mute"));
      last.volume = s.volume;
      last.muted = s.muted;
    }

    if (s.queueOpen !== last.queueOpen || s.sideOpen !== last.sideOpen) {
      root.dataset.queueOpen = String(s.queueOpen);
      root.dataset.sideOpen = String(s.sideOpen);
      const shown = isNowView(s.view) ? s.sideOpen : s.queueOpen;
      setAll(root, "[data-queue]", (el) => (el.hidden = !shown));
      setAll(root, '[data-act="queue"]', (el) => el.setAttribute("aria-pressed", String(shown)));
      last.queueOpen = s.queueOpen;
      last.sideOpen = s.sideOpen;
    }
    if (s.queueTab !== last.queueTab) {
      root.dataset.queueTab = s.queueTab;
      setAll(root, "[data-tab]", (el) => el.setAttribute("aria-selected", String(el.dataset.tab === s.queueTab)));
      setAll(root, "[data-tab-panel]", (el) => (el.hidden = el.dataset.tabPanel !== s.queueTab));
      last.queueTab = s.queueTab;
    }

    const history = s.history.join();
    if (history !== last.history) {
      fill($(root, "[data-history-list]"), s.history, song?.id ?? null);
      last.history = history;
    }
    const likedList = s.liked.join() + "|" + (song?.id ?? "");
    if (likedList !== last.likedList) {
      fill($(root, "[data-liked-list]"), s.liked, song?.id ?? null);
      setAll(root, "[data-liked-empty]", (el) => (el.hidden = s.liked.length > 0));
      last.likedList = likedList;
    }

    if ((s.toast?.id ?? null) !== last.toast) {
      if (toast && toastText) {
        toast.hidden = !s.toast;
        if (s.toast) toastText.textContent = s.toast.text;
      }
      last.toast = s.toast?.id ?? null;
    }

    if (canMinimise && s.minimised !== last.minimised) {
      root.toggleAttribute("data-minimised", s.minimised);
      root.inert = s.minimised;
      desktop?.toggleAttribute("data-minimised", s.minimised);
    }

    if (s.view === "search" && (s.query !== last.query || (s.result?.id ?? null) !== last.result)) {
      const box = $(root, "[data-search-result]");
      const empty = $(root, "[data-search-empty]");
      box?.replaceChildren(...(s.result ? [songRow(rowTemplate, s.result, s.result.id === song?.id, s.playing)] : []));
      if (empty) empty.hidden = !!s.result || !s.query;
      if (searchInput && searchInput.value !== s.query) searchInput.value = s.query;
      last.query = s.query;
      last.result = s.result?.id ?? null;
    }

    last.view = s.view;
    last.song = song?.id ?? null;
    last.playing = s.playing;
    last.minimised = s.minimised;
    last.lyricsOn = s.lyricsOn;
  }

  player.subscribe(render);
}

let bound = false;

export function bindReplicas(): Player | null {
  const roots = $$<HTMLElement>(document, "[data-replica]");
  if (!roots.length) return null;
  const first = roots[0];
  const player = sharedPlayer({
    songId: first.dataset.song,
    positionMs: Number(first.dataset.at ?? 0),
    view: (first.dataset.view as View) ?? "home",
    playing: false,
  });
  if (bound) return player;
  bound = true;
  // The hero's window is wired at once; the others (Make it yours, the features' picture)
  // when they come within a screen of the viewport or, failing that, one per idle moment,
  // so the first screen's start-up doesn't pay for windows nobody can see yet. Until then
  // they show the server's frame of the same state, and on binding they catch up with the
  // shared player (subscribe renders the current state at once).
  const wired = new WeakSet<Element>();
  const wire = (root: HTMLElement) => {
    if (wired.has(root)) return;
    wired.add(root);
    near?.unobserve(root);
    bindReplica(root, player);
  };
  const [hero, ...rest] = roots;
  const near =
    rest.length && "IntersectionObserver" in window
      ? new IntersectionObserver(
          (entries) => {
            for (const e of entries) if (e.isIntersecting) wire(e.target as HTMLElement);
          },
          { rootMargin: "100% 0px" },
        )
      : null;
  wire(hero);
  for (const root of rest) {
    if (near) near.observe(root);
    else wire(root);
  }
  const idle = (fn: () => void) =>
    "requestIdleCallback" in window ? requestIdleCallback(fn, { timeout: 4000 }) : setTimeout(fn, 1500);
  const queue = near ? [...rest] : [];
  const step = () => {
    const root = queue.shift();
    if (root) wire(root);
    if (queue.length) idle(step);
    else near?.disconnect();
  };
  if (queue.length) idle(step);
  const live = liveRegion();
  let toastId: number | null = null;
  player.subscribe((s) => {
    const id = s.toast?.id ?? null;
    if (id === toastId) return;
    toastId = id;
    live.textContent = s.toast?.text ?? "";
    if (s.toast) armToast(player);
    else {
      // Gone (Undo, or its time): a pointer that rested on it can't leave it any more.
      clearTimeout(toastTimer);
      toastHeld = false;
    }
  });
  bindAudio(player);
  bindCredits(player);
  return player;
}
