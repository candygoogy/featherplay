// The bar (Bar.astro): two states and nothing in between. `data-state` is "docked" at the top
// of the page and "capsule" once it has moved (an observer on a sentinel 8 px down, never a
// scroll handler). Each state is laid out by CSS alone, so whatever happens (a resize, a
// reload part-way down, reduced motion, a state changing again mid-glide) the bar at rest is
// exactly its CSS layout. The change itself is decoration over the new layout: the parts that
// are in both states glide from where they were (FLIP, by `translate`), the ones only in the
// new state fade in, the grounds cross-fade (CSS). Measured once per change, never per frame.
//
// Also here: the section on screen lit, the theme toggle joining the capsule for the keyboard,
// Windows' tooltips with Windows' timing, the flyouts (the song, the downloads) as popovers
// under their part, the tray icon showing and hiding the window, and the desktop's menu.

import type { Player } from "../demo/player";
import { currentTheme, setTheme } from "./theme";

export type BarState = "docked" | "capsule";

const GLIDE_MS = 300;
const EASE_WINDOW = "cubic-bezier(0.32, 0.72, 0, 1)";
/** Windows: a tooltip after the pointer rests 400 ms; at once if one was showing just now. */
const TIP_DELAY = 400;
const TIP_RESHOW = 500;

const reduced = () => matchMedia("(prefers-reduced-motion: reduce)").matches;

export function bindBar(bar: HTMLElement, player: Player): { readonly state: BarState } {
  const row = bar.querySelector<HTMLElement>("[data-row]")!;
  const parts = () => [...row.querySelectorAll<HTMLElement>("[data-part]")];
  let state: BarState = bar.dataset.state === "capsule" ? "capsule" : "docked";

  const finish = () => {
    for (const p of parts()) for (const a of p.getAnimations()) if (!(a instanceof CSSTransition)) a.finish();
  };

  /** Changes the layout, and glides the parts from their old places to the new. */
  function relayout(change: () => void, animate = true) {
    finish();
    if (!animate || reduced()) {
      change();
      return;
    }
    const before = new Map(parts().map((p) => [p, p.getBoundingClientRect()]));
    change();
    for (const p of parts()) {
      const a = p.getBoundingClientRect();
      const b = before.get(p)!;
      const was = b.width > 0;
      const is = a.width > 0;
      if (was && is) {
        const dx = b.left - a.left;
        const dy = b.top - a.top;
        if (Math.abs(dx) < 0.5 && Math.abs(dy) < 0.5) continue;
        p.animate([{ translate: `${dx}px ${dy}px` }, { translate: "0 0" }], { duration: GLIDE_MS, easing: EASE_WINDOW });
      } else if (is) {
        p.animate([{ opacity: 0 }, { opacity: 1 }], { duration: 180, delay: 120, easing: "ease-out", fill: "backwards" });
      }
    }
  }

  function setState(next: BarState, animate = true) {
    if (next === state) return;
    state = next;
    hideTip();
    for (const f of flyouts) if (f.matches(":popover-open") && !held.has(f)) f.hidePopover();
    relayout(() => (bar.dataset.state = next), animate);
    bar.dispatchEvent(new CustomEvent("barstate", { detail: next }));
  }

  // ---- Docked at the top, the capsule once the page has moved ----
  const sentinel = document.querySelector("[data-bar-sentinel]");
  let first = true;
  if (sentinel && "IntersectionObserver" in window) {
    new IntersectionObserver(([e]) => {
      setState(e.isIntersecting ? "docked" : "capsule", !first);
      first = false;
    }).observe(sentinel);
  }
  // A resize lands every part where its CSS puts it at once.
  addEventListener("resize", finish);

  // ---- The section on screen, lit (the capsule shows its name) ----
  const links = [...bar.querySelectorAll<HTMLAnchorElement>("[data-sec]")];
  const sections = [...document.querySelectorAll<HTMLElement>("main > section[id], main > [data-stage]")];
  if (links.length && "IntersectionObserver" in window) {
    const ids = new Set(links.map((a) => a.dataset.sec));
    let shown: string | null = null;
    let inView: HTMLElement | null = null;
    // The first answer (a load part-way down) is drawn at once: nothing glides on load.
    const born = performance.now();
    const draw = () => {
      const id = inView && ids.has(inView.id) ? inView.id : null;
      if (id === shown) return;
      shown = id;
      relayout(() => {
        for (const a of links) {
          if (a.dataset.sec === id) a.setAttribute("aria-current", "true");
          else a.removeAttribute("aria-current");
        }
      }, state === "capsule" && performance.now() - born > 500);
    };
    const io = new IntersectionObserver(
      (es) => {
        for (const e of es) if (e.isIntersecting) inView = e.target as HTMLElement;
        draw();
      },
      { rootMargin: "-45% 0px -55% 0px" },
    );
    for (const s of sections) io.observe(s);
  }

  // ---- The keyboard brings the theme toggle into the capsule ----
  bar.addEventListener("focusin", (e) => {
    if (bar.hasAttribute("data-kbd") || !(e.target as Element).matches(":focus-visible")) return;
    relayout(() => bar.toggleAttribute("data-kbd", true), state === "capsule");
  });
  bar.addEventListener("focusout", () =>
    setTimeout(() => {
      if (bar.contains(document.activeElement)) return;
      relayout(() => bar.removeAttribute("data-kbd"), state === "capsule");
    }),
  );

  // ---- Windows' tooltips ----
  const tip = bar.querySelector<HTMLElement>("[data-bar-tip]")!;
  let tipTimer = 0;
  let tipFor: HTMLElement | null = null;
  let tipLast = -1e9;
  const showTip = (el: HTMLElement, now = false) => {
    clearTimeout(tipTimer);
    tipFor = el;
    const go = () => {
      if (tipFor !== el || flyouts.some((f) => f.matches(":popover-open"))) return;
      tip.textContent = el.dataset.tip ?? el.getAttribute("aria-label") ?? "";
      const r = el.getBoundingClientRect();
      const base = row.getBoundingClientRect().bottom + 8;
      const w = tip.offsetWidth;
      const vw = document.documentElement.clientWidth;
      tip.style.translate = `${Math.round(Math.min(Math.max(8, r.left + r.width / 2 - w / 2), vw - w - 8))}px ${Math.round(base)}px`;
      tip.toggleAttribute("data-on", true);
      tipLast = performance.now();
    };
    if (now || performance.now() - tipLast < TIP_RESHOW) go();
    else tipTimer = window.setTimeout(go, TIP_DELAY);
  };
  function hideTip() {
    clearTimeout(tipTimer);
    if (tipFor && tip.hasAttribute("data-on")) tipLast = performance.now();
    tipFor = null;
    tip.removeAttribute("data-on");
  }
  for (const el of bar.querySelectorAll<HTMLElement>("[data-tip], [data-theme-toggle]")) {
    el.addEventListener("pointerenter", (e) => e.pointerType === "mouse" && showTip(el));
    el.addEventListener("pointerleave", hideTip);
    el.addEventListener("focus", () => el.matches(":focus-visible") && showTip(el, true));
    el.addEventListener("blur", hideTip);
    el.addEventListener("pointerdown", hideTip);
  }
  // Escape puts a tooltip away (WCAG 1.4.13).
  document.addEventListener("keydown", (e) => e.key === "Escape" && hideTip());

  // ---- The flyouts: popovers under their part ----
  const songFly = bar.querySelector<HTMLElement>("#fly-song")!;
  const dlFly = bar.querySelector<HTMLElement>("#fly-dl")!;
  const flyouts = [songFly, dlFly];
  const np = bar.querySelector<HTMLElement>("[data-np]")!;
  const tray = bar.querySelector<HTMLElement>("[data-tray]")!;
  const dl = bar.querySelector<HTMLElement>("[data-dl]")!;
  const anchor = new Map<HTMLElement, HTMLElement>([
    [songFly, np],
    [dlFly, dl],
  ]);
  /** Flyouts opened by something other than the pointer resting (kept on a state change). */
  const held = new Set<HTMLElement>();
  const place = (fly: HTMLElement) => {
    const a = anchor.get(fly)!;
    const r = a.getBoundingClientRect();
    const vw = document.documentElement.clientWidth;
    const w = fly.offsetWidth;
    fly.style.left = `${Math.round(Math.min(Math.max(8, r.left + r.width / 2 - w / 2), vw - w - 8))}px`;
    fly.style.top = `${Math.round(row.getBoundingClientRect().bottom + 8)}px`;
  };
  for (const fly of flyouts) {
    fly.addEventListener("toggle", (e) => {
      const open = (e as ToggleEvent).newState === "open";
      if (open) {
        place(fly);
        hideTip();
      } else held.delete(fly);
      const a = anchor.get(fly)!;
      for (const el of fly === songFly ? [np, tray] : [dl]) el.setAttribute("aria-expanded", String(open && el === a));
    });
  }
  const open = (fly: HTMLElement, from: HTMLElement) => {
    anchor.set(fly, from);
    if (fly.matches(":popover-open")) place(fly);
    else fly.showPopover();
  };
  np.addEventListener("click", () => {
    anchor.set(songFly, np);
    held.add(songFly);
  });
  bar.addEventListener("barstate", () => {
    for (const f of flyouts) if (f.matches(":popover-open")) requestAnimationFrame(() => place(f));
  });

  // Resting the pointer on the tray icon or Download opens its flyout (not straight after a
  // click on the tray, which shows or hides the window); leaving both closes it.
  let restTimer = 0;
  let leaveTimer = 0;
  let trayClickAt = -1e9;
  const rest = (part: HTMLElement, fly: HTMLElement) => {
    part.addEventListener("pointerenter", (e) => {
      if (e.pointerType !== "mouse") return;
      clearTimeout(restTimer);
      clearTimeout(leaveTimer);
      if (part === tray && performance.now() - trayClickAt < 800) return;
      restTimer = window.setTimeout(() => open(fly, part), fly.matches(":popover-open") ? 0 : 280);
    });
    for (const el of [part, fly])
      el.addEventListener("pointerleave", () => {
        clearTimeout(restTimer);
        clearTimeout(leaveTimer);
        leaveTimer = window.setTimeout(() => {
          if (held.has(fly) || fly.matches(":hover") || part.matches(":hover") || fly.contains(document.activeElement)) return;
          if (fly.matches(":popover-open")) fly.hidePopover();
        }, 260);
      });
  };
  rest(tray, songFly);
  rest(dl, dlFly);
  // Download's flyout opens for the keyboard too, and Tab goes on into it (it follows the link).
  dl.addEventListener("focus", () => dl.matches(":focus-visible") && open(dlFly, dl));
  dlFly.addEventListener("focusout", () =>
    setTimeout(() => {
      if (!dlFly.contains(document.activeElement) && document.activeElement !== dl && dlFly.matches(":popover-open")) dlFly.hidePopover();
    }),
  );
  dl.addEventListener("blur", () =>
    setTimeout(() => {
      if (!dlFly.contains(document.activeElement) && !dl.matches(":hover") && dlFly.matches(":popover-open")) dlFly.hidePopover();
    }),
  );

  // The tray icon shows and hides the window, as Windows' does.
  tray.addEventListener("click", () => {
    trayClickAt = performance.now();
    clearTimeout(restTimer);
    if (songFly.matches(":popover-open")) songFly.hidePopover();
    if (player.state.minimised) player.restore();
    else player.minimise();
  });
  bar.querySelector("[data-show-window]")?.addEventListener("click", () => {
    player.restore();
    songFly.hidePopover();
  });

  // ---- The song: the capsule's cover and name, the queue's current row ----
  const npCover = bar.querySelector<HTMLImageElement>("[data-np-cover]");
  const rows = [...bar.querySelectorAll<HTMLElement>(".fq")];
  let shownSong = "";
  player.subscribe((s) => {
    if (!s.song || s.song.id === shownSong) return;
    shownSong = s.song.id;
    if (npCover) npCover.src = s.song.cover;
    np.setAttribute("aria-label", `${s.song.title}: the song and the player`);
    for (const r of rows) {
      if (r.dataset.song === s.song.id) r.setAttribute("aria-current", "true");
      else r.removeAttribute("aria-current");
    }
  });

  // ---- The desktop's menu: a right-click on the empty desk, Shift+F10 or the menu key ----
  const ctx = bar.querySelector<HTMLElement>("[data-ctx]")!;
  const items = [...ctx.querySelectorAll<HTMLElement>(".ctx-item")];
  let ctxBack: HTMLElement | null = null;
  // The desk is anything that isn't a control, a word or a picture (the window keeps its own).
  const NOT_EMPTY =
    "a, button, input, textarea, select, label, summary, p, h1, h2, h3, h4, li, dt, dd, td, th, img, svg, figure, [data-window], [data-site-bar], [popover], [data-piece], [role]";
  const empty = (t: EventTarget | null) => t instanceof Element && !t.closest(NOT_EMPTY);
  const openCtx = (x: number, y: number) => {
    const play = ctx.querySelector("[data-ctx-play]");
    if (play) play.textContent = player.state.playing ? "Pause" : "Play";
    const th = ctx.querySelector("[data-ctx-theme]");
    if (th) th.textContent = currentTheme() === "dark" ? "Switch to light" : "Switch to dark";
    ctxBack = document.activeElement as HTMLElement | null;
    ctx.showPopover();
    const w = ctx.offsetWidth;
    const h = ctx.offsetHeight;
    const vw = document.documentElement.clientWidth;
    ctx.style.left = `${Math.round(Math.max(8, x + w > vw - 8 ? x - w : x))}px`;
    ctx.style.top = `${Math.round(Math.max(8, y + h > innerHeight - 8 ? y - h : y))}px`;
    items[0].focus();
  };
  document.addEventListener("contextmenu", (e) => {
    if (!empty(e.target) || String(getSelection()).trim()) return;
    e.preventDefault();
    openCtx(e.clientX, e.clientY);
  });
  document.addEventListener("keydown", (e) => {
    if (!(e.key === "ContextMenu" || (e.shiftKey && e.key === "F10"))) return;
    const a = document.activeElement;
    if (a && a !== document.body && !empty(a)) return;
    e.preventDefault();
    openCtx(innerWidth / 2 - 120, innerHeight / 2 - 80);
  });
  // Closed (Escape, a choice, a click elsewhere): the keys go back where they were.
  ctx.addEventListener("toggle", (e) => {
    if ((e as ToggleEvent).newState !== "closed") return;
    const a = document.activeElement;
    if (ctxBack?.isConnected && (!a || a === document.body || ctx.contains(a))) ctxBack.focus({ preventScroll: true });
  });
  ctx.addEventListener("click", (e) => {
    const it = (e.target as Element).closest<HTMLElement>("[data-ctx-do]");
    if (!it) return;
    const act = it.dataset.ctxDo;
    if (act === "toggle") player.toggle();
    else if (act === "next") player.next();
    else if (act === "theme") setTheme(currentTheme() === "dark" ? "light" : "dark");
    ctx.hidePopover();
  });
  ctx.addEventListener("keydown", (e) => {
    const i = items.indexOf(document.activeElement as HTMLElement);
    const n = items.length;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      items[(i + (e.key === "ArrowDown" ? 1 : n - 1) + n) % n].focus();
    } else if (e.key === "Home" || e.key === "End") {
      e.preventDefault();
      items[e.key === "Home" ? 0 : n - 1].focus();
    } else if (e.key === "Tab") {
      e.preventDefault();
      ctx.hidePopover();
    }
  });
  addEventListener("scroll", () => ctx.matches(":popover-open") && ctx.hidePopover(), { passive: true });

  return {
    get state() {
      return state;
    },
  };
}
