// The guided tour on the hero's laptop. It starts from the frame the hero shows (paused, the
// lyrics open) and moves on whenever its step is done, whether the visitor did it or
// "Show me" did. Then free play.

import type { Player, PlayerState } from "./player";
import { createCursor, type Cursor } from "./cursor";

export type TourStepId = "play" | "pick" | "link" | "tray" | "download";

export type TourStep = {
  id: TourStepId;
  title: string;
  body: string;
  /** Where the step points, inside [data-replica] (the first that's on screen wins). */
  target: string;
  done: (s: PlayerState) => boolean;
  /** Does the step on the replica, a visible move at a time; stops when `signal` aborts. */
  showMe: (signal: AbortSignal) => Promise<void>;
};

export type Tour = {
  readonly index: number; // -1 when closed
  readonly step: TourStep | null;
  /** True while "Show me" is doing its step. */
  readonly running: boolean;
  start(): void;
  /** Back to the step it was on when it was closed (or the first). */
  resume(): void;
  /** Goes ahead a step without doing this one. */
  next(): void;
  /** Closes the tour; resume() comes back to the same step. */
  skip(): void;
  showMe(): Promise<void>;
  /** Stops "Show me" part way; the step stays, to do or show again. */
  stop(): void;
  onChange(fn: () => void): void;
};

export function createTour(player: Player, steps: TourStep[]): Tour {
  let index = -1;
  let kept = 0; // the step a closed tour comes back to
  let run: AbortController | null = null;
  const listeners: (() => void)[] = [];
  const changed = () => listeners.forEach((fn) => fn());

  const go = (i: number) => {
    run?.abort();
    run = null;
    index = i;
    if (i >= 0) kept = i;
    changed();
  };
  const last = steps.length - 1;

  player.subscribe((s) => {
    if (index < 0 || run) return;
    const step = steps[index];
    if (step && index < last && step.done(s)) go(index + 1);
  });

  const tour: Tour = {
    get index() {
      return index;
    },
    get step() {
      return index >= 0 ? steps[index] : null;
    },
    get running() {
      return run !== null;
    },
    start() {
      go(0);
    },
    resume() {
      go(Math.min(kept, last));
    },
    next() {
      if (index >= 0 && index < last) go(index + 1);
    },
    skip() {
      const at = index;
      go(-1);
      kept = Math.max(0, at);
    },
    async showMe() {
      const step = steps[index];
      if (!step || run) return;
      const mine = new AbortController();
      run = mine;
      changed();
      try {
        await step.showMe(mine.signal);
      } catch (e) {
        if (!mine.signal.aborted) throw e;
      }
      if (run !== mine) return; // stopped, closed or moved on meanwhile
      run = null;
      if (steps[index] === step && step.done(player.state) && index < last) go(index + 1);
      else changed();
    },
    stop() {
      if (!run) return;
      run.abort();
      run = null;
      changed();
    },
    onChange(fn) {
      listeners.push(fn);
    },
  };
  return tour;
}

const wait = (ms: number, signal?: AbortSignal) =>
  new Promise<void>((resolve, reject) => {
    if (signal?.aborted) return reject(signal.reason);
    const t = setTimeout(resolve, ms);
    signal?.addEventListener(
      "abort",
      () => {
        clearTimeout(t);
        reject(signal.reason);
      },
      { once: true },
    );
  });
export const TOUR_LINK = "https://music.example/watch?v=she-dreams-in-blue";

/** The five steps, wired to the replica in `replica` and the cursor. */
export function heroSteps(player: Player, replica: HTMLElement, cursor: Cursor): TourStep[] {
  // A minimised window is still laid out (shrunk into the taskbar, inert): nothing in it counts.
  const visible = (sel: string) =>
    [...replica.querySelectorAll<HTMLElement>(sel)].find(
      (el) => el.offsetParent !== null && el.getClientRects().length && !el.closest("[inert]"),
    ) ?? null;
  const taskbarApp = replica.closest("[data-desktop]")?.querySelector<HTMLElement>("[data-taskbar-app]") ?? null;
  const firstVisible = (sels: string) => {
    for (const sel of sels.split("|")) {
      const el = visible(sel.trim());
      if (el) return el;
    }
    return null;
  };
  const check = (signal: AbortSignal) => {
    if (signal.aborted) throw signal.reason;
  };
  /** The cursor goes to the first of `sels` on screen and clicks it. */
  const press = async (sels: string, signal: AbortSignal) => {
    const el = firstVisible(sels);
    if (!el) return null;
    await cursor.moveTo(el, signal);
    check(signal);
    await cursor.press(signal);
    check(signal);
    el.click();
    return el;
  };
  /** Minimised (by the visitor or the tray step), the window comes back from the taskbar first. */
  const bringBack = async (signal: AbortSignal) => {
    if (!player.state.minimised || !taskbarApp) return;
    await cursor.moveTo(taskbarApp, signal);
    check(signal);
    await cursor.press(signal);
    check(signal);
    taskbarApp.click();
    await wait(360, signal); // the window grows back out of the taskbar (replica.css)
  };
  let songAtPick: string | null = null;

  return [
    {
      id: "play",
      title: "Press play.",
      body: "It plays a short clip of a real song, and the lyrics follow along.",
      target: ".play",
      done: (s) => {
        songAtPick = null; // the next step measures "another song" from where this one ends
        return s.playing;
      },
      showMe: async (signal) => {
        await bringBack(signal);
        await press(".play", signal);
      },
    },
    {
      id: "pick",
      title: "Pick another song.",
      body: "Choose one from Up next, or skip ahead, and the page takes its colour.",
      target: '[data-row]:not([aria-current]) | [data-act="next"]',
      done: (s) => {
        songAtPick ??= s.song?.id ?? null;
        return !!s.song && s.song.id !== songAtPick;
      },
      showMe: async (signal) => {
        await bringBack(signal);
        await press('.queue [data-row]:not([aria-current]) | [data-act="next"]', signal);
      },
    },
    {
      id: "link",
      title: "Find a song by its link.",
      body: "Copy this link and paste it into featherplay's search box. It finds the song.",
      target: '[data-search-input] | [data-act="back"]',
      done: (s) => s.view === "search" && !!s.result,
      showMe: async (signal) => {
        await bringBack(signal);
        // Search lives on the browse pages: from Now playing, Back first.
        if (player.state.view === "now" || player.state.view === "lyrics") {
          await press('[data-act="back"]', signal);
          await wait(320, signal);
        }
        const field = visible("[data-search-input]") as HTMLInputElement | null;
        if (!field) return;
        await cursor.moveTo(field, signal);
        await cursor.press(signal);
        check(signal);
        field.focus({ preventScroll: true });
        // Paste: the keys show by the pointer, then the link appears and it searches.
        cursor.keys("Ctrl V");
        await wait(380, signal);
        field.value = TOUR_LINK;
        player.search(TOUR_LINK);
        await wait(260, signal);
        cursor.keys(null);
        // Then the pointer rests on what it found.
        const found = visible("[data-search-result] [data-row]");
        if (found) await cursor.moveTo(found, signal);
        await wait(420, signal);
      },
    },
    {
      id: "tray",
      title: "Minimise it to the tray.",
      body: "It keeps playing in the background. Click it in the taskbar to bring it back.",
      target: '[data-act="minimise"]',
      done: (s) => s.minimised,
      showMe: async (signal) => {
        await press('[data-act="minimise"]', signal);
        await wait(300, signal);
      },
    },
    {
      id: "download",
      title: "That's featherplay.",
      body: "One small program for Windows 10 and 11, free: installer or portable exe. Or keep playing here.",
      target: "",
      done: () => false,
      showMe: async () => {},
    },
  ];
}

type Box = { left: number; top: number; right: number; bottom: number };
const hits = (a: Box, b: Box) => a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;
const grow = (r: Box, by: number): Box => ({ left: r.left - by, top: r.top - by, right: r.right + by, bottom: r.bottom + by });

/**
 * Where the card goes (page pixels): beside the laptop when the page has room on a side
 * (the target's side first), level with the target; otherwise on the screen, at the first
 * of a few places that covers neither the target nor anything in `avoid`.
 */
export function placeCard(
  card: { w: number; h: number },
  laptop: Box,
  screen: Box,
  view: { width: number },
  target: Box | null,
  avoid: Box[],
  pad = 16,
): { x: number; y: number; beside: boolean } {
  const cy = target ? (target.top + target.bottom) / 2 : screen.top + (screen.bottom - screen.top) * 0.4;
  const clampY = (y: number) => Math.min(Math.max(y, screen.top), screen.bottom - card.h);
  const roomL = laptop.left - pad;
  const roomR = view.width - laptop.right - pad;
  const fits = (room: number) => room >= card.w + pad;
  const onLeft = target ? (target.left + target.right) / 2 < (screen.left + screen.right) / 2 : true;
  const y = clampY(cy - card.h / 2);
  if (onLeft && fits(roomL)) return { x: laptop.left - card.w - pad, y, beside: true };
  if (fits(roomR)) return { x: laptop.right + pad, y, beside: true };
  if (fits(roomL)) return { x: laptop.left - card.w - pad, y, beside: true };

  // On the screen: beside the target, then the screen's corners.
  const blocked = [...(target ? [grow(target, 12)] : []), ...avoid.map((r) => grow(r, 8))];
  const inX = (x: number) => Math.min(Math.max(x, screen.left + pad), screen.right - card.w - pad);
  const spots = [
    ...(target
      ? [
          { x: target.left - card.w - pad, y },
          { x: target.right + pad, y },
          { x: inX((target.left + target.right) / 2 - card.w / 2), y: target.top - card.h - pad },
          { x: inX((target.left + target.right) / 2 - card.w / 2), y: target.bottom + pad },
        ]
      : []),
    { x: screen.left + pad, y: screen.top + pad },
    { x: screen.right - card.w - pad, y: screen.top + pad },
    { x: screen.left + pad, y: screen.bottom - card.h - pad },
    { x: screen.right - card.w - pad, y: screen.bottom - card.h - pad },
  ];
  const inside = (p: { x: number; y: number }) =>
    p.x >= screen.left && p.x + card.w <= screen.right && p.y >= screen.top && p.y + card.h <= screen.bottom;
  const free = spots.find((p) => {
    if (!inside(p)) return false;
    const box = { left: p.x, top: p.y, right: p.x + card.w, bottom: p.y + card.h };
    return !blocked.some((b) => hits(box, b));
  });
  const fallback = spots[spots.length - 1];
  const p = free ?? { x: inX(fallback.x), y: clampY(fallback.y) };
  return { ...p, beside: false };
}

/** Puts the tour on the page: the card, the ring round the target and the cursor. */
export function mountTour(root: HTMLElement, player: Player): Tour | null {
  const stage = root.closest<HTMLElement>("[data-hero-stage]") ?? root.parentElement!;
  const replica = stage.querySelector<HTMLElement>("[data-replica]");
  const laptop = stage.querySelector<HTMLElement>("[data-laptop]");
  if (!replica || !laptop) return null;
  const $ = <T extends HTMLElement = HTMLElement>(sel: string) => root.querySelector<T>(sel)!;
  const card = $("[data-tour-card]");
  const title = $("[data-tour-title]");
  const body = $("[data-tour-body]");
  const linkRow = $("[data-tour-linkrow]");
  const copy = $<HTMLButtonElement>("[data-tour-copy]");
  const show = $<HTMLButtonElement>("[data-tour-show]");
  const showLabel = $("[data-tour-show-label]");
  const nextBtn = $<HTMLButtonElement>("[data-tour-next]");
  const skip = $<HTMLButtonElement>("[data-tour-skip]");
  const download = $("[data-tour-download]");
  const restart = $<HTMLButtonElement>("[data-tour-restart]");
  const reopen = $<HTMLButtonElement>("[data-tour-reopen]");
  const starter = document.querySelector<HTMLButtonElement>("[data-tour-start]");
  const ring = $("[data-tour-ring]");
  const dots = [...root.querySelectorAll<HTMLElement>("[data-tour-dot]")];
  const cursor = createCursor($("[data-tour-cursor]"), stage);
  const steps = heroSteps(player, replica, cursor);
  const tour = createTour(player, steps);
  const phone = matchMedia("(max-width: 768px)");
  const startSong = replica.dataset.song!;
  const startAt = Number(replica.dataset.at ?? 0);
  const desktop = laptop.querySelector<HTMLElement>("[data-desktop]");
  const taskbarApp = desktop?.querySelector<HTMLElement>("[data-taskbar-app]") ?? null;

  const targetOf = (step: TourStep) => {
    for (const sel of step.target.split("|")) {
      const s = sel.trim();
      if (!s) continue;
      const el = [...replica.querySelectorAll<HTMLElement>(s)].find((e) => e.offsetParent !== null);
      if (el) return el;
    }
    return null;
  };
  // What the card must never cover when it has to sit on the screen: the memory panel.
  const avoid = () =>
    [...(desktop?.querySelectorAll<HTMLElement>("[data-meter]") ?? [])]
      .filter((el) => getComputedStyle(el).visibility !== "hidden" && el.getClientRects().length)
      .map((el) => el.getBoundingClientRect());

  // Place the card beside the laptop (or on its screen, clear of what matters), and draw
  // the ring round the target.
  let frame = 0;
  function place() {
    frame = 0;
    // Hidden until the lid settles (Tour.astro): nothing to place while the hero's scroll
    // moment runs, so its frames do no layout reads or writes for the tour. Settling places
    // it at once (the observer below), before the frame that shows it.
    if (laptop!.dataset.settled !== "true") return;
    const step = tour.step;
    // Minimised, every step but the tray's starts from the taskbar button: the ring shows the way back.
    const back = player.state.minimised && step && step.id !== "tray" && step.id !== "download";
    const target = !step ? null : back ? taskbarApp : player.state.minimised ? null : targetOf(step);
    const s = stage.getBoundingClientRect();
    const t = target?.getBoundingClientRect() ?? null;
    if (t) {
      ring.hidden = false;
      ring.style.transform = `translate(${t.left - s.left - 4}px, ${t.top - s.top - 4}px)`;
      ring.style.width = `${t.width + 8}px`;
      ring.style.height = `${t.height + 8}px`;
    } else {
      ring.hidden = true;
    }
    if (phone.matches) {
      card.style.transform = "";
      return;
    }
    const screen = laptop!.querySelector("[data-screen]")!.getBoundingClientRect();
    // Beside the lid, not the wider deck: the card stays level with the screen (placeCard
    // keeps it above the screen's bottom edge), so it never reaches the deck.
    const p = placeCard(
      { w: card.offsetWidth, h: card.offsetHeight },
      (laptop!.querySelector("[data-laptop-body]") ?? laptop!).getBoundingClientRect(),
      screen,
      { width: document.documentElement.clientWidth },
      t,
      avoid(),
    );
    card.dataset.beside = String(p.beside);
    card.style.transform = `translate(${Math.round(p.x - s.left)}px, ${Math.round(p.y - s.top)}px)`;
  }
  const schedule = () => {
    if (!frame) frame = requestAnimationFrame(place);
  };

  const firstAction = () => [show, download].find((b) => !b.hidden);
  let shownStep = -2;
  function render() {
    const step = tour.step;
    const on = !!step;
    card.hidden = !on;
    reopen.hidden = on;
    if (starter) starter.dataset.running = String(on);
    if (!on) {
      // The way back in, where the card was.
      reopen.style.transform = card.style.transform;
      ring.hidden = true;
      cursor.hide();
      shownStep = -2;
      return;
    }
    const end = step.id === "download";
    const running = tour.running;
    show.hidden = end;
    show.dataset.running = String(running);
    showLabel.textContent = running ? "Stop" : "Show me";
    nextBtn.hidden = end || running;
    download.hidden = !end;
    restart.hidden = !end;
    if (!running) cursor.keys(null);
    if (end) cursor.hide();
    if (tour.index !== shownStep) {
      shownStep = tour.index;
      title.textContent = step.title;
      body.textContent = step.body;
      linkRow.hidden = step.id !== "link";
      dots.forEach((d, i) => (d.dataset.on = String(i <= tour.index)));
      // A new step fades in where it lands (it may change sides): no slide across the laptop.
      card.animate?.([{ opacity: 0 }, { opacity: 1 }], { duration: 200, easing: "cubic-bezier(0.23, 1, 0.32, 1)" });
    }
    schedule();
  }

  tour.onChange(render);
  player.subscribe(() => {
    if (tour.step) schedule();
  });
  // Scrolling moves the card only while the laptop is on screen: further down the page the
  // scroll frames do no layout reads for it; coming back places it again.
  let near = true;
  const onScroll = () => {
    if (near) schedule();
  };
  if (typeof IntersectionObserver !== "undefined") {
    new IntersectionObserver(([e]) => {
      near = e.isIntersecting;
      if (near) schedule();
    }).observe(laptop);
  }
  addEventListener("resize", schedule, { passive: true });
  addEventListener("scroll", onScroll, { passive: true });
  // The lid settling moves the screen (the stage stops scaling): place the card again then.
  new MutationObserver(() => {
    cancelAnimationFrame(frame);
    place();
  }).observe(laptop, { attributes: true, attributeFilter: ["data-settled"] });

  // Show me, and while it runs, Stop (the same button, so focus stays put).
  show.addEventListener("click", () => {
    if (tour.running) {
      tour.stop();
      cursor.hide();
      return;
    }
    void tour.showMe().then(() => {
      // Done and moved on: focus the new step's first action if focus was left behind.
      if (tour.running || !tour.step) return;
      const active = document.activeElement;
      if (!active || active === document.body || active.closest("[hidden]")) firstAction()?.focus({ preventScroll: true });
    });
  });
  nextBtn.addEventListener("click", () => {
    tour.next();
    firstAction()?.focus({ preventScroll: true });
  });
  skip.addEventListener("click", () => {
    cursor.hide();
    tour.skip();
    reopen.focus({ preventScroll: true });
  });
  reopen.addEventListener("click", () => {
    tour.resume();
    firstAction()?.focus({ preventScroll: true });
  });
  // "Take the tour" (beside the download): brings the laptop into view, open, and goes on
  // with the tour where it was left (or starts it), with focus on the step's first action.
  starter?.addEventListener("click", () => {
    if (!tour.step) tour.resume();
    const hero = laptop.closest<HTMLElement>("[data-hero]");
    const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;
    const pinned = !phone.matches && !reduce && hero;
    const top = pinned
      ? hero.offsetTop + innerHeight * 0.9 // past the scroll moment: risen, upright and settled, still pinned
      : laptop.getBoundingClientRect().top + scrollY - Math.max(16, (innerHeight - laptop.offsetHeight) / 2);
    scrollTo({ top: Math.max(0, top), behavior: reduce ? "instant" : "smooth" });
    const focusStep = () => firstAction()?.focus({ preventScroll: true });
    if (laptop.dataset.settled === "true") return void setTimeout(focusStep, reduce ? 0 : 400);
    const watch = new MutationObserver(() => {
      if (laptop.dataset.settled !== "true") return;
      watch.disconnect();
      focusStep();
    });
    watch.observe(laptop, { attributes: true, attributeFilter: ["data-settled"] });
    setTimeout(() => watch.disconnect(), 3000);
  });
  restart.addEventListener("click", () => {
    player.restore();
    // The first song, paused where the tour began: cued, never played, so no sound loads.
    player.cue(startSong, startAt);
    player.show("lyrics");
    // Up next beside it, for the step that picks a song from it.
    player.toggleSide(true);
    tour.start();
    show.focus({ preventScroll: true });
  });
  copy.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(TOUR_LINK);
      copy.textContent = "Copied";
    } catch {
      copy.textContent = "Select and copy it";
    }
    setTimeout(() => (copy.textContent = "Copy"), 1600);
  });

  root.hidden = false;
  tour.start();
  return tour;
}
