// The tour's cursor: a pointer that travels to a control inside the laptop and presses it.
// It lives in the hero's stage (page pixels), above the laptop, and moves by transform only.
// A move or a press stops at once when its signal aborts (the tour's Stop).

const reduced = () => matchMedia("(prefers-reduced-motion: reduce)").matches;

export type Cursor = {
  moveTo(target: Element, signal?: AbortSignal): Promise<void>;
  press(signal?: AbortSignal): Promise<void>;
  /** Keys held while it acts ("Ctrl V"), shown by the pointer; null hides them. */
  keys(text: string | null): void;
  hide(): void;
};

export function createCursor(el: HTMLElement, stage: HTMLElement): Cursor {
  let x = 0;
  let y = 0;
  let placed = false;
  const badge = el.querySelector<HTMLElement>("[data-tour-keys]");

  const pointOf = (target: Element) => {
    const t = target.getBoundingClientRect();
    const s = stage.getBoundingClientRect();
    // The pointer's tip rests a little inside the target, as a hand would.
    return { x: t.left - s.left + t.width * 0.5, y: t.top - s.top + t.height * 0.55 };
  };

  /** Runs an animation to its end, or cancels it when the signal aborts. */
  const play = async (anim: Animation, signal?: AbortSignal) => {
    const stop = () => anim.cancel();
    signal?.addEventListener("abort", stop, { once: true });
    await anim.finished.catch(() => {});
    signal?.removeEventListener("abort", stop);
  };

  const cursor: Cursor = {
    async moveTo(target, signal) {
      if (signal?.aborted) return;
      const to = pointOf(target);
      if (!placed) {
        // Enter from the lower right of the screen.
        const s = stage.getBoundingClientRect();
        x = Math.min(s.width - 40, to.x + 220);
        y = Math.min(s.height - 40, to.y + 160);
        placed = true;
      }
      el.removeAttribute("hidden");
      const from = `translate(${x}px, ${y}px)`;
      const end = `translate(${to.x}px, ${to.y}px)`;
      const distance = Math.hypot(to.x - x, to.y - y);
      x = to.x;
      y = to.y;
      el.style.transform = end;
      if (reduced()) return;
      await play(
        el.animate([{ transform: from }, { transform: end }], {
          duration: Math.min(850, 420 + distance * 0.6),
          easing: "cubic-bezier(0.77, 0, 0.175, 1)",
        }),
        signal,
      );
    },
    async press(signal) {
      if (reduced() || signal?.aborted) return;
      const at = `translate(${x}px, ${y}px)`;
      await play(
        el.animate([{ transform: at }, { transform: `${at} scale(0.82)` }, { transform: at }], {
          duration: 220,
          easing: "cubic-bezier(0.23, 1, 0.32, 1)",
        }),
        signal,
      );
    },
    keys(text) {
      if (!badge) return;
      badge.hidden = !text;
      badge.textContent = text ?? "";
    },
    hide() {
      el.setAttribute("hidden", "");
      cursor.keys(null);
      placed = false;
    },
  };
  return cursor;
}
