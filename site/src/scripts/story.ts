// The scroll story (Stage.astro): with the stage pinned for 2,400 px, the window steps back as
// the app does: whole (centred and large), narrow (the app's real narrow layout, beside its
// words), into the tray (the capsule's song, its flyout showing the song still playing). The
// page settles into each hold: a scroll that stops part-way through a move is carried on by
// the page's own glide (its own clock and curve, so the move has time to be seen and lands
// softly), until the visitor scrolls again. Only where it can be seen whole: a pointer-sized
// screen (761 px wide, 600 tall), motion allowed; elsewhere the three steps are plain blocks.
//
// The loop runs only while something moves (the scroll's smoothing settling), reads no layout
// (geometry is measured on resize and when the window lays out) and moves the window, its
// ground and the words by transform and opacity. The change of shape is a window resizing
// (story-morph.ts): the frame goes from the whole rect to the narrow one with the title bar,
// the cover and the player bar on screen all the way, the app laid out narrow once as the move
// begins and its words swapping in for the whole one's.
// A window the visitor dragged, maximised or sent to the tray joins in from where it is.

import type { Player } from "../demo/player";
import { whenWindow, type Frame } from "../demo/window";
import { fitWindow, IDEAL } from "../demo/fit";
import { LYRICS_MIN } from "./first-math";
import { HOLDS, captionOpacity, easeGlide, glidePath, phases, seg, settleTarget, storyProgress } from "./story-math";
import { hideNote, showNoteAt } from "./notes";
import { morpher } from "./story-morph";

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
  // The notes only fade: their place is their translate (src/scripts/notes.ts showNoteAt).
  const hand = (el: HTMLElement) => el.classList.contains("hand");
  const caps = [...stage.querySelectorAll<HTMLElement>("[data-cap]")];
  const ghost = stage.querySelector<HTMLElement>("[data-ghost]")!;
  const morph = morpher(win, ghost);
  const songFly = document.getElementById("fly-song");
  const stillNote = stage.querySelector<HTMLElement>("[data-note='playing']");
  const steps = document.querySelector<HTMLElement>("[data-steps]");
  /** The story's flyout goes when its beat ends, or the story does. */
  const endBeat = () => {
    if (stillNote) hideNote(stillNote);
    if (!songFly?.hasAttribute("data-story")) return;
    delete songFly.dataset.story;
    if (!songFly.matches(":popover-open")) return;
    // Back into the capsule's song it came out of (Bar.astro: data-into), which takes it.
    songFly.hidePopover();
    const chip = document.querySelector<HTMLElement>("[data-site-bar] [data-np]");
    if (chip?.getClientRects().length) {
      chip.toggleAttribute("data-caught", true);
      setTimeout(() => chip.removeAttribute("data-caught"), 800);
    }
  };

  let G: {
    top: number;
    L: { w: number; h: number };
    r1: Rect & { s: number };
    r2: Rect & { s: number };
    caps: { x: number; y: number }[];
    /** Caption 3 kept clear of the tray's flyout (fitTrayWords), for this measure. */
    trayFit?: boolean;
  } | null = null;

  // The tray's hold: the song's flyout opens under the capsule's song (bar.ts), and on a narrow
  // screen it would cover the hold's words. Before they show, they are kept to the room left of
  // where the flyout will be (worked out as bar.ts places it, from the capsule as it is now).
  const siteBar = document.querySelector<HTMLElement>("[data-site-bar]");
  const np = siteBar?.querySelector<HTMLElement>("[data-np]");
  function fitTrayWords() {
    // (Only from the capsule: the flyout opens under it. A jump straight into the story can
    // come before the bar has become one; then it is done as it does, below.)
    if (!G || G.trayFit || siteBar?.dataset.state !== "capsule") return;
    G.trayFit = true;
    const cap = caps[2];
    if (!np || !songFly) return;
    const vw = root.clientWidth;
    const w = Math.min(22 * 16, vw - 16);
    // Where the capsule's song rests (less the glide it may be in, as bar.ts reads it).
    const r = np.getBoundingClientRect();
    const t = getComputedStyle(np.closest("[data-part]") ?? np).translate;
    const dx = t === "none" ? 0 : parseFloat(t) || 0;
    const left = Math.max(8, Math.min(r.left - dx + r.width / 2 - w / 2, vw - w - 8));
    const room = Math.floor(left - 24 - G.caps[2].x);
    if (room >= cap.offsetWidth) return;
    cap.style.width = `${Math.max(200, room)}px`;
    G.caps[2].y = CAP_B + (innerHeight - CAP_B) / 2 - cap.offsetHeight / 2;
  }

  function measure() {
    if (!mq.matches) return;
    const vw = root.clientWidth;
    const vh = innerHeight;
    const top = stage!.getBoundingClientRect().top + scrollY;
    // The frame's left edge: the composition's (first.ts), else where the first screen sits.
    const first = stage!.querySelector<HTMLElement>(".first")!;
    const fl = Number(first.dataset.fl) || first.getBoundingClientRect().left;
    caps[2].style.width = "";
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
    let h1 = L.h * s1;
    if (Math.round(Math.max(capMin, w1)) !== Math.round(capW)) {
      capW = Math.max(capMin, w1);
      caps[0].style.width = `${Math.round(capW)}px`;
    }
    let r1 = { x: (vw - w1) / 2, y: CAP_B + 20 + Math.max(0, (roomH - h1) / 2), w: w1, h: h1, s: s1 };
    let cap0 = { x: (vw - capW) / 2, y: r1.y + h1 + 28 };
    const capW1 = caps[1].offsetWidth;
    // A short screen (1024 x 600): under its words the whole app came out smaller than the
    // narrow one of the next hold. There its words go beside it, as the next hold's do, when
    // that gives the window more room.
    const beside = (L: { w: number; h: number }) => Math.min(1, (vw - fl - capW1 - 96 - 24) / L.w, (vh - CAP_B - 56) / L.h);
    let Ls = { w: IDEAL.w, h: IDEAL.h };
    let ss = beside(Ls);
    if (24 * ss < LYRICS_MIN) {
      Ls = { w: win.home.w, h: win.home.h };
      ss = beside(Ls);
    }
    if (Ls.w * Ls.h * ss * ss > w1 * h1 * 1.1) {
      L = Ls;
      s1 = ss;
      w1 = L.w * s1;
      h1 = L.h * s1;
      caps[0].style.width = `${capW1}px`;
      const x = Math.max(fl + capW1 + 96, vw * 0.52 - w1 / 2);
      r1 = { x: Math.min(x, vw - w1 - 24), y: CAP_B + (vh - CAP_B - h1) / 2, w: w1, h: h1, s: s1 };
      cap0 = { x: fl, y: r1.y + h1 / 2 - caps[0].offsetHeight / 2 };
    }
    // Hold 2: the narrow window beside its words.
    const f2 = fitWindow({ w: Math.max(240, vw - fl - capW1 - 96 - 24), h: vh - CAP_B - 56 }, { want: NARROW });
    // A large screen: the narrow window grows with the screen's height, to about 70 % of it (a
    // scale of 1.4 at most, smaller than the whole one before it), and its words and it are one
    // group in the middle of the frame, the words 96 px from it. Elsewhere as it was.
    let s2 = f2.s;
    let capX1 = fl;
    const grow = Math.min(1.4, (vh * 0.7) / NARROW.h);
    const large = vw >= 1600 && grow > s2 && NARROW.w * grow * NARROW.h * grow < w1 * h1;
    if (large) s2 = grow;
    const W2 = NARROW.w * s2;
    const H2 = NARROW.h * s2;
    let x2 = Math.max(fl + capW1 + 96, vw * 0.52 - W2 / 2);
    if (large) {
      capX1 = fl + Math.max(0, (vw - 2 * fl - (capW1 + 96 + W2)) / 2);
      x2 = capX1 + capW1 + 96;
    }
    const r2 = { x: Math.min(x2, vw - W2 - 24), y: CAP_B + (vh - CAP_B - H2) / 2, w: W2, h: H2, s: s2 };
    G = {
      top,
      L,
      r1,
      r2,
      caps: [
        cap0,
        { x: capX1, y: r2.y + H2 / 2 - caps[1].offsetHeight / 2 },
        { x: fl, y: CAP_B + (vh - CAP_B) / 2 - caps[2].offsetHeight / 2 },
      ],
    };
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
  /** Where the page is (the scroll handler keeps it), and how far past the pin's end the tray's
   *  flyout stays: until the next section would come up under it (set as it opens). */
  let lastY = scrollY;
  let flyRoom = innerHeight * 0.3;
  let lastGone = false;
  /** The visitor has scrolled since the last picture (the scroll handler says so). */
  let nudged = false;
  function render(p: number, force = false) {
    if (!G) return;
    // Past its end the stage scrolls away with the tray's words still in sight: the flyout
    // stays with them, and goes as the next section comes up to it.
    // (Where the page is now, read here: a load straight to a section below, #kept, comes with
    // no scroll event to tell the handler, and from the top the tray's beat would open far past it.)
    const gone = scrollY - (G.top + LEN) > flyRoom;
    if (p === lastP && gone === lastGone && !force && !nudged) return;
    lastP = p;
    lastGone = gone;
    // The story moves its words (a selection in them is drawn again, src/scripts/selection.ts).
    dispatchEvent(new Event("placechange"));
    const { a, b, c } = phases(p);
    // The beat in the tray: the capsule's flyout shows the song, still playing. It comes out as
    // the window goes in (the last of its way there), so the hold is never its words alone:
    // the page rests anywhere from the move's end on, and a flyout that waited past that left
    // a stretch with nothing in it. Closed by the visitor there (a press beside it, Escape),
    // it is back when they scroll on (their own scroll: not the picture easing up to where
    // the page already is, which would undo an Escape pressed just after a scroll).
    const inBeat = c >= 0.96 && !gone;
    const again = inBeat && (nudged || force) && !songFly?.matches(":popover-open");
    nudged = false;
    if (songFly && (inBeat !== beat || again)) {
      beat = inBeat;
      if (inBeat && !songFly.matches(":popover-open")) {
        // Marked first: the bar places the story's flyout under the capsule's song.
        songFly.dataset.story = "";
        songFly.dataset.into = "";
        songFly.showPopover();
        // "still playing" beside it, pointing back at it, where its words have room.
        if (stillNote)
          requestAnimationFrame(() => {
            if (!beat || !songFly.matches(":popover-open")) return;
            // Where it rests (its own box in the viewport), not where its way out of the
            // capsule has got to.
            const f = { right: songFly.offsetLeft + songFly.offsetWidth, top: songFly.offsetTop, bottom: songFly.offsetTop + songFly.offsetHeight };
            flyRoom = Math.max(0, innerHeight - f.bottom + 64);
            showNoteAt(stillNote, f.right + 12, f.top + 36);
          });
      } else if (!inBeat) endBeat();
    }
    // The first screen's words leave first (keys that were in them go on from the steps).
    if (p >= 0.6) fitTrayWords();
    const out = seg(p, 0, 0.07);
    for (const el of hero) {
      el.style.opacity = out ? String(1 - out * out * (3 - 2 * out)) : "";
      if (!hand(el)) el.style.translate = out ? `0 ${(-out * 12).toFixed(1)}px` : "";
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
      if (win.minimised) return morph.end();
    } else userMin = null;

    if (p <= 0) {
      morph.end();
      win.drive(null);
      return;
    }
    // Leaving the first screen, the visitor's placement (dragged, maximised) is carried along;
    // once the window is whole in hold 1 it is forgotten, and the first screen gets it home.
    if (a >= 1) win.forget();
    const from = win.rested;
    const L = G.L;
    const r1 = { ...G.r1, w: L.w, h: L.h };
    let f: Frame;
    if (b <= 0 && a > 0 && a < 1 && (from.w !== L.w || from.h !== L.h)) {
      // The first screen's narrow window becoming the whole wide app: resized, as to narrow.
      f = morph.frame("first", { w: from.w, h: from.h, x: from.x, y: from.y, s: from.s }, r1, a);
    } else if (b <= 0) {
      morph.end();
      f = { w: L.w, h: L.h, x: lerp(from.x, G.r1.x, a), y: lerp(from.y, G.r1.y, a), s: lerp(from.s, G.r1.s, a), o: 1 };
    } else if (b < 1) {
      // To narrow: the window resizes from the whole rect to the narrow one.
      f = morph.frame("narrow", r1, { ...G.r2, w: NARROW.w, h: NARROW.h }, b);
    } else {
      morph.end();
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
  }

  // ---- Scroll: a target from scrollY, smoothed; the loop sleeps when it arrives ----
  let target = 0;
  let cur = 0;
  let raf = 0;
  let last = 0;
  let dir = 1;
  /** The page's own glide on to a hold, and where it last put the page. */
  let glide: { from: number; edge: number; to: number; t0: number; ms: number; tail: number } | null = null;
  let glideY = 0;
  // In its beat the flyout is all there is to see, and a press beside it closes it as it would
  // any flyout (the browser's light dismiss), as does another flyout opening. It comes back
  // once nothing else is open, unless the visitor closed it on purpose: Escape, or its own
  // button in the capsule (then it is back when the page moves, as render has it).
  let meant = -1e9;
  const mean = () => (meant = performance.now());
  addEventListener("keydown", (e) => e.key === "Escape" && mean(), true);
  for (const type of ["pointerdown", "click"]) np?.addEventListener(type, mean);
  for (const fly of document.querySelectorAll<HTMLElement>("[data-fly]"))
    fly.addEventListener("toggle", (e) => {
      if ((e as ToggleEvent).newState !== "closed" || !beat) return;
      if (fly === songFly && performance.now() - meant < 600) return;
      // After its way back into the capsule (Bar.astro), so it is seen to come out again.
      // (Not if Escape came meanwhile: the visitor wants it gone.)
      window.setTimeout(() => {
        if (beat && G && performance.now() - meant > 600 && !document.querySelector("[data-fly]:popover-open")) render(lastP, true);
      }, 380);
    });

  const frame = (now: number) => {
    raf = 0;
    const dt = Math.min(64, now - (last || now));
    last = now;
    if (glide && G) {
      // To where the picture finishes, on the glide's curve; then the still stretch to the hold.
      const t = now - (glide.t0 ||= now);
      const u = glide.tail ? Math.min(1, (t - glide.ms) / glide.tail) : 1;
      if (t < glide.ms) glideY = glide.from + (glide.edge - glide.from) * easeGlide(t / glide.ms);
      else glideY = u >= 1 ? glide.to : glide.edge + (glide.to - glide.edge) * u * (2 - u);
      if (t >= glide.ms && u >= 1) glide = null;
      scrollTo(0, glideY);
      target = storyProgress(glideY, G.top, LEN);
    }
    // A jump of more than a third of the story (a section link) lands at once.
    if (Math.abs(target - cur) > 0.35) cur = target;
    cur += (target - cur) * (1 - Math.exp(-dt / 90));
    if (Math.abs(target - cur) < 0.0002) cur = target;
    render(cur);
    if (cur !== target || glide) raf = requestAnimationFrame(frame);
    else last = 0;
  };
  const wake = () => {
    if (!raf) raf = requestAnimationFrame(frame);
  };
  let settleTimer = 0;
  const settle = () => {
    if (!G || glide) return;
    const to = settleTarget(target, dir);
    if (to == null) return;
    const from = scrollY;
    const end = Math.round(G.top + to * LEN);
    if (Math.abs(end - from) < 1) return;
    const path = glidePath(target, to);
    glide = { from, edge: G.top + path.edge * LEN, to: end, t0: 0, ms: path.ms, tail: path.tail };
    wake();
  };
  addEventListener(
    "scroll",
    () => {
      if (!G) return;
      const y = scrollY;
      // The glide's own steps aren't the visitor's: they neither turn it nor ask for another.
      // (A scroll far from where the glide put the page is the visitor's: the scrollbar.)
      if (glide && Math.abs(y - glideY) < 48) {
        lastY = y;
        return;
      }
      glide = null;
      nudged = true;
      if (y !== lastY) dir = y > lastY ? 1 : -1;
      lastY = y;
      target = storyProgress(y, G.top, LEN);
      clearTimeout(settleTimer);
      settleTimer = window.setTimeout(settle, 90);
      wake();
    },
    { passive: true },
  );
  addEventListener("scrollend", () => {
    if (glide) return;
    clearTimeout(settleTimer);
    settleTimer = window.setTimeout(settle, 40);
  });
  // The visitor's hand on the page ends the glide at once: the scroll that follows is theirs,
  // and if none follows (a key, a click) the page settles again from where it was left.
  const letGo = () => {
    if (!glide) return;
    glide = null;
    clearTimeout(settleTimer);
    settleTimer = window.setTimeout(settle, 250);
  };
  for (const type of ["wheel", "touchstart", "pointerdown", "keydown"]) addEventListener(type, letGo, { passive: true });

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
      glide = null;
      beat = false;
      endBeat();
      morph.end();
      win.drive(null);
      // Only what the story set: the shelf's own style holds its place (first.ts), and
      // clearing all of it left the sleeves at the screen's corner wherever the story is off
      // (reduced motion, a short window).
      for (const el of [...hero, ...caps]) {
        el.style.opacity = "";
        if (!hand(el)) for (const p of ["translate", "transform", "visibility"]) el.style.removeProperty(p);
      }
      for (const el of hero) el.inert = false;
    }
  };

  win.onLayout(() => requestAnimationFrame(measure));
  if (siteBar)
    new MutationObserver(() => {
      if (G && !G.trayFit && lastP >= 0.6) render(lastP, true);
    }).observe(siteBar, { attributes: true, attributeFilter: ["data-state"] });
  // On the first frame: a width query needs the page's layout to answer, and listening asks it.
  requestAnimationFrame(() => {
    mq.addEventListener("change", apply);
    apply();
  });
}
