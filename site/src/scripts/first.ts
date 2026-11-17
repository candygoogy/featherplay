// The first screen (Stage.astro), composed by measure: the words read where the headline set
// itself, src/scripts/first-math.ts decides the arrangement and the sizes, and this places the
// words, the shelf, the window's room and the two notes. Measured on load, when the fonts arrive
// and on resize, never per frame. Phones and small rooms stack by CSS alone (data-laid="stack").
//
// Then the arrival, once, at load (not on a reload part-way down, not under reduced motion):
// the light comes up, the headline sharpens line by line, the sentence, the buttons and the
// line under them follow, the window opens from the tray, the sleeves land 50 ms apart and the
// notes write themselves in; about 1.3 s. Until it is laid out the first screen isn't shown
// (it would only jump), so it never shows half-placed.

import { compose, type Plan } from "./first-math";
import { whenWindow } from "../demo/window";
import { firstPlaced } from "../demo/lines";
import { showNoteAt, hideNote, penArrow } from "./notes";

const reduced = () => matchMedia("(prefers-reduced-motion: reduce)").matches;
const EASE_OUT = "cubic-bezier(0.22, 1, 0.36, 1)";
const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

export function startFirst(): void {
  const stage = document.querySelector<HTMLElement>("[data-stage]");
  const first = stage?.querySelector<HTMLElement>("[data-first]");
  if (!stage || !first) return;
  const root = document.documentElement;
  const head = first.querySelector<HTMLElement>("[data-head]")!;
  const lines = [...head.querySelectorAll<HTMLElement>(".line")];
  const col = first.querySelector<HTMLElement>("[data-col]")!;
  const actions = col.querySelector<HTMLElement>(".actions")!;
  const shelf = first.querySelector<HTMLElement>("[data-shelf]");
  const slot = first.querySelector<HTMLElement>("[data-win-slot]")!;
  const noteWin = first.querySelector<HTMLElement>("[data-note='screenshot']");
  const noteShelf = first.querySelector<HTMLElement>("[data-note='sleeves']");
  let plan: Plan | null = null;
  let arrived = false;
  /** The notes are written once everything has landed; till then a relayout doesn't show them. */
  let landed = false;

  const px = (n: number) => `${Math.round(n * 100) / 100}px`;
  /** The phone's window height, for the width it was worked out at. */
  let phoneH: { vw: number; h: string } | null = null;
  function measure() {
    const vw = root.clientWidth;
    const vh = innerHeight;
    // The column as wide as its buttons need (within 340 to 470), then its height at that width.
    const aw = [...actions.children].reduce((a, k) => a + (k as HTMLElement).offsetWidth, 0) + 12 * (actions.children.length - 1);
    const colW = clamp(aw + 2, 340, 470);
    first.style.setProperty("--col-w", px(colW));
    const fs = parseFloat(getComputedStyle(head).fontSize);
    const lh = parseFloat(getComputedStyle(head).lineHeight) || fs * 0.94;
    const colGap = Math.round(clamp(fs * 0.34, 18, 32));
    plan = compose(vw, vh, { l1w: lines[0].offsetWidth, l2w: lines[1].offsetWidth, lh, colW, colH: col.offsetHeight, colGap });
    if (!plan) {
      first.dataset.laid = "stack";
      for (const k of ["--hx", "--hy", "--cy", "--first-h", "--col-w"]) first.style.removeProperty(k);
      // The shelf's place and size in the composition go with it: stacked, its CSS sizes it.
      for (const k of ["--sx", "--sy", "--sz", "--sg", "--cols"]) shelf?.style.removeProperty(k);
      delete slot.dataset.fitW;
      slot.dataset.fit = "first";
      slot.removeAttribute("style");
      // A phone: the window as wide as the column, in the narrow app's own proportions (its
      // 480 x 680 layout, lyrics at some 14 px), or the rest of the first screen where that is
      // more; never taller than a screen under the bar. (Fitted into what was left of the
      // first screen it shrank to a third of the app's size, its lyrics some 7 px: a short
      // scroll to its bar reads better than that.) A browser's toolbar showing or hiding as the
      // page moves changes the viewport's height (iOS Safari's layout viewport too: the window
      // grew and shrank with it), so the height is the one the page loaded with.
      // Worked out once for each width (a phone turned works it out again): a change of height
      // alone is a toolbar, and keeps it.
      if (vw < 761) {
        if (phoneH?.vw !== vw) {
          const top = slot.getBoundingClientRect().top + scrollY;
          const readable = Math.min((slot.getBoundingClientRect().width * 680) / 480, root.clientHeight - 88);
          phoneH = { vw, h: px(clamp(Math.max(root.clientHeight - top - 12, readable), 250, 752)) };
        }
        slot.style.height = phoneH.h;
      }
      first.dataset.fl = String(first.getBoundingClientRect().left);
      // Stacked in a room too short for it (the first screen held to one screen by the story),
      // the shelf would be pushed onto the words: it steps aside instead (its songs are in the
      // capsule's song flyout and the app's own queue too).
      if (shelf) {
        shelf.removeAttribute("data-off");
        const s = shelf.getBoundingClientRect();
        const c = col.getBoundingClientRect();
        const f = first.getBoundingClientRect();
        shelf.toggleAttribute("data-off", s.top < c.bottom + 8 || s.bottom > f.bottom + 0.5);
      }
      return;
    }
    const p = plan;
    first.dataset.laid = p.kind;
    first.dataset.fl = String(p.fl);
    first.style.setProperty("--hx", px(p.text.x));
    first.style.setProperty("--hy", px(p.text.y));
    first.style.setProperty("--cy", px(p.text.y + lh * 2 + colGap));
    first.style.setProperty("--first-h", px(Math.max(vh, p.win.y + p.win.h + 24, p.shelf.off ? 0 : p.shelf.y + p.shelf.h + 24)));
    if (shelf) {
      shelf.toggleAttribute("data-off", p.shelf.off);
      shelf.style.setProperty("--sx", px(p.shelf.x));
      shelf.style.setProperty("--sy", px(p.shelf.y));
      shelf.style.setProperty("--sz", `${Math.floor(p.shelf.size)}px`);
      shelf.style.setProperty("--sg", `${p.shelf.gap}px`);
      shelf.style.setProperty("--cols", String(p.shelf.cols));
    }
    // The window's room is exactly the window: window.ts draws the app at the size decided here.
    Object.assign(slot.style, { left: px(p.win.x), top: px(p.win.y), width: px(p.win.w), height: px(p.win.h) });
    slot.dataset.fitW = String(p.fit.w);
    slot.dataset.fitH = String(p.fit.h);
    delete slot.dataset.fit;
  }

  /** The hand's size for the first screen's two notes, from the window's laid-out width: about
   *  36 px for the smallest desktop composition, 40 px at 1440 x 900, 50 px at the widest
   *  frame (beside a 92 px headline: an aside, not a second headline). */
  const PEN = [
    [526, 36],
    [804, 40],
    [1155, 42],
    [1295, 50],
  ] as const;
  const penFor = (w: number) => {
    if (w <= PEN[0][0]) return PEN[0][1];
    for (let i = 1; i < PEN.length; i++) {
      const [w1, s1] = PEN[i];
      const [w0, s0] = PEN[i - 1];
      if (w <= w1) return s0 + ((w - w0) / (w1 - w0)) * (s1 - s0);
    }
    return PEN[PEN.length - 1][1];
  };

  /** The notes, in free ground, each with its arrow drawn to the window. "not a screenshot"
   *  over the window's top, right of the headline, its first word underlined. "drop one on
   *  the player" where there is room: in the ground between the credit line and the sleeves,
   *  its arrow a shallow arc across to the window's left edge; else under the window's
   *  lower left beside the sleeves, its arrow rising into it; else on two lines in the gap
   *  between the sleeves and the window. Hidden where none is free (showNoteAt: never on the
   *  words, the sleeves, the window or the bar, never off the screen). */
  function notes() {
    const p = plan;
    if (!p) {
      if (noteWin) hideNote(noteWin);
      if (noteShelf) hideNote(noteShelf);
      return;
    }
    const top = stage!.getBoundingClientRect().top;
    const l1 = lines[0].getBoundingClientRect();
    const colBox = col.getBoundingClientRect();
    const words = [l1, lines[1].getBoundingClientRect(), colBox];
    const win = { left: p.win.x, top: p.win.y + top, right: p.win.x + p.win.w, bottom: p.win.y + p.win.h + top };
    const sh = { left: p.shelf.x, top: p.shelf.y + top, right: p.shelf.x + p.shelf.w, bottom: p.shelf.y + p.shelf.h + top };
    const fs = Math.round(penFor(p.win.w));
    const size = (n: HTMLElement, wrap = false) => {
      n.style.fontSize = `${fs}px`;
      n.classList.toggle("wrap", wrap);
      const r = n.querySelector(":scope > span")!.getBoundingClientRect();
      return { w: r.width, h: r.height };
    };
    // (Each arrow stops just short of the window: the window is drawn over the notes.)
    const wordsOf = (n: HTMLElement) => n.querySelector(":scope > span")!.getBoundingClientRect();

    if (noteWin) {
      const { w, h } = size(noteWin);
      // Clear of the headline, a little off the title bar, never flush with the window's edge.
      const y = win.top - h - fs * 0.95;
      const ww = win.right - win.left;
      const right = win.right - fs * 1.2 - w;
      // Right of the headline over the window's middle; else further right; else centred over
      // the window (a tall room, the headline above).
      const xs = [Math.max(l1.right + fs, win.left + ww * 0.48), win.left + ww * 0.6, win.left + (ww - w) / 2];
      let shown = false;
      for (const x of xs) {
        if (x > right) continue;
        if (showNoteAt(noteWin, x, y, [...words, win, sh])) {
          shown = true;
          break;
        }
      }
      if (!shown) hideNote(noteWin);
      else {
        const r = wordsOf(noteWin);
        // Just over the window (a tall room): a short stroke down; else a longer curve.
        const near = win.top - r.bottom < fs * 1.5;
        const tx = Math.min(r.right + fs * (near ? 0.7 : 1.5), win.right - fs * 0.8);
        penArrow(noteWin, { x: r.right + fs * 0.25, y: r.top + r.height * 0.55 }, { x: tx, y: win.top - fs * 0.12 }, -fs * 0.55);
      }
    }

    if (noteShelf && p.shelf.off) hideNote(noteShelf);
    else if (noteShelf) {
      // (Clear of the other note too.)
      const other = noteWin?.hasAttribute("data-on") ? wordsOf(noteWin) : null;
      const avoid = [...words, sh, win, ...(other ? [other] : [])];
      let placed = false;
      // 1. Above the sleeves, in the ground under the credit line (optically nearer the sleeves).
      {
        const { w, h } = size(noteShelf);
        const gap = sh.top - colBox.bottom;
        if (gap >= h + fs * 0.9) {
          const x = sh.left + Math.max(0, (sh.right - sh.left - w) / 2);
          const y = colBox.bottom + (gap - h) * 0.6;
          if (x + w + fs * 1.5 < win.left && showNoteAt(noteShelf, x, y, avoid)) {
            const r = wordsOf(noteShelf);
            // A shallow arc across to the window, its head pointing at the window (not down).
            const from = { x: r.right + fs * 0.3, y: r.top + r.height * 0.55 };
            const to = { x: win.left - fs * 0.15, y: clamp(from.y + fs * 0.3, win.top + fs, win.bottom - fs) };
            penArrow(noteShelf, from, to, -fs * 0.28);
            placed = true;
          }
        }
      }
      // 2. Under the window's lower left, beside the sleeves' lower row; the arrow rises into it.
      if (!placed) {
        const { w, h } = size(noteShelf);
        const x = sh.right + fs * 0.7;
        const row = (sh.bottom - sh.top) / 2;
        const y = Math.max(win.bottom + fs * 1.4, sh.bottom - row / 2 - h / 2);
        if (x + w < document.documentElement.clientWidth - 8 && y + h < innerHeight - 8 && showNoteAt(noteShelf, x, y, avoid)) {
          const r = wordsOf(noteShelf);
          const from = { x: r.left + r.width * 0.32, y: r.top - fs * 0.12 };
          penArrow(noteShelf, from, { x: from.x + fs * 0.7, y: win.bottom + fs * 0.15 }, fs * 0.35);
          placed = true;
        }
      }
      // 3. On two lines in the gap between the sleeves' lower row and the window.
      if (!placed) {
        const { w, h } = size(noteShelf, true);
        const x = win.left - w - fs * 1.4;
        const row = (sh.bottom - sh.top) / 2;
        const y = sh.bottom - row / 2 - h / 2;
        if (x > sh.right + fs * 0.4 && showNoteAt(noteShelf, x, y, avoid)) {
          const r = wordsOf(noteShelf);
          penArrow(noteShelf, { x: r.right + fs * 0.2, y: r.top + r.height * 0.4 }, { x: win.left - fs * 0.15, y: r.top - fs * 0.2 }, -fs * 0.6);
          placed = true;
        }
      }
      // 4. A tall room (the window under the words and the sleeves): on two lines left of the
      //    sleeves, the arrow down into the window.
      if (!placed) {
        const { w, h } = size(noteShelf, true);
        const x = sh.left - w - fs;
        // Above "not a screenshot" where that sits in the same gap.
        const y = Math.min(sh.bottom - h, other ? other.top - fs * 0.7 - h : Infinity);
        if (win.top > sh.bottom && x > colBox.right + fs * 0.4 && showNoteAt(noteShelf, x, y, avoid)) {
          const r = wordsOf(noteShelf);
          const from = { x: r.right + fs * 0.1, y: r.bottom - fs * 0.2 };
          // Stacked over "not a screenshot" in the same gap, the two share this one arrow (two
          // heads landing side by side read as a slip of the pen).
          const shared = !!other && other.right > r.left && other.left < from.x + fs * 2;
          if (shared) noteWin!.querySelector("svg.free path")?.setAttribute("d", "M0 0");
          const tx = Math.max(from.x + fs * 0.6, (other?.right ?? 0) + fs * (shared ? 1 : 2));
          penArrow(noteShelf, from, { x: tx, y: win.top - fs * 0.15 }, -fs * 0.35);
          placed = true;
        }
      }
      if (!placed) {
        noteShelf.classList.remove("wrap");
        hideNote(noteShelf);
      }
    }
  }

  // ---- The arrival ----
  function arrive(win: Awaited<typeof whenWindow>) {
    if (arrived) return;
    arrived = true;
    const again = root.dataset.land || scrollY > 4;
    first.toggleAttribute("data-shown", true);
    if (reduced() || again) {
      landed = true;
      notes();
      return;
    }
    // The visitor moving the page ends the arrival at once: nothing keeps landing out of view.
    const all: (Animation | undefined)[] = [];
    addEventListener("scroll", () => all.forEach((a) => a?.finish()), { once: true, passive: true });
    const rise = (el: Element | null, delay: number, extra: Keyframe = {}) =>
      el?.animate([{ opacity: 0, translate: "0 8px", ...extra }, { opacity: 1, translate: "0 0", filter: "none" }], { duration: 480, delay, easing: EASE_OUT, fill: "backwards" });
    // The light: the desk's lit layers come up under the words.
    // (To the opacity they have: one keyframe, so no style is read.)
    for (const l of document.querySelectorAll(".desk .slot.front :is(.k-lit, .k-hi)"))
      all.push(l.animate([{ opacity: 0, offset: 0 }], { duration: 900, easing: "ease-out", fill: "backwards" }));
    // The headline, line by line, sharpening.
    lines.forEach((l, i) => all.push(l.animate([{ opacity: 0, filter: "blur(8px)", translate: "0 0.12em" }, { opacity: 1, filter: "blur(0)", translate: "0 0" }], { duration: 480, delay: i * 110, easing: EASE_OUT, fill: "backwards" })));
    // The sentence, the buttons, the line under them.
    [...col.children].forEach((c, i) => all.push(rise(c, [200, 280, 340, 380][i] ?? 380)));
    // The window opens from the tray.
    win.arrive(450);
    // The sleeves land 50 ms apart.
    shelf?.querySelectorAll(".sleeve").forEach((s, i) =>
      all.push(s.animate([{ opacity: 0, translate: "0 6px", scale: "1.04" }, { opacity: 1, translate: "0 0", scale: "1" }], { duration: 480, delay: 650 + i * 50, easing: EASE_OUT, fill: "backwards" })),
    );
    // The notes write themselves in once everything has landed.
    setTimeout(() => {
      landed = true;
      notes();
    }, 1000);
  }

  // ---- On load, fonts and resize ----
  let frame = 0;
  const relayout = () => {
    if (frame) return;
    frame = requestAnimationFrame(async () => {
      frame = 0;
      measure();
      const win = await whenWindow;
      win.layout();
      if (landed) notes();
    });
  };
  // At load, not while the scripts start (a measure would force the page's first layout inside
  // their task): the composition on the first frame, the window placed in it on the next, and
  // only then the lines and the lyrics (src/demo/lines.ts), each its own task, not one long
  // one. The first screen is hidden until it is placed.
  frame = requestAnimationFrame(() => {
    frame = 0;
    measure();
    requestAnimationFrame(async () => {
      const win = await whenWindow;
      win.layout();
      arrive(win);
      firstPlaced();
    });
  });
  document.fonts?.ready.then(relayout);
  addEventListener("resize", relayout);
}
