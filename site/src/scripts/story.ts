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
import { fitWindow } from "../demo/fit";
import { HOLDS, captionOpacity, phases, seg, settleTarget, storyProgress } from "./story-math";

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
  const win = await whenWindow;
  const hero = [...stage.querySelectorAll<HTMLElement>(".first > .head, .first > .col")];
  const caps = [...stage.querySelectorAll<HTMLElement>("[data-cap]")];
  const ghost = stage.querySelector<HTMLElement>("[data-ghost]")!;
  const icon = stage.querySelector<HTMLElement>("[data-ghost-icon]")!;
  const snap = stage.querySelector<HTMLElement>("[data-snap]")!;
  const songFly = document.getElementById("fly-song");

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
    const frame = stage!.querySelector<HTMLElement>(".first")!.getBoundingClientRect();
    const fl = frame.left;
    const L = { w: win.home.w, h: win.home.h };
    // Hold 1: the whole window, centred and large, its words under it.
    const side = Math.max(fl, 40);
    caps[0].style.width = `${Math.round(Math.min(vw - 2 * side, L.w))}px`;
    const capH0 = caps[0].offsetHeight;
    const roomH = vh - CAP_B - 20 - capH0 - 28 - 24;
    const s1 = Math.min(1, (vw - 2 * side) / L.w, roomH / L.h);
    const w1 = L.w * s1;
    const h1 = L.h * s1;
    caps[0].style.width = `${Math.round(w1)}px`;
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
        { x: r1.x, y: r1.y + h1 + 28 },
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
        songFly.showPopover();
        songFly.dataset.story = "";
      } else if (!inBeat && songFly.hasAttribute("data-story")) {
        delete songFly.dataset.story;
        if (songFly.matches(":popover-open")) songFly.hidePopover();
      }
    }
    // The first screen's words leave first.
    const out = seg(p, 0, 0.07);
    for (const el of hero) {
      el.style.opacity = out ? String(1 - out * out * (3 - 2 * out)) : "";
      el.style.translate = out ? `0 ${(-out * 12).toFixed(1)}px` : "";
      el.inert = out > 0.5;
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
    if (b <= 0) {
      f = { w: L.w, h: L.h, x: lerp(from.x, G.r1.x, a), y: lerp(from.y, G.r1.y, a), s: lerp(from.s, G.r1.s, a), o: 1 };
      if (from.w !== L.w || from.h !== L.h) f = { ...f, w: from.w, h: from.h };
    } else if (c <= 0) {
      // The snap: the app fades, its ground carries its icon to the narrow rect, the app comes
      // back laid out narrow.
      gr = { x: lerp(G.r1.x, G.r2.x, b), y: lerp(G.r1.y, G.r2.y, b), w: lerp(G.r1.w, G.r2.w, b), h: lerp(G.r1.h, G.r2.h, b) };
      ghostO = Math.min(seg(b, 0, 0.2), 1 - seg(b, 0.85, 1));
      f = b < 0.5 ? { w: L.w, h: L.h, ...G.r1, o: 1 - seg(b, 0, 0.35) } : { w: NARROW.w, h: NARROW.h, ...G.r2, o: seg(b, 0.8, 1) };
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
    icon.style.opacity = (ghostO * Math.min(seg(b, 0.15, 0.35), 1 - seg(b, 0.7, 0.85))).toFixed(3);
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

  // "The app" in the bar goes to the first hold, the whole window.
  document.addEventListener("click", (e) => {
    const a = (e.target as Element).closest?.('a[href="#app"]');
    if (!a || !G) return;
    e.preventDefault();
    scrollTo({ top: Math.round(G.top + HOLDS[0] * LEN), behavior: "smooth" });
  });

  // ---- On and off with the screen's size and the visitor's motion setting ----
  const apply = () => {
    root.toggleAttribute("data-story", mq.matches);
    if (mq.matches) requestAnimationFrame(measure);
    else {
      G = null;
      win.drive(null);
      for (const el of [...hero, ...caps, ghost, snap, icon]) el.removeAttribute("style");
      for (const el of hero) el.inert = false;
    }
  };
  mq.addEventListener("change", apply);
  win.onLayout(() => requestAnimationFrame(measure));
  apply();
}
