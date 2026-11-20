// The story's change of shape (story.ts): the window resizes from one hold's layout to the
// next's, as a window does, without the app being laid out again on the way (its lyrics would
// re-wrap on every frame, and the page would count every frame as a layout shift). The app is
// laid out once, in the shape it is going to; a still copy of the shape it leaves lies under it
// in the window's ground (Stage.astro's [data-ghost]), which is the frame: it takes the window's
// size from one rect to the other. What both shapes have stays on screen the whole way: the
// title bar (its strip in the frame, the caption buttons kept to its right), the player bar's
// capsule (drawn in the copy, as wide as the frame), the cover travelling from its place in one
// shape to its place in the other. The rest swaps: the old shape's words and the bar's old
// buttons fade out, then the new ones fade in, never both at once. Everything that moves on
// the way moves by transform; only the frame's and the capsule's own sizes change.

import type { Frame, WindowController } from "../demo/window";
import { followOffset } from "../demo/follow";
import { seg } from "./story-math";

/** A hold's window: its layout size (the app's px) and where it is drawn (the pin's px). */
export type Hold = { w: number; h: number; x: number; y: number; s: number };

type Box = { x: number; y: number; w: number; h: number };
/** One shape's player bar: its middle kept to the frame's middle or its right, its end to the
 *  right, the whole bar to the frame's foot. */
type BarParts = { bar: HTMLElement; mid: HTMLElement[]; end: HTMLElement | null; centred: boolean; kids: HTMLElement[] };
type Run = {
  key: string;
  copy: HTMLElement;
  /** In the live app while it moves: the capsule, and the old shape's bar (its buttons). */
  pill: HTMLElement | null;
  oldBarEl: HTMLElement | null;
  /** The live app's own grounds, hidden while the frame paints the one ground for both. */
  grounds: HTMLElement[];
  /** The copy's parts that fade out, and the live app's that fade in. */
  outFade: HTMLElement[];
  inFade: HTMLElement[];
  oldBar: BarParts | null;
  newBar: BarParts | null;
  caption: HTMLElement | null;
  cover: HTMLElement | null;
  /** The cover's way: from the old shape's place to its own, in the new shape's px. */
  from: Box | null;
  inset: number;
};

const smooth = (t: number) => t * t * (3 - 2 * t);
const lerp = (a: number, b: number, t: number) => a + (b - a) * t;
/** The window's corner on screen (Window.astro .win-body). */
const CORNER = 8;
const px = (v: number) => `${v.toFixed(2)}px`;

/** Switches the window's morph state at once: the parts it changes the look of (the bar's ground
 *  fades with the song's colour, --t-colour) would otherwise fade there, the bar seen through. */
function switchMorph(el: HTMLElement, on: boolean) {
  const parts = [...el.querySelectorAll<HTMLElement>(".replica, .app, .bar")];
  for (const p of parts) p.style.transition = "none";
  el.toggleAttribute("data-morph", on);
  if (parts[0]) getComputedStyle(parts[parts.length - 1]).backgroundColor;
  for (const p of parts) p.style.removeProperty("transition");
}

function barParts(root: ParentNode): BarParts | null {
  const bar = root.querySelector<HTMLElement>(".bar");
  return bar ? barPartsOf(bar) : null;
}
function barPartsOf(bar: HTMLElement): BarParts {
  const mid = bar.querySelector<HTMLElement>(":scope > .bar-mid");
  const end = bar.querySelector<HTMLElement>(":scope > .bar-end");
  // The whole app's transport sits in the bar's middle; the narrow one's by its right end.
  const centred = !!mid && Math.abs(mid.offsetLeft + mid.offsetWidth / 2 - bar.offsetWidth / 2) < bar.offsetWidth * 0.1;
  const kids = [...bar.children].filter((c): c is HTMLElement => c instanceof HTMLElement);
  // The middle's parts move, not the middle: a transform on it would make it the box its
  // absolutely placed parts are placed in (the narrow bar's seek line, along the bar's foot).
  const parts = mid ? [...mid.children].filter((c): c is HTMLElement => c instanceof HTMLElement && getComputedStyle(c).position !== "absolute") : [];
  return { bar, mid: parts, end, centred, kids };
}

/** Places a shape's bar in a frame `dw` wider and `dh` taller than its layout. */
function placeBar(p: BarParts | null, dw: number, dh: number) {
  if (!p) return;
  p.bar.style.translate = `0 ${px(dh)}`;
  if (p.end) p.end.style.translate = px(dw);
  for (const m of p.mid) m.style.translate = px(p.centred ? dw / 2 : dw);
}

export function morpher(win: WindowController, shell: HTMLElement) {
  const el = win.el;
  let run: Run | null = null;

  /** A box inside the window, in its own px (whatever it is drawn at now). */
  const local = (node: Element): Box | null => {
    if (!node.getClientRects().length) return null;
    const w = el.getBoundingClientRect();
    const k = w.width / el.offsetWidth;
    const r = node.getBoundingClientRect();
    if (!r.width) return null;
    return { x: (r.left - w.left) / k, y: (r.top - w.top) / k, w: r.width / k, h: r.height / k };
  };
  /** Its own fades (the lyrics' panel's, the lines') would trail the move's: off for it. */
  const still = (list: HTMLElement[]) => {
    for (const n of list) n.style.transition = "none";
    return list;
  };

  /** Starts a change of shape from `a` to `b`: the copy of `a`, then the app laid out as `b`. */
  function begin(key: string, a: Hold, b: Hold) {
    // Laid out as the shape it leaves, to copy and to measure.
    const relaid = el.offsetWidth !== a.w || el.offsetHeight !== a.h;
    win.drive({ ...a, o: 1 });
    const body = el.querySelector<HTMLElement>("[data-win-body]");
    const replica = body?.querySelector<HTMLElement>(".replica");
    if (!body || !replica) return;
    const isNow = /^(now|lyrics)$/.test(replica.dataset.view ?? "");
    const coverA = isNow ? replica.querySelector(".np-cover") : null;
    const fromBox = coverA ? local(coverA) : null;
    // The window's ground, frame, title bar and bar, as the app paints them now. The ground is
    // the frame's alone while it moves (one ground under both shapes, never a seam where the new
    // layout ends): Now playing's field, which runs from the window's top (the title bar is a
    // row of controls over it), else the app's own ground.
    const wash = replica.querySelector<HTMLElement>(".stage-wash");
    const washOn = !!wash && !!wash.getClientRects().length;
    shell.style.background = getComputedStyle(replica).backgroundColor;
    shell.style.boxShadow = getComputedStyle(body).boxShadow;
    const titleEl = replica.querySelector<HTMLElement>(".titlebar");
    const liveBarA = replica.querySelector<HTMLElement>(".bar");
    const barStyle = liveBarA ? getComputedStyle(liveBarA) : null;
    const pillLook = barStyle ? { background: barStyle.backgroundColor, boxShadow: barStyle.boxShadow, borderRadius: barStyle.borderRadius } : null;
    const inset = barStyle ? parseFloat(barStyle.left) || 10 : 10;
    // The old bar's columns as they are (replica.css asks them of the window's width, which
    // will be the new shape's once it lies in it).
    const barGrid =
      barStyle && liveBarA
        ? { gridTemplateColumns: barStyle.gridTemplateColumns, columnGap: barStyle.columnGap, paddingBottom: barStyle.paddingBottom, width: px(liveBarA.offsetWidth) }
        : null;

    // Laid out again just now, its lyrics haven't followed their line yet (src/demo/follow.ts
    // does on the next frame): the copy's are put where they will be.
    const panel0 = replica.querySelector<HTMLElement>("[data-lyrics]");
    const list0 = panel0?.querySelector<HTMLElement>("[data-lines]");
    let lyricY: number | null = null;
    if (relaid && panel0 && list0 && panel0.getClientRects().length) {
      const li = list0.querySelector("[aria-current='true']")?.closest<HTMLElement>("[data-lines] > *") ?? null;
      const top = li ? (li.offsetParent === list0 ? li.offsetTop : li.offsetTop - list0.offsetTop) : 0;
      lyricY = followOffset(li ? { top, height: li.offsetHeight } : null, panel0.clientHeight, list0.offsetHeight);
    }

    // The still copy: the old shape's layout at its own size, on the frame's ground; its title
    // bar, its grounds and its cover left to the frame and the live app; its bar taken out (it
    // goes into the live app, over the capsule).
    const copy = body.cloneNode(true) as HTMLElement;
    for (const n of copy.querySelectorAll("[id]")) n.removeAttribute("id");
    for (const n of copy.querySelectorAll("audio, video")) n.remove();
    for (const n of copy.querySelectorAll("[popover]")) n.removeAttribute("popover");
    copy.removeAttribute("data-win-body");
    copy.classList.add("morph-copy");
    Object.assign(copy.style, { inset: "auto", left: "0", top: "0", width: px(a.w), height: px(a.h), boxShadow: "none", borderRadius: "0" });
    const copyList = copy.querySelector<HTMLElement>("[data-lines]");
    if (copyList && lyricY != null) Object.assign(copyList.style, { transition: "none", transform: `translateY(${-lyricY}px)` });
    for (const n of copy.querySelectorAll<HTMLElement>([".titlebar", ".stage-wash", ...(fromBox ? [".np-cover"] : [])].join(",")))
      n.style.visibility = "hidden";
    const copyApp = copy.querySelector<HTMLElement>(".app");
    for (const n of [copy.querySelector<HTMLElement>(".replica"), copyApp]) if (n) n.style.background = "none";
    const oldBarEl = copy.querySelector<HTMLElement>(".bar");
    oldBarEl?.remove();
    // Now playing's field, under the copy, as tall as the frame.
    const layers: HTMLElement[] = [];
    if (washOn) {
      const g = document.createElement("div");
      Object.assign(g.style, { position: "absolute", inset: "0", backgroundImage: getComputedStyle(wash!).backgroundImage });
      layers.push(g);
    }
    shell.replaceChildren(...layers, copy);
    const outFade = still([...(copyApp?.children ?? [])].filter((c): c is HTMLElement => c instanceof HTMLElement));

    // The app laid out once in the new shape.
    win.drive({ ...b, x: a.x, y: a.y, s: a.s, o: 1 });
    const app = replica.querySelector<HTMLElement>(".app");
    const coverB = fromBox ? replica.querySelector<HTMLElement>(".np-cover") : null;
    const toBox = coverB ? local(coverB) : null;
    const travel = !!(fromBox && toBox && app?.contains(coverB));
    // What fades in: the app's parts but the bar, and on the cover's way down to it, everything
    // beside it at each step (the cover itself stays).
    const inFade: HTMLElement[] = [];
    let keep: Element | null = travel ? coverB : null;
    if (!keep) for (const c of app?.children ?? []) if (c instanceof HTMLElement && !c.classList.contains("bar")) inFade.push(c);
    while (keep && keep !== app && keep.parentElement) {
      for (const c of keep.parentElement.children) if (c !== keep && c instanceof HTMLElement && !c.classList.contains("bar")) inFade.push(c);
      keep = keep.parentElement;
    }
    // The live grounds stay out of it (the frame paints the ground), and out of the fade.
    const grounds = [...replica.querySelectorAll<HTMLElement>(".stage-wash")];
    for (const g of grounds) g.style.visibility = "hidden";
    const inFaded = still(inFade.filter((n) => !grounds.includes(n)));
    const newBar = barParts(replica);
    if (newBar) still(newBar.kids);
    // Into the live app, after its bar (the app's own lookups find its bar first) and under it:
    // the capsule, as wide as the frame, then the old shape's bar, its buttons over the capsule
    // and the live app's ground, kept to its own columns.
    let pill: HTMLElement | null = null;
    let oldBar: BarParts | null = null;
    const liveBar = newBar?.bar ?? null;
    if (app && liveBar && pillLook) {
      pill = document.createElement("div");
      pill.setAttribute("aria-hidden", "true");
      Object.assign(pill.style, { position: "absolute", zIndex: "2", left: px(inset), top: px(liveBar.offsetTop), height: px(liveBar.offsetHeight), ...pillLook });
      app.append(pill);
      if (oldBarEl && barGrid) {
        oldBarEl.inert = true;
        oldBarEl.setAttribute("aria-hidden", "true");
        // A picture now: none of the app's hooks (its scripts' and the page's lookups would find
        // it), only the ones its look is keyed on.
        for (const n of [oldBarEl, ...oldBarEl.querySelectorAll("*")])
          for (const at of [...n.attributes]) if (at.name.startsWith("data-") && at.name !== "data-act") n.removeAttribute(at.name);
        Object.assign(oldBarEl.style, { zIndex: "2", left: px(inset), right: "auto", bottom: "auto", top: px(liveBar.offsetTop), ...barGrid });
        app.append(oldBarEl);
        oldBar = barPartsOf(oldBarEl);
        still(oldBar.kids);
      }
    }
    switchMorph(el, true);
    if (travel) coverB!.style.transformOrigin = "0 0";
    run = {
      key,
      copy,
      pill,
      oldBarEl: oldBar ? oldBarEl : null,
      grounds,
      outFade,
      inFade: inFaded,
      oldBar,
      newBar,
      caption: titleEl?.querySelector<HTMLElement>(".caption") ?? null,
      cover: travel ? coverB : null,
      from: travel ? { x: fromBox!.x - toBox!.x, y: fromBox!.y - toBox!.y, w: fromBox!.w / toBox!.w, h: 0 } : null,
      inset,
    };
  }

  /** Ends a change of shape: the copy gone, the app as it is. */
  function end() {
    if (!run) return;
    const r = run;
    run = null;
    shell.replaceChildren();
    shell.style.opacity = "0";
    r.pill?.remove();
    r.oldBarEl?.remove();
    for (const g of r.grounds) g.style.removeProperty("visibility");
    switchMorph(el, false);
    el.style.removeProperty("clip-path");
    r.caption?.style.removeProperty("translate");
    const nb = r.newBar;
    if (nb) for (const n of [nb.bar, ...nb.mid, nb.end]) n?.style.removeProperty("translate");
    // Back to the app's own, at once (then its own fades again).
    const back = [...r.inFade, ...(nb?.kids ?? [])];
    for (const n of back) n.style.removeProperty("opacity");
    if (back[0]) getComputedStyle(back[0]).opacity;
    for (const n of back) n.style.removeProperty("transition");
    if (r.cover) for (const p of ["translate", "scale", "transform-origin"]) r.cover.style.removeProperty(p);
    // Laid out again as it ends (the story draws the hold's shape next), the parts that appear
    // would play their entrance (the queue's fade): the move has shown them already.
    requestAnimationFrame(() => {
      for (const an of el.getAnimations({ subtree: true }))
        if (an instanceof CSSAnimation && an.effect?.getComputedTiming().iterations !== Infinity && Number(an.currentTime ?? 0) < 120) an.finish();
    });
  }

  /** The picture at `t` (0 the old shape, 1 the new) of the change from `a` to `b`: the frame
   *  for the window (win.drive), with the frame, the copy and the parts placed. */
  function frame(key: string, a: Hold, b: Hold, t: number): Frame {
    if (run?.key !== key) {
      end();
      begin(key, a, b);
    }
    const e = smooth(t);
    const s = lerp(a.s, b.s, e);
    const x = lerp(a.x, b.x, e);
    const y = lerp(a.y, b.y, e);
    // The frame, in the app's px at this scale.
    const W = lerp(a.w * a.s, b.w * b.s, e) / s;
    const H = lerp(a.h * a.s, b.h * b.s, e) / s;
    const f: Frame = { w: b.w, h: b.h, x, y, s, o: 1 };
    if (!run) return f;
    const r = run;
    const corner = (CORNER / s).toFixed(2);
    Object.assign(shell.style, {
      opacity: "1",
      width: px(W),
      height: px(H),
      borderRadius: `${corner}px`,
      transform: `translate(${x.toFixed(2)}px, ${y.toFixed(2)}px) scale(${s.toFixed(5)})`,
    });
    // The words: the old shape's out, then the new one's in; the bar's buttons a little ahead.
    const outO = (1 - seg(t, 0.05, 0.42)).toFixed(3);
    const inO = seg(t, 0.58, 0.92).toFixed(3);
    for (const n of r.outFade) n.style.opacity = outO;
    for (const n of r.inFade) n.style.opacity = inO;
    const outB = (1 - seg(t, 0.05, 0.4)).toFixed(3);
    const inB = seg(t, 0.5, 0.85).toFixed(3);
    for (const n of r.oldBar?.kids ?? []) n.style.opacity = outB;
    for (const n of r.newBar?.kids ?? []) n.style.opacity = inB;
    // The capsule as wide as the frame, at its foot; each shape's buttons kept to their sides.
    if (r.pill) {
      r.pill.style.width = px(W - 2 * r.inset);
      r.pill.style.translate = `0 ${px(H - b.h)}`;
    }
    placeBar(r.oldBar, W - a.w, H - b.h);
    placeBar(r.newBar, W - b.w, H - b.h);
    if (r.caption) r.caption.style.translate = px(W - b.w);
    // The app is drawn in the frame: past its layout where the frame is larger, cut to it where
    // the frame is smaller.
    el.style.clipPath = `inset(0 ${(b.w - W).toFixed(2)}px ${(b.h - H).toFixed(2)}px 0 round ${corner}px)`;
    if (r.cover && r.from) {
      // There before the new shape's words come.
      const c = 1 - smooth(seg(t, 0, 0.7));
      r.cover.style.translate = `${px(r.from.x * c)} ${px(r.from.y * c)}`;
      r.cover.style.scale = lerp(1, r.from.w, c).toFixed(4);
    }
    return f;
  }

  return {
    frame,
    end,
    get active() {
      return !!run;
    },
  };
}
