// featherplay's window on the page (Window.astro): the replica laid out for the window's own size
// and drawn scaled to fit (src/demo/fit.ts), always wholly inside the viewport. It can be dragged
// by its title bar, maximised and restored (the caption button or a double-click on the title
// bar), snapped to a half of the screen or maximised by pushing it against the screen's side
// or top (as Windows snaps a window: the place it will take shows first), minimised to the
// tray (and back from the bar's tray icon), and closed (the app's own question first). The
// wheel over it scrolls the page until the visitor clicks into it.
//
// Each of the three caption buttons has its own move, as on a real desktop:
// - minimise: the window goes to the tray icon along a curve, shrinking, and the icon takes it;
//   it comes back the same way, landing a little past its size;
// - maximise: the app at its real size fills the screen under the bar, laid out for that size
//   as a real maximised window is. The frame grows while the old layout dissolves into the new
//   (a view transition of the window alone; where the browser has none, the frame glides);
// - close: "Close featherplay" closes it where it stands (and pauses), and the tray icon opens
//   it again as the app opens; "Keep playing in the tray" is minimise.
//
// Every move is a transform of the one element (translate and scale, from its top left): the
// app is laid out again only when the window's size really changes (a resize of the page, the
// story's narrow hold, maximise). Coordinates are the pin's (Stage.astro): the viewport's while the story
// holds the stage, the stage's own otherwise.

import { IDEAL, MIN, clampInto, fitWindow, placeIn, type Box, type Fit } from "./fit";
import { bezier } from "../scripts/story-math";
import { fitFirst } from "../scripts/first-math";
import type { Player } from "./player";
import { hideNote, showNoteAt } from "../scripts/notes";

export type Frame = { w: number; h: number; x: number; y: number; s: number; o: number };
/** The two shapes the steps after the first screen show it in (Stage.astro), where the story
 *  doesn't run: the whole app, or narrow beside the visitor's work. */
export type Shape = "whole" | "narrow";

export type WindowController = {
  readonly el: HTMLElement;
  /** Where the window rests on the first screen, and its layout size. */
  readonly home: Frame;
  /** Where it is drawn now (the pin's coordinates). */
  readonly frame: Frame;
  /** Where it rests by the visitor's own placement (home, dragged, snapped or maximised). */
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
  /** Which of the two shapes the size the page gives it is: narrow up to 860 px of the app's
   *  own (a phone's, src/scripts/first-math.ts narrowFit), whole above. */
  readonly natural: Shape;
  /** Laid out in that shape in its place (null: the size the page gives it), from wherever
   *  the visitor had it. */
  shape(to: Shape | null): void;
};

const MIN_MS = 380;
/** Back from the tray in the time the app opens in (the note by the tray says so). */
const RESTORE_MS = 240;
const MAX_MS = 340;
const CLOSE_MS = 180;
const OPEN_MS = 240;
/** Opened from its shortcut on the desk: out of the icon, unhurried enough to be seen. */
const SHORTCUT_MS = 460;
const EASE_WINDOW = "cubic-bezier(0.32, 0.72, 0, 1)";
/** The way into the tray quickens; the way back out lands. */
const easeIn = bezier(0.5, 0, 0.75, 0.4);
const easeOut = bezier(0.2, 0.8, 0.2, 1);
/** The widest the app is laid out when maximised (a larger screen shows that, scaled up). */
const MAX_LAYOUT_W = 2560;
/** A pointer-sized screen: the window can be dragged and maximised there. */
const DESK = "(min-width: 761px)";
/** The docked bar's height, which a maximised or dragged window stays under. */
const BAR = 52;
/** How far past the screen's side the pointer pushes a dragged window before it offers to snap. */
const PUSH = 40;
/** The gap round the shown place a snap would take. */
const SNAP_GAP = 8;
type Side = "left" | "right";
/** The narrow shape, in the app's px: its real least width, and a phone-like height (the
 *  story's narrow hold is the same). */
const NARROW = { w: 480, h: 660 };
/** The widest layout of the app's own that is still its narrow one (no queue beside). */
const NARROW_MAX = 860;

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
  const gone = slot.querySelector<HTMLElement>("[data-win-gone]");
  const goneBack = gone?.querySelector<HTMLButtonElement>("[data-win-back]");
  const snapEl = slot.querySelector<HTMLElement>("[data-win-snap]");
  const desk = matchMedia(DESK);
  const stage = el.closest<HTMLElement>("[data-stage]");

  let slotOff = { x: 0, y: 0 };
  let home: Frame = { w: 1440, h: 852, x: 0, y: 0, s: 1, o: 1 };
  let drag = { x: 0, y: 0 };
  let maxed = false;
  /** Snapped to a half of the screen. */
  let tile: Side | null = null;
  let minimised = false;
  /** In the tray because it was closed (not minimised): it went where it stood, and opens again. */
  let closed = false;
  let driven: Frame | null = null;
  let shown: Frame = home;
  let size = { w: 0, h: 0 };
  let scaleTimer = 0;
  /** The shape the visitor chose in the steps (null: the size the page gives it). */
  let shape: Shape | null = null;
  let natural: Shape = "whole";
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
  /** The room a snapped window takes: a half of the screen under the bar, or all of it. */
  const snapBox = (to: Side | "max"): Box => {
    const v = viewportBox();
    return to === "max" ? v : { ...v, w: v.w / 2, x: to === "right" ? v.x + v.w / 2 : v.x };
  };
  const rested = (): Frame => {
    if (maxed || tile) {
      // The app at its real size, laid out for the whole screen under the bar (or the half it
      // is snapped to); scaled only where that is under its least size or over the widest it
      // is laid out.
      const v = snapBox(tile ?? "max");
      const s = Math.min(Math.max(1, v.w / MAX_LAYOUT_W), v.w / MIN.w, v.h / MIN.h);
      return { w: v.w / s, h: v.h / s, s, o: 1, x: v.x, y: v.y };
    }
    return { ...home, x: home.x + drag.x, y: home.y + drag.y };
  };
  /** The bar's part the window goes into: the tray icon, or the capsule's song (whichever this
   *  state shows). */
  const trayTarget = () =>
    [...document.querySelectorAll<HTMLElement>("[data-tray-target]")].find((t) => t.getClientRects().length && getComputedStyle(t).visibility !== "hidden");
  /** The desk's own tray (where the bar shows no tray icon, showGone marks it): its lit icon. */
  const goneTray = gone?.querySelector<HTMLElement>("[data-gone-tray]");
  const ownTray = () => (gone?.hasAttribute("data-own-tray") ? (goneTray?.querySelector<HTMLElement>(".tb.lit") ?? null) : null);
  const trayPoint = () => {
    const p = pinRect();
    const icon = ownTray() ?? trayTarget();
    if (icon) {
      const r = icon.getBoundingClientRect();
      return { x: r.left + r.width / 2 - p.left, y: r.top + r.height / 2 - p.top };
    }
    return { x: document.documentElement.clientWidth - 120 - p.left, y: 26 - p.top };
  };
  const trayFrame = (from: Frame): Frame => {
    const t = trayPoint();
    // As small as the icon it goes into.
    const s = Math.min(from.s * 0.2, 40 / from.w);
    return { ...from, s, o: 0, x: t.x - (from.w * s) / 2, y: t.y - (from.h * s) / 2 };
  };
  /** Closed where it stands: a little smaller about its middle, and gone. */
  const closedFrame = (from: Frame): Frame => ({ ...from, s: from.s * 0.95, o: 0, x: from.x + from.w * from.s * 0.025, y: from.y + from.h * from.s * 0.025 });

  /** Ends the window's own moves (the ones scripted here). Not the stylesheet's transitions:
   *  the delay that keeps a minimised window in sight until it has reached the tray is one,
   *  and ending it hid the window the moment minimise was pressed. */
  const stop = () => {
    for (const a of el.getAnimations()) if (!(a instanceof CSSTransition)) a.cancel();
  };

  // ---- Drawing: one transform, the size only when it changes ----
  const draw = (f: Frame, scaleNow = false) => {
    if (f.w !== size.w || f.h !== size.h) {
      el.style.width = `${f.w}px`;
      el.style.height = `${f.h}px`;
      size = { w: f.w, h: f.h };
    }
    el.style.transform = tf(f, slotOff);
    el.style.opacity = f.o >= 1 ? "" : f.o.toFixed(3);
    // Text selected in it is drawn where it now is (src/scripts/selection.ts).
    dispatchEvent(new Event("placechange"));
    // Away from home (dragged, maximised, in the tray, the story's): the first screen's notes
    // about it step aside.
    stage?.toggleAttribute("data-win-away", maxed || !!tile || minimised || !!driven || !!(drag.x || drag.y));
    // The scale for the page-size caption targets and corners: written once the window rests
    // (it changes their layout), not on every frame of a move, and not for a frame it is gone
    // in (the tray's: its corners would round to a pill on the way there).
    clearTimeout(scaleTimer);
    const s = f.s.toFixed(4);
    if (f.o >= 1 && el.style.getPropertyValue("--s") !== s) {
      if (scaleNow) el.style.setProperty("--s", s);
      else scaleTimer = window.setTimeout(() => el.style.setProperty("--s", s), 120);
    }
    shown = f;
    // The desk under it, where this uncovers it.
    showGone();
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
    stop();
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
  /** To the tray and back along a curve: across first and then up into the icon (back: out of
   *  it and then across), its size and place sampled into keyframes, since the two axes ease
   *  differently. It fades only by the icon; coming back it lands a little past its size. */
  const fly = (rest: Frame, toTray: boolean) => {
    const tray = trayFrame(rest);
    const from = current();
    stop();
    draw(toTray ? tray : rest);
    if (reduced()) return;
    // From wherever it is drawn now (a press in the middle of the other way turns it round).
    const a = toTray || from.o > 0.02 ? from : tray;
    const b = toTray ? tray : rest;
    const mid = (f: Frame) => ({ x: f.x + (f.w * f.s) / 2, y: f.y + (f.h * f.s) / 2 });
    const [ca, cb] = [mid(a), mid(b)];
    const N = 18;
    const frames: Keyframe[] = [];
    for (let i = 0; i <= N; i++) {
      const t = i / N;
      const e = toTray ? easeIn(t) : easeOut(t);
      // The axis that leads: across on the way in, up and out of the icon on the way back.
      const lead = 1 - (1 - e) * (1 - e);
      const lag = e * e;
      const sc = a.s + (b.s - a.s) * (toTray ? e : e + 0.05 * Math.sin(Math.PI * e) * e);
      const cx = ca.x + (cb.x - ca.x) * (toTray ? lead : lag);
      const cy = ca.y + (cb.y - ca.y) * (toTray ? lag : lead);
      const o = toTray ? a.o * (1 - Math.max(0, (e - 0.6) / 0.4) ** 2) : Math.max(a.o, Math.min(1, e / 0.3));
      frames.push({ transform: tf({ ...b, s: sc, x: cx - (b.w * sc) / 2, y: cy - (b.h * sc) / 2 }, slotOff), opacity: o });
    }
    el.animate(frames, { duration: toTray ? MIN_MS : RESTORE_MS, easing: "linear" });
  };
  /** A change of the window's own size (maximise, restore): the app is laid out once for the new
   *  size, and the frame grows from the old one to the new while the old layout dissolves into
   *  it (a view transition of this one element: Window.astro has its rules). Elsewhere the new
   *  layout's frame glides from the old frame's place, stretched to it at first. */
  let morphs = 0;
  const morph = (ms: number) => {
    stop();
    const from = shown;
    // Drawn as the window's state is when the browser asks (a second press may have come
    // before the first one's transition began: both then draw the same, latest, state).
    const go = () => {
      if (driven || minimised) return;
      stop();
      draw(rested(), true);
    };
    if (reduced() || ms <= 0) return go();
    const doc = document as Document & { startViewTransition?: (fn: () => void) => { finished: Promise<void> } };
    // Only in Chromium (told by an API only it has): Firefox and WebKit have view transitions
    // too, but drew this one as a cut with a stray frame over it when checked (November 2026),
    // and the plain glide below is better than that.
    if (doc.startViewTransition && "userAgentData" in navigator) {
      const root = document.documentElement;
      const mine = ++morphs;
      el.style.setProperty("view-transition-name", "fp-window");
      root.toggleAttribute("data-win-morph", true);
      // Only the latest one clears up: an earlier one, cut short by it, ends while it runs.
      const done = () => {
        if (mine !== morphs) return;
        el.style.removeProperty("view-transition-name");
        root.removeAttribute("data-win-morph");
      };
      doc.startViewTransition(go).finished.then(done, done);
      return;
    }
    go();
    const to = shown;
    el.animate(
      [
        { transform: `translate(${(from.x - slotOff.x).toFixed(2)}px, ${(from.y - slotOff.y).toFixed(2)}px) scale(${((from.w * from.s) / to.w).toFixed(5)}, ${((from.h * from.s) / to.h).toFixed(5)})` },
        { transform: tf(to, slotOff) },
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
      stop();
      draw(f);
    }
  };

  /** Measures the room and places the window at rest; with `ms`, a change of shape it morphs
   *  into (as maximise does). */
  function layout(ms = 0) {
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
    let fit: Fit = slot.dataset.fitW
      ? fitWindow(box, { want: { w: Number(slot.dataset.fitW), h: Number(slot.dataset.fitH) } })
      : slot.dataset.fit === "first"
        ? fitFirst(box)
        : fitWindow(box);
    natural = fit.w <= NARROW_MAX ? "narrow" : "whole";
    // The other shape, drawn no larger than the page's own (the same window, another shape),
    // and on a phone in the middle of the room it leaves.
    const other = !!shape && shape !== natural;
    if (other) {
      const f = fitWindow(box, { want: shape === "narrow" ? NARROW : IDEAL });
      fit = { ...f, s: Math.min(f.s, fit.s) };
    }
    const at = placeIn(box, fit, desk.matches ? "end" : "center", other && !desk.matches ? "center" : "start");
    home = { w: fit.w, h: fit.h, s: fit.s, o: 1, ...at };
    // A dragged window stays where it was put, as far as the new viewport allows.
    if (drag.x || drag.y) {
      const v = viewportBox();
      const at2 = clampInto({ x: home.x + drag.x, y: home.y + drag.y, w: fit.w * fit.s, h: fit.h * fit.s }, v);
      drag = { x: at2.x - home.x, y: at2.y - home.y };
    }
    if (!desk.matches) {
      drag = { x: 0, y: 0 };
      markTile(null);
      setMax(false, 0);
    }
    if (ms && !minimised) morph(ms);
    else settle();
    showGone(true);
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
  function markTile(side: Side | null) {
    tile = side;
    if (side) el.dataset.tile = side;
    else delete el.dataset.tile;
  }
  function setMax(on: boolean, ms = MAX_MS) {
    if ((on && !desk.matches) || driven || on === maxed) return;
    // The note by the tray (how fast it opened) was placed clear of the window as it stood.
    opensOff();
    markMax(on);
    if (on) {
      drag = { x: 0, y: 0 };
      markTile(null);
    }
    if (ms > 0 && !minimised) morph(ms);
    else settle();
  }
  /** Snapped to a half of the screen: laid out for it, as a maximised window is for the whole. */
  const snapTo = (side: Side) => {
    if (driven || !desk.matches) return;
    opensOff();
    markTile(side);
    drag = { x: 0, y: 0 };
    morph(MAX_MS);
  };
  // Too small a screen to maximise on: the button says it can't.
  const deskOnly = () => maxButton?.setAttribute("aria-disabled", String(!desk.matches));

  // ---- Minimise to the tray, and back ----
  // "opens in 0.24 s": by the tray icon as it lights again, each time featherplay is opened
  // after it was closed (not a window shown again from the tray: that isn't the app opening;
  // nor when the story brings it back), for a few seconds or until the page moves, or the
  // window does (maximised, dragged, minimised again: it would lie over the note).
  const opensNote = document.querySelector<HTMLElement>("[data-note='opens']");
  let opensTimer = 0;
  const opensOff = () => {
    clearTimeout(opensTimer);
    if (opensNote) hideNote(opensNote);
    removeEventListener("scroll", opensOff);
  };
  const showOpens = () => {
    if (!opensNote || driven) return;
    // Written anew each time.
    opensOff();
    opensNote.classList.remove("written");
    const t = trayTarget()?.getBoundingClientRect();
    if (!t) return;
    const p = pinRect();
    const f = rested();
    const win = { left: p.left + f.x, top: p.top + f.y, right: p.left + f.x + f.w * f.s, bottom: p.top + f.y + f.h * f.s };
    if (!showNoteAt(opensNote, t.left + t.width / 2 - 6, t.bottom + 2, [win])) return;
    opensTimer = window.setTimeout(opensOff, 4500);
    addEventListener("scroll", opensOff, { passive: true });
  };

  /** Closed, not in the tray: the bar says so. Its tray icon is unlit and offers to open the
   *  app (Bar.astro; its pop-up of the player stays shut: bar.ts), and the song's flyout
   *  offers the same. */
  const markShut = () => {
    const shut = minimised && closed;
    document.documentElement.toggleAttribute("data-window-closed", shut);
    const trayIcon = document.querySelector<HTMLElement>("[data-site-bar] [data-tray]");
    if (trayIcon) {
      trayIcon.dataset.tip = shut ? "featherplay is closed: open it" : "featherplay: show or hide the window";
      trayIcon.setAttribute("aria-label", shut ? "featherplay is closed: open it" : "featherplay in the tray: show or hide the window");
    }
    const show = document.querySelector<HTMLElement>("[data-site-bar] [data-show-window] span");
    if (show) show.textContent = shut ? "Open featherplay" : "Show featherplay’s window";
  };
  // A song started from somewhere else on the page while it is closed: featherplay is running
  // again, in the tray (the desk's note and the tray icon say so).
  player.subscribe((s) => {
    if (!(minimised && closed && s.playing)) return;
    closed = false;
    markShut();
    showGone();
  });
  const setMinimised = (on: boolean) => {
    if (on === minimised) return;
    minimised = on;
    if (!on && closed) showOpens();
    else opensOff();
    markShut();
    const hadFocus = el.contains(document.activeElement);
    el.toggleAttribute("data-minimised", on);
    document.documentElement.toggleAttribute("data-window-minimised", on);
    el.inert = on;
    // From where it rests, or from where the story holds it (the story leaves a minimised
    // window alone, and draws it again as the page moves on).
    const rest = driven ?? rested();
    const deskAt = goneAt;
    showGone();
    if (on && closed) glide(closedFrame(current()), CLOSE_MS);
    else if (on) {
      fly(rest, true);
      // The icon takes it (Bar.astro: data-caught).
      const icon = ownTray() ? null : trayTarget();
      if (icon && !reduced()) {
        icon.toggleAttribute("data-caught", true);
        setTimeout(() => icon.removeAttribute("data-caught"), MIN_MS + 600);
      }
    } else if (byShortcut) openFrom(rest, deskAt ?? rest);
    else if (closed) open(rest);
    else fly(rest, false);
    byShortcut = false;
    if (!on) closed = false;
    // Put away, it forgets where the visitor had it (dragged, snapped, maximised): it goes
    // from there, and comes back to its place on the first screen. (A real window would
    // remember; on a page, coming back over the words is the worse surprise.)
    else if (!driven) {
      drag = { x: 0, y: 0 };
      markTile(null);
      if (maxed) markMax(false);
    }
    // The keys go where the window went: the tray icon, or the capsule's song.
    if (on && hadFocus) (ownTray() ? goneTray : trayTarget())?.focus({ preventScroll: true });
  };
  /** The desk in the window's place on the first screen (its home), shown while the window
   *  isn't on it: sent to the tray or closed, or moved off it by the visitor (dragged, snapped:
   *  the desk lies under the window, and shows as far as the window uncovers it; never over
   *  the page's words, wherever the window went). The app's shortcut in the middle and the
   *  maker's note over it for the way the window went: closed (and paused), in the tray,
   *  playing on, or only moved (the shortcut then takes it home). The note is written each
   *  time it changes (the pen's reveal starts over); a moved window's, once the window has
   *  uncovered it. */
  type Away = "closed" | "tray" | "moved";
  const goneHand = gone?.querySelector<HTMLElement>("[data-gone-hand]");
  const gonePick = gone?.querySelector<HTMLElement>("[data-gone-pick]");
  let goneWas: Away | null = null;
  /** The desk's box while it shows. */
  let goneAt: Frame | null = null;
  let goneWritten = false;
  const writeGone = (which: Away, delay: number) => {
    goneWritten = true;
    setTimeout(() => goneWas === which && goneHand?.classList.add("written"), delay);
    // "click one" close behind it, as the first note's words end (once: it stays while the
    // note over the shortcut changes).
    setTimeout(() => goneWas && gonePick?.classList.add("written"), delay + 340);
  };
  function showGone(place = false) {
    if (!gone || !goneBack) return;
    const which: Away | null = minimised ? (closed ? "closed" : "tray") : !driven && !maxed && (tile || drag.x || drag.y) ? "moved" : null;
    if (which !== goneWas) {
      goneWas = which;
      if (which) gone.dataset.on = which;
      else delete gone.dataset.on;
      goneBack.tabIndex = which ? 0 : -1;
      // Its own tray on a phone, where the bar shows none, in the steps' switch (Stage.astro);
      // kept (unseen) as the window comes back, which it comes out of.
      if (which) {
        const barTray = document.querySelector<HTMLElement>("[data-site-bar] [data-tray]");
        const barHas = !!barTray && barTray.getClientRects().length > 0 && getComputedStyle(barTray).visibility !== "hidden";
        gone.toggleAttribute("data-own-tray", which === "tray" && !barHas && !desk.matches && document.documentElement.hasAttribute("data-shapes"));
      }
      if (goneTray) goneTray.tabIndex = which === "tray" && gone.hasAttribute("data-own-tray") ? 0 : -1;
      if (!which) {
        goneAt = null;
        goneHand?.classList.remove("written");
        gonePick?.classList.remove("written");
        sayOff();
        return;
      }
      goneBack.setAttribute("aria-label", which === "closed" ? "Open featherplay" : which === "tray" ? "Show featherplay’s window" : "Move featherplay’s window back");
      if (goneHand) {
        goneHand.querySelector("[data-gone-words]")!.textContent = goneHand.dataset[which === "closed" ? "sayClosed" : which === "tray" ? "sayTray" : "sayMoved"]!;
        goneHand.classList.remove("written");
        goneWritten = false;
        // Once the window has gone and the shortcut is in (Window.astro's delay).
        if (which !== "moved") writeGone(which, 420);
      }
      place = true;
    }
    if (!which) return;
    if (place) {
      // The box is the window's own at home (the story's window: where the story holds it):
      // its things are set from its middle.
      // (With its own tray, the room the window had, whatever its shape: the shortcut, its note
      // and the tray need the height a whole window on a phone doesn't have.)
      const room = gone.hasAttribute("data-own-tray") ? slot.getBoundingClientRect() : null;
      const rest: Frame = room ? { x: slotOff.x, y: slotOff.y, w: room.width, h: room.height, s: 1, o: 1 } : (minimised && driven) || home;
      goneAt = rest;
      const [bw, bh] = [rest.w * rest.s, rest.h * rest.s];
      gone.style.width = `${bw.toFixed(1)}px`;
      gone.style.height = `${bh.toFixed(1)}px`;
      gone.style.translate = `${(rest.x - slotOff.x).toFixed(1)}px ${(rest.y - slotOff.y).toFixed(1)}px`;
      // The others only where the window had the room for them (they lie some 18.5 rem either
      // side of its middle, 13 rem above and 11 below).
      gone.toggleAttribute("data-slim", bw < 620 || bh < 430);
    }
    if (which === "moved" && !goneWritten && goneAt) {
      // The note's place (over the shortcut in the middle), clear of the window as drawn now.
      const [x, y] = [goneAt.x + (goneAt.w * goneAt.s) / 2, goneAt.y + (goneAt.h * goneAt.s) / 2 - 40];
      const f = shown;
      if (x < f.x || x > f.x + f.w * f.s || y < f.y || y > f.y + f.h * f.s) writeGone("moved", 120);
    }
  }
  /** Back to its place on the first screen from wherever the visitor put it. */
  const goHome = () => {
    if (driven || minimised) return;
    const sized = maxed || !!tile;
    drag = { x: 0, y: 0 };
    markTile(null);
    if (maxed) markMax(false);
    if (sized) morph(MAX_MS);
    else glide(rested(), 320);
  };
  /** The window is coming back by its shortcut on the desk (it opens out of that). */
  let byShortcut = false;
  goneTray?.addEventListener("click", () => {
    player.restore();
    replica.querySelector<HTMLElement>('[data-act="minimise"]')?.focus({ preventScroll: true });
  });
  goneBack?.addEventListener("click", () => {
    if (minimised) {
      byShortcut = true;
      player.restore();
    } else goHome();
    // The keys go back into the window with it.
    replica.querySelector<HTMLElement>('[data-act="minimise"]')?.focus({ preventScroll: true });
  });
  // The desk's other things: pressed, each says its line in a small pane under itself (kept
  // inside the box), for a few seconds, or until a press anywhere else, or Escape.
  const goneSay = gone?.querySelector<HTMLElement>("[data-desk-say]");
  let sayTimer = 0;
  function sayOff() {
    clearTimeout(sayTimer);
    goneSay?.removeAttribute("data-on");
  }
  addEventListener("pointerdown", (e) => {
    if (goneSay?.hasAttribute("data-on") && !(e.target as Element | null)?.closest?.("[data-say]")) sayOff();
  });
  addEventListener("keydown", (e) => {
    if (e.key === "Escape" && goneSay?.hasAttribute("data-on")) sayOff();
  });
  gone?.addEventListener("click", (e) => {
    const b = (e.target as Element).closest<HTMLElement>("[data-say]");
    if (!b || !goneSay || !gone) return;
    goneSay.textContent = b.dataset.say!;
    const w = goneSay.offsetWidth;
    const left = Math.max(0, Math.min(b.offsetLeft + b.offsetWidth / 2 - w / 2 - b.offsetWidth / 2, gone.clientWidth - w));
    // (The icon is placed by its middle: its offsets are its box's before that shift.)
    goneSay.style.translate = `${Math.round(left)}px ${Math.round(b.offsetTop + b.offsetHeight / 2 + 4)}px`;
    goneSay.toggleAttribute("data-on", true);
    clearTimeout(sayTimer);
    sayTimer = window.setTimeout(sayOff, 4200);
  });
  /** Opened by its shortcut: out of the icon in the middle of the desk's box, growing to its
   *  size and place, in sight from the first moment. */
  const openFrom = (to: Frame, from0: Frame) => {
    stop();
    draw(to);
    if (reduced()) return;
    const k = 0.18;
    const c = { x: from0.x + (from0.w * from0.s) / 2, y: from0.y + (from0.h * from0.s) / 2 };
    const from = { ...to, s: to.s * k, x: c.x - (to.w * to.s * k) / 2, y: c.y - (to.h * to.s * k) / 2 };
    el.animate(
      [
        { transform: tf(from, slotOff), opacity: 0 },
        { opacity: 1, offset: 0.25 },
        { transform: tf(to, slotOff), opacity: 1 },
      ],
      { duration: SHORTCUT_MS, easing: EASE_WINDOW },
    );
  };
  /** Opened again after a close: as the app opens, from 0.9 by its top right (the tray's side). */
  const open = (to: Frame) => {
    stop();
    draw(to);
    if (reduced()) return;
    const from = { ...to, s: to.s * 0.9, x: to.x + to.w * to.s * 0.1 };
    el.animate(
      [
        { transform: tf(from, slotOff), opacity: 0 },
        { transform: tf(to, slotOff), opacity: 1 },
      ],
      { duration: OPEN_MS, easing: EASE_WINDOW },
    );
  };
  player.subscribe((s) => setMinimised(s.minimised));

  // ---- Close: the app asks, then what was chosen; remembered for the visit if asked to ----
  let remembered: "tray" | "close" | null = null;
  const closeAs = (choice: "tray" | "close") => {
    if (choice === "close") {
      if (player.state.playing) player.pause();
      closed = true;
    }
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
  // Pushed on against the screen's side it offers to snap to that half, against the top to
  // maximise (the place shows under it; let go there and it takes it), as on Windows.
  let dragFrom: { px: number; py: number; x: number; y: number; held: boolean } | null = null;
  let aimed: Side | "max" | null = null;
  const aim = (to: Side | "max" | null) => {
    if (to === aimed) return;
    aimed = to;
    if (!snapEl) return;
    if (to) {
      const b = snapBox(to);
      snapEl.style.width = `${(b.w - 2 * SNAP_GAP).toFixed(1)}px`;
      snapEl.style.height = `${(b.h - 2 * SNAP_GAP).toFixed(1)}px`;
      snapEl.style.translate = `${(b.x + SNAP_GAP - slotOff.x).toFixed(1)}px ${(b.y + SNAP_GAP - slotOff.y).toFixed(1)}px`;
      snapEl.dataset.on = to;
    } else delete snapEl.dataset.on;
  };
  titlebar.addEventListener("pointerdown", (e) => {
    if (e.button !== 0 || !desk.matches || minimised || driven || (e.target as Element).closest("button")) return;
    e.preventDefault();
    opensOff();
    titlebar.setPointerCapture(e.pointerId);
    stop();
    // A maximised or snapped window stays as it is until the pointer moves (a press alone,
    // or the first of a double-click, isn't a drag).
    dragFrom = { px: e.clientX, py: e.clientY, x: drag.x, y: drag.y, held: maxed || !!tile };
    el.setAttribute("data-dragging", "");
  });
  let moveFrame = 0;
  titlebar.addEventListener("pointermove", (e) => {
    // The story took the window over (the page scrolled under the drag): it draws it now.
    if (!dragFrom || driven) return;
    if (dragFrom.held) {
      if (Math.hypot(e.clientX - dragFrom.px, e.clientY - dragFrom.py) < 4) return;
      // It comes back to its size under the pointer, as Windows does.
      const f = shown;
      const left = pinRect().left;
      const k = (e.clientX - left - f.x) / (f.w * f.s);
      markMax(false);
      markTile(null);
      drag = { x: e.clientX - left - k * home.w * home.s - home.x, y: f.y - home.y };
      draw(rested(), true);
      dragFrom = { px: e.clientX, py: e.clientY, x: drag.x, y: drag.y, held: false };
    }
    const want = { x: dragFrom.x + e.clientX - dragFrom.px, y: dragFrom.y + e.clientY - dragFrom.py };
    if (moveFrame) cancelAnimationFrame(moveFrame);
    moveFrame = requestAnimationFrame(() => {
      moveFrame = 0;
      const w = home.w * home.s;
      const h = home.h * home.s;
      const raw = { x: home.x + want.x, y: home.y + want.y };
      const at = clampInto({ ...raw, w, h }, viewportBox());
      drag = { x: at.x - home.x, y: at.y - home.y };
      draw(rested());
      aim(at.y - raw.y > PUSH ? "max" : at.x - raw.x > PUSH ? "left" : raw.x - at.x > PUSH ? "right" : null);
    });
  });
  /** The drag ends: let go, the window takes the place it was offered; taken away (by the
   *  story, a cancelled pointer), it stays where it is. */
  function endDrag(letGo = false) {
    if (!dragFrom) return;
    if (moveFrame) cancelAnimationFrame(moveFrame);
    moveFrame = 0;
    dragFrom = null;
    el.removeAttribute("data-dragging");
    const to = aimed;
    aim(null);
    if (!letGo || !to) return;
    if (to === "max") setMax(true);
    else snapTo(to);
  }
  titlebar.addEventListener("pointerup", () => endDrag(true));
  titlebar.addEventListener("pointercancel", () => endDrag());

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
  // On the first screen the window is placed first by the composition (src/scripts/first.ts),
  // once that is set; until then nothing here measures it (it would be placed twice at load,
  // the first time at a size the composition then changes). If that never comes, the slot's
  // own observation places it after a moment.
  let waiting = !!slot.closest(".first");
  if (waiting)
    setTimeout(() => {
      waiting = false;
      relayout();
    }, 1500);
  const relayout = () => {
    if (waiting || resizeFrame) return;
    resizeFrame = requestAnimationFrame(() => {
      resizeFrame = 0;
      layout();
    });
  };
  new ResizeObserver(relayout).observe(slot);
  addEventListener("resize", relayout);
  // A width query needs the page's layout to answer (whether a scrollbar takes room), and
  // listening to one asks it: both on the first frame, with the first measure.
  requestAnimationFrame(() => {
    desk.addEventListener("change", relayout);
    desk.addEventListener("change", deskOnly);
    deskOnly();
  });
  // Not measured while the page's scripts are starting: read now, it would force the page's
  // first layout inside this script's task (a 50 ms task at load). The first screen places it
  // on its first frame (src/scripts/first.ts); without that, the slot's first observation does.
  // Hidden until then (Window.astro), so nothing shows unplaced.
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
    layout() {
      waiting = false;
      layout();
    },
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
        stop();
        draw(f);
      } else settle();
    },
    forget() {
      drag = { x: 0, y: 0 };
      if (maxed) markMax(false);
      markTile(null);
    },
    trayPoint,
    onLayout(fn) {
      layoutFns.push(fn);
    },
    get natural() {
      return natural;
    },
    shape(to) {
      if (to === shape || driven) return;
      shape = to;
      opensOff();
      drag = { x: 0, y: 0 };
      markTile(null);
      if (maxed) markMax(false);
      waiting = false;
      layout(MAX_MS);
    },
    arrive(delay) {
      if (reduced() || driven || minimised || maxed || tile) return;
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
