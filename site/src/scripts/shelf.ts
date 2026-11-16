// The shelf's sleeves (Shelf.astro): pick one up and drop it on the player, double-click it,
// press Enter or Space on it, or tap it on a touch screen, and it plays. A mouse's single click
// only picks up (a click is how a sleeve is taken in hand). While a sleeve is carried over the
// window, the window shows a ring; dropped there, the sleeve goes into it. The one playing is
// marked. Nothing plays until the visitor does one of these.

import type { Player } from "../demo/player";

const reduced = () => matchMedia("(prefers-reduced-motion: reduce)").matches;
/** The carried sleeve stays below the docked bar. */
const BAR = 56;

export function bindShelf(shelf: HTMLElement, player: Player): void {
  const sleeves = [...shelf.querySelectorAll<HTMLButtonElement>("[data-sleeve]")];
  const win = () => document.querySelector<HTMLElement>("[data-window]");
  const over = (x: number, y: number) => {
    const w = win();
    if (!w || w.hasAttribute("data-minimised") || getComputedStyle(w).visibility === "hidden") return false;
    const r = w.getBoundingClientRect();
    return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
  };
  const play = (id: string) => player.play(id);

  // The one playing.
  let shown = "";
  player.subscribe((s) => {
    const id = s.song?.id ?? "";
    if (id === shown) return;
    shown = id;
    for (const el of sleeves) el.toggleAttribute("data-current", el.dataset.sleeve === id);
  });

  // Out of view, the playing sleeve's mark holds still (app-controls.css: [data-away]).
  if ("IntersectionObserver" in window)
    new IntersectionObserver(([e]) => {
      for (const m of shelf.querySelectorAll(".sl-mark")) m.toggleAttribute("data-away", !e.isIntersecting);
    }).observe(shelf);

  let held: { el: HTMLElement; sx: number; sy: number; moved: boolean; carry?: HTMLElement; ox: number; oy: number } | null = null;
  for (const el of sleeves) {
    const id = el.dataset.sleeve!;
    el.addEventListener("pointerdown", (e) => {
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
        const r = el.getBoundingClientRect();
        const c = el.querySelector("img")!.cloneNode() as HTMLElement;
        c.className = "carry";
        c.setAttribute("aria-hidden", "true");
        Object.assign(c.style, { width: `${r.width}px`, height: `${r.height}px` });
        document.body.append(c);
        held.carry = c;
        held.ox = r.left - held.sx;
        held.oy = r.top - held.sy;
        el.classList.add("ghosted");
      }
      held.carry!.style.transform = `translate(${e.clientX + held.ox}px, ${Math.max(BAR, e.clientY + held.oy)}px)`;
      win()?.toggleAttribute("data-drop", over(e.clientX, e.clientY));
    });
    const end = (e: PointerEvent) => {
      if (!held || held.el !== el) return;
      const h = held;
      held = null;
      el.classList.remove("held");
      const drop = h.moved && e.type === "pointerup" && over(e.clientX, e.clientY);
      win()?.removeAttribute("data-drop");
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
      setTimeout(() => el.classList.remove("ghosted"), drop ? 120 : ms);
    };
    el.addEventListener("pointerup", end);
    el.addEventListener("pointercancel", end);
    el.addEventListener("dblclick", () => play(id));
    // The keyboard (Enter, Space: a click with no pointer) and touch play at once.
    el.addEventListener("click", (e) => {
      const touch = (e as PointerEvent).pointerType === "touch" || matchMedia("(pointer: coarse)").matches;
      if (e.detail === 0 || touch) play(id);
    });
  }
}
