// The shelf's sleeves (Shelf.astro): click one, press Enter or Space on it, tap it, or pick it up
// and drop it on the player, and it plays. A sleeve carried and let go anywhere else goes back
// without playing. While a sleeve is carried over the
// window, the window shows a ring; dropped there, the sleeve goes into it. The one playing is
// marked. Nothing plays until the visitor does one of these.

import type { Player } from "../demo/player";

const reduced = () => matchMedia("(prefers-reduced-motion: reduce)").matches;
/** The carried sleeve stays below the docked bar. */
const BAR = 56;

export function bindShelf(shelf: HTMLElement, player: Player): void {
  const sleeves = [...shelf.querySelectorAll<HTMLButtonElement>("[data-sleeve]")];
  const win = () => document.querySelector<HTMLElement>("[data-window]");
  /** The window's box as a drop target, or null where it can't take one. Read at pick-up and
   *  after a scroll, not on every move (each read restyled the page after the move's write). */
  const target = (): DOMRect | null => {
    const w = win();
    if (!w || w.hasAttribute("data-minimised") || getComputedStyle(w).visibility === "hidden") return null;
    return w.getBoundingClientRect();
  };
  let box: DOMRect | null = null;
  const over = (x: number, y: number) => !!box && x >= box.left && x <= box.right && y >= box.top && y <= box.bottom;
  const play = (id: string) => player.play(id);

  // The one playing.
  let shown = "";
  player.subscribe((s) => {
    const id = s.song?.id ?? "";
    if (id === shown) return;
    shown = id;
    for (const el of sleeves) {
      const on = el.dataset.sleeve === id;
      el.toggleAttribute("data-current", on);
      if (on) el.setAttribute("aria-current", "true");
      else el.removeAttribute("aria-current");
    }
  });

  // Out of view, the playing sleeve's mark holds still (app-controls.css: [data-away]).
  if ("IntersectionObserver" in window)
    new IntersectionObserver(([e]) => {
      for (const m of shelf.querySelectorAll(".sl-mark")) m.toggleAttribute("data-away", !e.isIntersecting);
    }).observe(shelf);

  let held: { el: HTMLElement; sx: number; sy: number; moved: boolean; carry?: HTMLElement; ox: number; oy: number } | null = null;
  /** This press became a carry: the click that ends it plays nothing. */
  let carried = false;
  let dropOn = false;
  const mark = (on: boolean) => {
    if (on === dropOn) return;
    dropOn = on;
    win()?.toggleAttribute("data-drop", on);
  };
  addEventListener("scroll", () => held?.moved && (box = target()), { passive: true });
  /** Escape puts the carried sleeve back. */
  let cancel: (() => void) | null = null;
  addEventListener("keydown", (e) => e.key === "Escape" && cancel?.());
  const ghostTimers = new WeakMap<HTMLElement, number>();
  for (const el of sleeves) {
    const id = el.dataset.sleeve!;
    el.addEventListener("pointerdown", (e) => {
      carried = false;
      if (e.button !== 0 || e.pointerType === "touch") return;
      e.preventDefault();
      el.focus({ preventScroll: true });
      el.setPointerCapture(e.pointerId);
      held = { el, sx: e.clientX, sy: e.clientY, moved: false, ox: 0, oy: 0 };
      el.classList.add("held");
    });
    el.addEventListener("pointermove", (e) => {
      if (!held || held.el !== el) return;
      if (!held.moved) {
        if (Math.hypot(e.clientX - held.sx, e.clientY - held.sy) < 5) return;
        held.moved = true;
        carried = true;
        const r = el.getBoundingClientRect();
        const c = el.querySelector("img")!.cloneNode() as HTMLElement;
        c.className = "carry";
        c.setAttribute("aria-hidden", "true");
        Object.assign(c.style, { width: `${r.width}px`, height: `${r.height}px` });
        document.body.append(c);
        held.carry = c;
        held.ox = r.left - held.sx;
        held.oy = r.top - held.sy;
        // (A second pick-up during the last drop's end: that drop's timer mustn't show it.)
        clearTimeout(ghostTimers.get(el));
        el.classList.add("ghosted");
        box = target();
        cancel = () => end({ type: "cancel", clientX: 0, clientY: 0 });
      }
      held.carry!.style.transform = `translate(${e.clientX + held.ox}px, ${Math.max(BAR, e.clientY + held.oy)}px)`;
      mark(over(e.clientX, e.clientY));
    });
    const end = (e: { type: string; clientX: number; clientY: number }) => {
      if (!held || held.el !== el) return;
      const h = held;
      held = null;
      cancel = null;
      el.classList.remove("held");
      const drop = h.moved && e.type === "pointerup" && over(e.clientX, e.clientY);
      mark(false);
      box = null;
      if (!h.moved || !h.carry) return;
      const c = h.carry;
      const from = c.style.transform;
      const ms = reduced() ? 1 : drop ? 300 : 320;
      let to: string;
      if (drop) {
        play(id);
        const r = win()!.getBoundingClientRect();
        to = `translate(${r.left + r.width * 0.12}px, ${r.top + r.height * 0.3}px) scale(0.35)`;
      } else {
        const r = el.getBoundingClientRect();
        to = `translate(${r.left}px, ${r.top}px)`;
      }
      c.animate([{ transform: from, opacity: 1 }, { transform: to, opacity: drop ? 0 : 1 }], {
        duration: ms,
        easing: drop ? "cubic-bezier(0.32, 0.72, 0, 1)" : "cubic-bezier(0.22, 1, 0.36, 1)",
        fill: "forwards",
      }).finished.then(() => c.remove());
      ghostTimers.set(el, window.setTimeout(() => el.classList.remove("ghosted"), drop ? 120 : ms));
    };
    el.addEventListener("pointerup", end);
    el.addEventListener("pointercancel", end);
    // A click plays: the mouse, the keyboard (Enter, Space: a click with no pointer) or a tap.
    // Not the click that ends a carry, and not the second click of a double-click (the first
    // has started the song).
    el.addEventListener("click", (e) => {
      if (e.detail === 0 || (!carried && e.detail < 2)) play(id);
      carried = false;
    });
  }
}
