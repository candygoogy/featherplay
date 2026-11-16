// featherplay's window on the page (Window.astro): the replica laid out for the window's own size
// and drawn scaled to fit (src/demo/fit.ts), always wholly inside the viewport. It can be dragged
// by its title bar, maximised and restored (the caption button or a double-click on the title
// bar), minimised to the tray (and back from the bar's tray icon), and closed (the app's own
// question first). The wheel over it scrolls the page until the visitor clicks into it.
//
// Every move is a transform of the one element (translate and scale, from its top left): the
// app is laid out again only when the window's size really changes (a resize of the page, the
// story's narrow hold). Coordinates are the pin's (Stage.astro): the viewport's while the story
// holds the stage, the stage's own otherwise.

import { clampInto, fitWindow, placeIn, type Box, type Fit } from "./fit";
import { fitFirst } from "../scripts/first-math";
import type { Player } from "./player";
import { hideNote, showNoteAt } from "../scripts/notes";

export type Frame = { w: number; h: number; x: number; y: number; s: number; o: number };

export type WindowController = {
  readonly el: HTMLElement;
  /** Where the window rests on the first screen, and its layout size. */
  readonly home: Frame;
  /** Where it is drawn now (the pin's coordinates). */
  readonly frame: Frame;
  /** Where it rests by the visitor's own placement (home, dragged or maximised). */
  readonly rested: Frame;
  readonly maximised: boolean;
  readonly minimised: boolean;
  /** Measure again (a resize, the fonts arriving). */
  layout(): void;
  /** The story draws the window itself while it moves; null hands it back. */
  drive(frame: Frame | null): void;
  /** The visitor's own placement (a drag, maximised), which the story wears away. */
  forget(): void;
  /** The tray icon's centre (or the capsule's song), in the pin's coordinates. */
  trayPoint(): { x: number; y: number };
  onLayout(fn: () => void): void;
  /** The first screen's arrival: the window opens from the tray, after `delay` ms. */
  arrive(delay: number): void;
};

const MIN_MS = 300;
const RESTORE_MS = 260;
const MAX_MS = 280;
const EASE_WINDOW = "cubic-bezier(0.32, 0.72, 0, 1)";
/** A pointer-sized screen: the window can be dragged and maximised there. */
const DESK = "(min-width: 761px)";
/** The docked bar's height, which a maximised or dragged window stays under. */
const BAR = 52;

const reduced = () => matchMedia("(prefers-reduced-motion: reduce)").matches;
const tf = (f: Frame, off: { x: number; y: number }) =>
  `translate(${(f.x - off.x).toFixed(2)}px, ${(f.y - off.y).toFixed(2)}px) scale(${f.s.toFixed(5)})`;

let ready: (w: WindowController) => void;
/** The page's window, once it is bound (the story waits for it). */
export const whenWindow = new Promise<WindowController>((r) => (ready = r));

export function bindWindow(el: HTMLElement, player: Player): WindowController {
  const slot = el.parentElement as HTMLElement;
  const pin = el.closest<HTMLElement>("[data-pin]") ?? document.body;
  const replica = el.querySelector<HTMLElement>("[data-replica]")!;
  const titlebar = replica.querySelector<HTMLElement>(".titlebar")!;
  const dialog = slot.querySelector<HTMLDialogElement>("[data-closer]");
  const desk = matchMedia(DESK);
  const stage = el.closest<HTMLElement>("[data-stage]");

  let slotOff = { x: 0, y: 0 };
  let home: Frame = { w: 1440, h: 852, x: 0, y: 0, s: 1, o: 1 };
  let drag = { x: 0, y: 0 };
  let maxed = false;
  let minimised = false;
  let driven: Frame | null = null;
  let shown: Frame = home;
  let size = { w: 0, h: 0 };
  let scaleTimer = 0;
  const layoutFns: (() => void)[] = [];

  // ---- Geometry ----
  const pinRect = () => pin.getBoundingClientRect();
  /** Where the pin's top is at rest, from the top of the page: the stage's, which never sticks
   *  (the pin itself does while the story holds it). */
  const pinHome = () => {
    const stage = pin.parentElement ?? pin;
    return stage.getBoundingClientRect().top + scrollY;
  };
  /** The first screen's viewport below the docked bar, in the pin's coordinates: wherever the
   *  page is scrolled, a dragged or maximised window stays on the first screen (it never comes
   *  down over the sections after it). */
  const viewportBox = (): Box => {
    const vw = document.documentElement.clientWidth;
    return { x: -pinRect().left, y: BAR - pinHome(), w: vw, h: innerHeight - BAR };
  };
  const rested = (): Frame => {
    if (maxed) {
      const v = viewportBox();
      const s = Math.min(v.w / home.w, v.h / home.h);
      return { ...home, s, ...placeIn(v, { w: home.w, h: home.h, s }, "center", "center") };
    }
    return { ...home, x: home.x + drag.x, y: home.y + drag.y };
  };
  /** The bar's part the window goes into: the tray icon, or the capsule's song (whichever this
   *  state shows). */
  const trayTarget = () =>
    [...document.querySelectorAll<HTMLElement>("[data-tray-target]")].find((t) => t.getClientRects().length && getComputedStyle(t).visibility !== "hidden");
  const trayPoint = () => {
    const p = pinRect();
    const icon = trayTarget();
    if (icon) {
      const r = icon.getBoundingClientRect();
      return { x: r.left + r.width / 2 - p.left, y: r.top + r.height / 2 - p.top };
    }
    return { x: document.documentElement.clientWidth - 120 - p.left, y: 26 - p.top };
  };
  const trayFrame = (from: Frame): Frame => {
    const t = trayPoint();
    const s = from.s * 0.2;
    return { ...from, s, o: 0, x: t.x - (from.w * s) / 2, y: t.y - (from.h * s) / 2 };
  };

  // ---- Drawing: one transform, the size only when it changes ----
  const draw = (f: Frame) => {
    if (f.w !== size.w || f.h !== size.h) {
      el.style.width = `${f.w}px`;
      el.style.height = `${f.h}px`;
      size = { w: f.w, h: f.h };
    }
    el.style.transform = tf(f, slotOff);
    el.style.opacity = f.o >= 1 ? "" : f.o.toFixed(3);
    // Away from home (dragged, maximised, in the tray, the story's): the first screen's notes
    // about it step aside.
    stage?.toggleAttribute("data-win-away", maxed || minimised || !!driven || !!(drag.x || drag.y));
    // The scale for the page-size caption targets and corners: written once the window rests
    // (it changes their layout), not on every frame of a move.
    clearTimeout(scaleTimer);
    const s = f.s.toFixed(4);
    if (el.style.getPropertyValue("--s") !== s) scaleTimer = window.setTimeout(() => el.style.setProperty("--s", s), 120);
    shown = f;
  };
  /** Where the window is on screen now: in the middle of a glide, where the glide has got to. */
  const current = (): Frame => {
    if (!el.getAnimations().length) return shown;
    const cs = getComputedStyle(el);
    const m = new DOMMatrixReadOnly(cs.transform === "none" ? undefined : cs.transform);
    return { ...shown, x: m.e + slotOff.x, y: m.f + slotOff.y, s: m.a, o: Number(cs.opacity) };
  };
  /** Glides from where the window is drawn now to `to`, then rests there. */
  const glide = (to: Frame, ms: number) => {
    const from = current();
    for (const a of el.getAnimations()) a.cancel();
    draw(to);
    if (reduced() || ms <= 0) return;
    el.animate(
      [
        { transform: tf(from, slotOff), opacity: from.o },
        { transform: tf(to, slotOff), opacity: to.o },
      ],
      { duration: ms, easing: EASE_WINDOW },
    );
  };
  const settle = (ms = 0) => {
    if (driven) return;
    const f = minimised ? trayFrame(rested()) : rested();
    // Laid out again with nothing changed (the fonts arriving, say): whatever is under way (the
    // arrival's opening) goes on.
    const same = (["x", "y", "s", "w", "h", "o"] as const).every((k) => Math.abs(f[k] - shown[k]) < 0.01);
    if (!ms && same && el.style.transform) return;
    if (ms) glide(f, ms);
    else {
      for (const a of el.getAnimations()) a.cancel();
      draw(f);
    }
  };

  function layout() {
    el.classList.add("placed");
    const p = pinRect();
    const s = slot.getBoundingClientRect();
    slotOff = { x: s.left - p.left, y: s.top - p.top };
    // The slot, but never more than the viewport: on the first screen the window ends inside
    // it (from the pin's top); in the page's flow it fits one screen under the bar.
    // The room is the first screen's, wherever the page is scrolled (the slot's top as it is at
    // rest, not as the pinned stage shows it now). On a phone the slot itself is sized in small
    // viewport units (Stage.astro), so a toolbar showing and hiding never resizes the window.
    const vw = document.documentElement.clientWidth;
    const room = desk.matches ? innerHeight - (pinHome() + slotOff.y) - 8 : s.height;
    const box: Box = { x: slotOff.x, y: slotOff.y, w: Math.min(s.width, vw), h: Math.max(1, Math.min(s.height, room)) };
    // The first screen's composition decides the app's size (src/scripts/first.ts: the room is
    // the window); stacked, the first screen's own rule; elsewhere the general one.
    const fit: Fit = slot.dataset.fitW
      ? fitWindow(box, { want: { w: Number(slot.dataset.fitW), h: Number(slot.dataset.fitH) } })
      : slot.dataset.fit === "first"
        ? fitFirst(box)
        : fitWindow(box);
    const at = placeIn(box, fit, desk.matches ? "end" : "center", "start");
    home = { w: fit.w, h: fit.h, s: fit.s, o: 1, ...at };
    // A dragged window stays where it was put, as far as the new viewport allows.
    if (drag.x || drag.y) {
      const v = viewportBox();
      const at2 = clampInto({ x: home.x + drag.x, y: home.y + drag.y, w: fit.w * fit.s, h: fit.h * fit.s }, v);
      drag = { x: at2.x - home.x, y: at2.y - home.y };
    }
    if (!desk.matches) {
      drag = { x: 0, y: 0 };
      setMax(false, 0);
    }
    settle();
    if (dialog?.open) placeDialog();
    for (const fn of layoutFns) fn();
  }

  // ---- Maximise and restore ----
  const maxButton = replica.querySelector<HTMLButtonElement>('[data-act="maximise"]');
  /** The state, and the caption button's name saying what it does next (as Windows' does). */
  const markMax = (on: boolean) => {
    maxed = on;
    el.toggleAttribute("data-max", on);
    maxButton?.setAttribute("aria-label", on ? "Restore featherplay" : "Maximise featherplay");
  };
  function setMax(on: boolean, ms = MAX_MS) {
    if ((on && !desk.matches) || driven) return;
    markMax(on);
    if (on) drag = { x: 0, y: 0 };
    settle(ms);
  }
  // Too small a screen to maximise on: the button says it can't.
  const deskOnly = () => maxButton?.setAttribute("aria-disabled", String(!desk.matches));

  // ---- Minimise to the tray, and back ----
  // "opens in 0.24 s": by the tray icon, the first time the window comes back from the tray
  // (not when the story brings it back), for a few seconds or until the page moves.
  const opensNote = document.querySelector<HTMLElement>("[data-note='opens']");
  let opensShown = false;
  const opensOff = () => {
    if (opensNote) hideNote(opensNote);
    removeEventListener("scroll", opensOff);
  };
  const showOpens = () => {
    if (!opensNote || opensShown || driven) return;
    const t = trayTarget()?.getBoundingClientRect();
    if (!t) return;
    const p = pinRect();
    const f = rested();
    const win = { left: p.left + f.x, top: p.top + f.y, right: p.left + f.x + f.w * f.s, bottom: p.top + f.y + f.h * f.s };
    if (!showNoteAt(opensNote, t.left + t.width / 2 - 6, t.bottom + 2, [win])) return;
    opensShown = true;
    setTimeout(opensOff, 4500);
    addEventListener("scroll", opensOff, { passive: true });
  };

  const setMinimised = (on: boolean) => {
    if (on === minimised) return;
    minimised = on;
    if (!on) showOpens();
    const hadFocus = el.contains(document.activeElement);
    el.toggleAttribute("data-minimised", on);
    document.documentElement.toggleAttribute("data-window-minimised", on);
    el.inert = on;
    if (!driven) glide(on ? trayFrame(rested()) : rested(), on ? MIN_MS : RESTORE_MS);
    // The keys go where the window went: the tray icon, or the capsule's song.
    if (on && hadFocus) trayTarget()?.focus({ preventScroll: true });
  };
  player.subscribe((s) => setMinimised(s.minimised));

  // ---- Close: the app asks, then what was chosen; remembered for the visit if asked to ----
  let remembered: "tray" | "close" | null = null;
  const closeAs = (choice: "tray" | "close") => {
    if (choice === "close" && player.state.playing) player.pause();
    player.minimise();
  };
  const remember = dialog?.querySelector<HTMLInputElement>("[data-closer-remember]");
  const closerText = dialog?.querySelector("[data-closer-text]");
  remember?.addEventListener("change", () => {
    if (closerText)
      closerText.textContent = remember.checked
        ? "Choose what the close button does. Your choice becomes the default; you can change it later in Settings, Window."
        : "Choose what the close button does this time. featherplay will ask again next time.";
  });
  function placeDialog() {
    if (!dialog) return;
    const p = pinRect();
    const f = shown;
    const cx = p.left + f.x + (f.w * f.s) / 2;
    const cy = p.top + f.y + (f.h * f.s) / 2;
    const w = dialog.offsetWidth;
    const h = dialog.offsetHeight;
    const vw = document.documentElement.clientWidth;
    dialog.style.left = `${Math.round(Math.min(Math.max(8, cx - w / 2), vw - w - 8))}px`;
    dialog.style.top = `${Math.round(Math.min(Math.max(8, cy - h / 2), innerHeight - h - 8))}px`;
  }
  const closeButton = () => replica.querySelector<HTMLElement>('[data-act="close"]');
  function askClose() {
    if (remembered) return closeAs(remembered);
    if (!dialog) return closeAs("tray");
    if (remember) remember.checked = false;
    remember?.dispatchEvent(new Event("change"));
    dialog.showModal();
    placeDialog();
    dialog.querySelector<HTMLElement>("[data-closer-do='tray']")?.focus();
  }
  dialog?.addEventListener("click", (e) => {
    const b = (e.target as Element).closest<HTMLElement>("[data-closer-do]");
    if (!b) return;
    const choice = b.dataset.closerDo as "tray" | "close";
    if (remember?.checked) remembered = choice;
    dialog.close();
    closeAs(choice);
  });
  dialog?.addEventListener("close", () => {
    if (!minimised && (document.activeElement === document.body || !document.activeElement)) closeButton()?.focus();
  });

  // ---- The caption buttons (replica.ts sends maximise and close; minimise is the store's) ----
  replica.addEventListener("fp-window", (e) => {
    const act = (e as CustomEvent<string>).detail;
    if (act === "maximise") setMax(!maxed);
    else if (act === "close") askClose();
  });
  titlebar.addEventListener("dblclick", (e) => {
    if ((e.target as Element).closest("button")) return;
    setMax(!maxed);
  });

  // ---- Dragging by the title bar: the window stays wholly in view ----
  let dragFrom: { px: number; py: number; x: number; y: number } | null = null;
  titlebar.addEventListener("pointerdown", (e) => {
    if (e.button !== 0 || !desk.matches || minimised || driven || (e.target as Element).closest("button")) return;
    e.preventDefault();
    titlebar.setPointerCapture(e.pointerId);
    for (const a of el.getAnimations()) a.cancel();
    if (maxed) {
      // A maximised window comes back to its size under the pointer, as Windows does.
      const f = shown;
      const k = (e.clientX - pinRect().left - f.x) / (f.w * f.s);
      markMax(false);
      const w = home.w * home.s;
      drag = { x: e.clientX - pinRect().left - k * w - home.x, y: f.y - home.y };
      draw(rested());
    }
    dragFrom = { px: e.clientX, py: e.clientY, x: drag.x, y: drag.y };
    el.setAttribute("data-dragging", "");
  });
  let moveFrame = 0;
  titlebar.addEventListener("pointermove", (e) => {
    // The story took the window over (the page scrolled under the drag): it draws it now.
    if (!dragFrom || driven) return;
    const want = { x: dragFrom.x + e.clientX - dragFrom.px, y: dragFrom.y + e.clientY - dragFrom.py };
    if (moveFrame) cancelAnimationFrame(moveFrame);
    moveFrame = requestAnimationFrame(() => {
      moveFrame = 0;
      const w = home.w * home.s;
      const h = home.h * home.s;
      const at = clampInto({ x: home.x + want.x, y: home.y + want.y, w, h }, viewportBox());
      drag = { x: at.x - home.x, y: at.y - home.y };
      draw(rested());
    });
  });
  const endDrag = () => {
    if (!dragFrom) return;
    dragFrom = null;
    el.removeAttribute("data-dragging");
  };
  titlebar.addEventListener("pointerup", endDrag);
  titlebar.addEventListener("pointercancel", endDrag);

  // ---- The wheel over the window scrolls the page until the visitor clicks into the app ----
  let idle = 0;
  const activate = () => {
    clearTimeout(idle);
    el.setAttribute("data-active", "");
  };
  el.addEventListener("pointerdown", activate, true);
  el.addEventListener("focusin", activate);
  el.addEventListener("pointerenter", () => clearTimeout(idle));
  el.addEventListener("pointerleave", () => {
    clearTimeout(idle);
    if (!el.contains(document.activeElement)) idle = window.setTimeout(() => el.removeAttribute("data-active"), 1000);
  });
  el.addEventListener("focusout", () =>
    setTimeout(() => {
      if (!el.contains(document.activeElement) && !el.matches(":hover")) el.removeAttribute("data-active");
    }),
  );
  // Not yet active: the app's own wheel handlers (the lyrics, the volume) don't see it, and the
  // lists can't scroll (CSS), so the page takes it, natively.
  el.addEventListener("wheel", (e) => {
    if (!el.hasAttribute("data-active") && !e.ctrlKey) e.stopPropagation();
  }, { capture: true });

  // ---- Measuring ----
  let resizeFrame = 0;
  const relayout = () => {
    if (resizeFrame) return;
    resizeFrame = requestAnimationFrame(() => {
      resizeFrame = 0;
      layout();
    });
  };
  new ResizeObserver(relayout).observe(slot);
  addEventListener("resize", relayout);
  desk.addEventListener("change", relayout);
  desk.addEventListener("change", deskOnly);
  deskOnly();
  layout();
  document.fonts?.ready.then(relayout);

  const controller: WindowController = {
    el,
    get home() {
      return home;
    },
    get frame() {
      return shown;
    },
    get rested() {
      return rested();
    },
    get maximised() {
      return maxed;
    },
    get minimised() {
      return minimised;
    },
    layout,
    drive(f) {
      driven = f;
      // The story takes over from a drag in progress.
      if (f && dragFrom) endDrag();
      // Out of sight (gone into the tray, faded for a change of shape), out of the keyboard's way;
      // keys that were in it go on from the steps after the stage.
      const out = minimised || (!!f && f.o < 0.5);
      if (out && !el.inert && el.contains(document.activeElement)) document.querySelector<HTMLElement>("[data-steps]")?.focus({ preventScroll: true });
      el.inert = out;
      if (f) {
        for (const a of el.getAnimations()) a.cancel();
        draw(f);
      } else settle();
    },
    forget() {
      drag = { x: 0, y: 0 };
      if (maxed) markMax(false);
    },
    trayPoint,
    onLayout(fn) {
      layoutFns.push(fn);
    },
    arrive(delay) {
      if (reduced() || driven || minimised || maxed) return;
      // Drawn invisibly first (it never freezes), then from 0.9 by its top right, the tray's
      // side, in 240 ms on the window curve.
      const to = shown;
      const from = { ...to, s: to.s * 0.9, x: to.x + to.w * to.s * 0.1, o: 0 };
      el.animate(
        [
          { transform: tf(from, slotOff), opacity: 0 },
          { transform: tf(to, slotOff), opacity: 1 },
        ],
        { duration: 240, delay, easing: EASE_WINDOW, fill: "backwards" },
      );
    },
  };
  ready(controller);
  return controller;
}
