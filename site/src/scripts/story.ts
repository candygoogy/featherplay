// The scroll story (Stage.astro): with the stage pinned for 2,400 px, the window steps back as
// the app does: whole (centred and large), narrow (the app's real narrow layout, beside its
// words), into the tray (the capsule's song, its flyout showing the song still playing). The
// page settles into each hold. Only where it can be seen whole: a pointer-sized screen
// (761 px wide, 600 tall), motion allowed; elsewhere the three steps are plain blocks.
//
// The loop runs only while something moves (the scroll's smoothing settling), reads no layout
// (geometry is measured on resize and when the window lays out) and moves the window, its
// ground and the words by transform and opacity. The change of shape is Windows' snap: the
// app fades, its ground carries its icon from the whole rect to the narrow one, the app is laid
// out narrow (once, while hidden) and fades back in; the snap outline marks where it goes.
// A window the visitor dragged, maximised or sent to the tray joins in from where it is.

import type { Player } from "../demo/player";
import { whenWindow, type Frame } from "../demo/window";
import { fitWindow, IDEAL } from "../demo/fit";
import { LYRICS_MIN } from "./first-math";
import { HOLDS, captionOpacity, phases, seg, settleTarget, storyProgress } from "./story-math";
import { hideNote, showNoteAt } from "./notes";

export const STORY = "(min-width: 761px) and (min-height: 600px) and (prefers-reduced-motion: no-preference)";
/** The pinned distance (Stage.astro --story-len). */
const LEN = 2400;
/** The narrow window, in the app's px: its real least width, and a phone-like height. */
const NARROW = { w: 480, h: 660 };
/** The capsule's foot: the story's pictures stay below it. */
const CAP_B = 60;

type Rect = { x: number; y: number; w: number; h: number };
const lerp = (a: number, b: number, t: number) => a + (b - a) * t;

export async function startStory(player: Player): Promise<void> {
  const root = document.documentElement;
  const mq = matchMedia(STORY);
  const stage = document.querySelector<HTMLElement>("[data-stage]");
  const pin = stage?.querySelector<HTMLElement>("[data-pin]");
  if (!stage || !pin) return;
  // The story's script is here: the pinned stage boot.js promised is kept.
  root.toggleAttribute("data-story-live", true);
  const win = await whenWindow;
  const hero = [...stage.querySelectorAll<HTMLElement>(".first > .head, .first > .col, .first > [data-shelf], .first > .hand")];
  const caps = [...stage.querySelectorAll<HTMLElement>("[data-cap]")];
  const ghost = stage.querySelector<HTMLElement>("[data-ghost]")!;
  const icon = stage.querySelector<HTMLElement>("[data-ghost-icon]")!;
  const snap = stage.querySelector<HTMLElement>("[data-snap]")!;
  const songFly = document.getElementById("fly-song");
  const stillNote = stage.querySelector<HTMLElement>("[data-note='playing']");
  const steps = document.querySelector<HTMLElement>("[data-steps]");
  /** The story's flyout goes when its beat ends, or the story does. */
  const endBeat = () => {
    if (stillNote) hideNote(stillNote);
    if (!songFly?.hasAttribute("data-story")) return;
    delete songFly.dataset.story;
    if (songFly.matches(":popover-open")) songFly.hidePopover();
  };

  let G: {
    top: number;
    L: { w: number; h: number };
    r1: Rect & { s: number };
    r2: Rect & { s: number };
    caps: { x: number; y: number }[];
  } | null = null;

  function measure() {
    if (!mq.matches) return;
    const vw = root.clientWidth;
    const vh = innerHeight;
    const top = stage!.getBoundingClientRect().top + scrollY;
    // The frame's left edge: the composition's (first.ts), else where the first screen sits.
    const first = stage!.querySelector<HTMLElement>(".first")!;
    const fl = Number(first.dataset.fl) || first.getBoundingClientRect().left;
    // Hold 1: the whole window, centred and large, its words under it, as wide as the window but
    // never narrower than a comfortable measure (measured at the width they will have: narrower,
    // they take another line, and the window gives way to them). The whole wide app wherever it
    // reads there (lyrics at 13 px or more), even where the first screen showed the narrow one;
    // else the first screen's own layout.
    const side = Math.max(fl, 40);
    const capMin = Math.min(vw - 2 * side, 416);
    const holdFor = (L: { w: number; h: number }) => {
      let w1 = Math.min(vw - 2 * side, L.w);
      let capW = w1;
      let roomH = 0;
      let s1 = 1;
      for (let i = 0; i < 3; i++) {
        capW = Math.max(capMin, w1);
        caps[0].style.width = `${Math.round(capW)}px`;
        roomH = vh - CAP_B - 20 - caps[0].offsetHeight - 28 - 24;
        s1 = Math.min(1, (vw - 2 * side) / L.w, roomH / L.h);
        if (Math.abs(L.w * s1 - w1) < 1) break;
        w1 = L.w * s1;
      }
      return { capW, roomH, s1 };
    };
    let L = { w: IDEAL.w, h: IDEAL.h };
    let { capW, roomH, s1 } = holdFor(L);
    if (24 * s1 < LYRICS_MIN && (win.home.w !== L.w || win.home.h !== L.h)) {
      L = { w: win.home.w, h: win.home.h };
      ({ capW, roomH, s1 } = holdFor(L));
    }
    let w1 = L.w * s1;
    const h1 = L.h * s1;
    if (Math.round(Math.max(capMin, w1)) !== Math.round(capW)) {
      capW = Math.max(capMin, w1);
      caps[0].style.width = `${Math.round(capW)}px`;
    }
    const r1 = { x: (vw - w1) / 2, y: CAP_B + 20 + Math.max(0, (roomH - h1) / 2), w: w1, h: h1, s: s1 };
    // Hold 2: the narrow window beside its words.
    const capW1 = caps[1].offsetWidth;
    const f2 = fitWindow({ w: Math.max(240, vw - fl - capW1 - 96 - 24), h: vh - CAP_B - 56 }, { want: NARROW });
    const W2 = NARROW.w * f2.s;
    const H2 = NARROW.h * f2.s;
    const x2 = Math.max(fl + capW1 + 96, vw * 0.52 - W2 / 2);
    const r2 = { x: Math.min(x2, vw - W2 - 24), y: CAP_B + (vh - CAP_B - H2) / 2, w: W2, h: H2, s: f2.s };
    G = {
      top,
      L,
      r1,
      r2,
      caps: [
        { x: (vw - capW) / 2, y: r1.y + h1 + 28 },
        { x: fl, y: r2.y + H2 / 2 - caps[1].offsetHeight / 2 },
        { x: fl, y: CAP_B + (vh - CAP_B) / 2 - caps[2].offsetHeight / 2 },
      ],
    };
    ghost.style.width = `${w1}px`;
    ghost.style.height = `${h1}px`;
    Object.assign(snap.style, { width: `${W2}px`, height: `${H2}px`, transform: `translate(${r2.x}px, ${r2.y}px)` });
    target = storyProgress(scrollY, G.top, LEN);
    cur = target;
    trayAt = null;
    render(cur, true);
    // Laid out part-way through a move (a reload, a resize there): finished the way it was going.
    clearTimeout(settleTimer);
    settleTimer = window.setTimeout(settle, 160);
  }

  // ---- One picture of the story at `p` ----
  let userMin: { p: number } | null = null;
  let trayAt: { x: number; y: number } | null = null;
  let beat = false;
  let lastP = -1;
  function render(p: number, force = false) {
    if (!G) return;
    if (p === lastP && !force) return;
    lastP = p;
    const { a, b, c } = phases(p);
    // The beat in the tray: the capsule's flyout shows the song, still playing.
    const inBeat = p >= 0.76 && p < 0.995 && c >= 1;
    if (inBeat !== beat && songFly) {
      beat = inBeat;
      if (inBeat && !songFly.matches(":popover-open")) {
        // Marked first: the bar places the story's flyout under the capsule's song.
        songFly.dataset.story = "";
        songFly.showPopover();
        // "still playing" beside it, pointing back at it, where its words have room.
        if (stillNote)
          requestAnimationFrame(() => {
            if (!beat || !songFly.matches(":popover-open")) return;
            const f = songFly.getBoundingClientRect();
            showNoteAt(stillNote, f.right + 12, f.top + 36);
          });
      } else if (!inBeat) endBeat();
    }
    // The first screen's words leave first (keys that were in them go on from the steps).
    const out = seg(p, 0, 0.07);
    for (const el of hero) {
      el.style.opacity = out ? String(1 - out * out * (3 - 2 * out)) : "";
      el.style.translate = out ? `0 ${(-out * 12).toFixed(1)}px` : "";
      const gone = out > 0.5;
      if (gone && !el.inert && el.contains(document.activeElement)) steps?.focus({ preventScroll: true });
      el.inert = gone;
      // Gone, its live parts (the playing sleeve's mark) hold still.
      el.toggleAttribute("data-away", gone);
    }
    caps.forEach((cap, i) => {
      const o = captionOpacity(i, p);
      cap.style.opacity = o.toFixed(3);
      cap.style.visibility = o > 0.01 ? "visible" : "hidden";
      cap.style.transform = `translate(${G!.caps[i].x}px, ${(G!.caps[i].y + (1 - o) * 8).toFixed(1)}px)`;
    });

    // A window in the tray by the visitor's hand comes back once the page moves on.
    if (win.minimised) {
      if (!userMin) userMin = { p };
      else if (Math.abs(p - userMin.p) > 0.05) {
        userMin = null;
        player.restore();
      }
      if (win.minimised) return;
    } else userMin = null;

    if (p <= 0) {
      win.drive(null);
      ghost.style.opacity = "0";
      snap.style.opacity = "0";
      return;
    }
    // Leaving the first screen, the visitor's placement (dragged, maximised) is carried along;
    // once the window is whole in hold 1 it is forgotten, and the first screen gets it home.
    if (a >= 1) win.forget();
    const from = win.rested;
    const L = G.L;
    let f: Frame;
    let ghostO = 0;
    let gr: Rect = G.r1;
    // The snap's progress, in whichever move changes the app's layout (for the icon's fade).
    let k = b;
    if (b <= 0 && (from.w !== L.w || from.h !== L.h)) {
      // The first screen's narrow window becoming the whole wide app: as the snap to narrow,
      // the app fades, its ground carries its icon from the first screen's rect to hold 1's,
      // the app is laid out wide once while hidden and fades back.
      k = a;
      const r0 = { x: from.x, y: from.y, w: from.w * from.s, h: from.h * from.s };
      gr = { x: lerp(r0.x, G.r1.x, a), y: lerp(r0.y, G.r1.y, a), w: lerp(r0.w, G.r1.w, a), h: lerp(r0.h, G.r1.h, a) };
      ghostO = a > 0 ? Math.min(seg(a, 0, 0.2), 1 - seg(a, 0.85, 1)) : 0;
      f =
        a < 0.5
          ? { w: from.w, h: from.h, x: gr.x, y: gr.y, s: gr.w / from.w, o: 1 - seg(a, 0, 0.35) }
          : { ...G.r1, w: L.w, h: L.h, o: seg(a, 0.8, 1) };
    } else if (b <= 0) {
      f = { w: L.w, h: L.h, x: lerp(from.x, G.r1.x, a), y: lerp(from.y, G.r1.y, a), s: lerp(from.s, G.r1.s, a), o: 1 };
    } else if (c <= 0) {
      // The snap: the app fades, its ground carries its icon to the narrow rect, the app comes
      // back laid out narrow.
      gr = { x: lerp(G.r1.x, G.r2.x, b), y: lerp(G.r1.y, G.r2.y, b), w: lerp(G.r1.w, G.r2.w, b), h: lerp(G.r1.h, G.r2.h, b) };
      ghostO = Math.min(seg(b, 0, 0.2), 1 - seg(b, 0.85, 1));
      // The rect gives the place and the scale; the size the app is laid out at is its own.
      f = b < 0.5 ? { ...G.r1, w: L.w, h: L.h, o: 1 - seg(b, 0, 0.35) } : { ...G.r2, w: NARROW.w, h: NARROW.h, o: seg(b, 0.8, 1) };
    } else {
      // Into the tray: toward the capsule's song, smaller, then gone (where the song is, read
      // once as the move starts, not every frame).
      const t = (trayAt ??= win.trayPoint());
      const s = G.r2.s * lerp(1, 0.2, c);
      const cx = lerp(G.r2.x + G.r2.w / 2, t.x, c);
      const cy = lerp(G.r2.y + G.r2.h / 2, t.y, c);
      f = { w: NARROW.w, h: NARROW.h, x: cx - (NARROW.w * s) / 2, y: cy - (NARROW.h * s) / 2, s, o: 1 - seg(c, 0.68, 1) };
    }
    if (c <= 0) trayAt = null;
    win.drive(f);
    ghost.style.opacity = ghostO.toFixed(3);
    if (ghostO > 0) {
      ghost.style.transform = `translate(${gr.x.toFixed(1)}px, ${gr.y.toFixed(1)}px) scale(${(gr.w / G.r1.w).toFixed(4)}, ${(gr.h / G.r1.h).toFixed(4)})`;
      icon.style.transform = `translate(${(gr.x + gr.w / 2 - 28).toFixed(1)}px, ${(gr.y + gr.h / 2 - 28).toFixed(1)}px)`;
    }
    icon.style.opacity = (ghostO * Math.min(seg(k, 0.15, 0.35), 1 - seg(k, 0.7, 0.85))).toFixed(3);
    const bRaw = seg(p, 0.27, 0.5);
    snap.style.opacity = (seg(bRaw, 0, 0.2) * (1 - seg(bRaw, 0.72, 1))).toFixed(3);
  }

  // ---- Scroll: a target from scrollY, smoothed; the loop sleeps when it arrives ----
  let target = 0;
  let cur = 0;
  let raf = 0;
  let last = 0;
  let lastY = scrollY;
  let dir = 1;
  const frame = (now: number) => {
    raf = 0;
    const dt = Math.min(64, now - (last || now));
    last = now;
    // A jump of more than a third of the story (a section link) lands at once.
    if (Math.abs(target - cur) > 0.35) cur = target;
    cur += (target - cur) * (1 - Math.exp(-dt / 90));
    if (Math.abs(target - cur) < 0.0002) cur = target;
    render(cur);
    if (cur !== target) raf = requestAnimationFrame(frame);
    else last = 0;
  };
  const wake = () => {
    if (!raf) raf = requestAnimationFrame(frame);
  };
  let settleTimer = 0;
  let settlingUntil = 0;
  const settle = () => {
    if (!G) return;
    // The page's own settling glide is under way: ask again once it has had its time.
    const wait = settlingUntil - performance.now();
    if (wait > 0) {
      clearTimeout(settleTimer);
      settleTimer = window.setTimeout(settle, wait + 40);
      return;
    }
    const to = settleTarget(target, dir);
    if (to == null) return;
    settlingUntil = performance.now() + 600;
    scrollTo({ top: Math.round(G.top + to * LEN), behavior: "smooth" });
  };
  addEventListener(
    "scroll",
    () => {
      if (!G) return;
      const y = scrollY;
      if (y !== lastY) dir = y > lastY ? 1 : -1;
      lastY = y;
      target = storyProgress(y, G.top, LEN);
      clearTimeout(settleTimer);
      settleTimer = window.setTimeout(settle, 160);
      wake();
    },
    { passive: true },
  );
  addEventListener("scrollend", () => {
    clearTimeout(settleTimer);
    settleTimer = window.setTimeout(settle, 40);
  });

  // "The app" in the bar goes to the first hold, the whole window, and the keys go there too.
  document.addEventListener("click", (e) => {
    const a = (e.target as Element).closest?.('a[href="#app"]');
    if (!a || !G) return;
    e.preventDefault();
    scrollTo({ top: Math.round(G.top + HOLDS[0] * LEN), behavior: "smooth" });
    stage!.focus({ preventScroll: true });
  });

  // ---- On and off with the screen's size and the visitor's motion setting ----
  const apply = () => {
    root.toggleAttribute("data-story", mq.matches);
    if (mq.matches) requestAnimationFrame(measure);
    else {
      G = null;
      beat = false;
      endBeat();
      win.drive(null);
      for (const el of [...hero, ...caps, ghost, snap, icon]) el.removeAttribute("style");
      for (const el of hero) el.inert = false;
    }
  };
  mq.addEventListener("change", apply);
  win.onLayout(() => requestAnimationFrame(measure));
  apply();
}
