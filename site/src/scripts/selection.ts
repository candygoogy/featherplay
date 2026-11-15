// Selected text, drawn: rounded highlights in the playing song's colour behind the selected
// lines, that fade and grow in, follow the selection as it changes, and ease to the next song's
// colour on the page's one colour clock. Browsers can't round or animate ::selection, so native
// selection stays (copy, the keyboard, screen readers all work as ever) with its background
// made transparent (base.css, under html.sel-drawn), and this draws the wash from the
// selection's own line boxes.
//
// The pieces lie on two layers, blended so the text keeps its own colour on any surface:
// screened under light ink (the dark theme's text, the white lines on the lyric card), multiplied
// under dark ink (the light theme's text, a light window on a dark page). Both lie under the bar.
//
// Fields keep the native highlight, and so does what floats over the page in the top layer or
// the bar (flyouts, the menu, the dialog, the bar itself): a piece drawn under them would be
// hidden. Under reduced motion (and forced colours) none of this runs: the native wash stays.

export type Box = { left: number; top: number; right: number; bottom: number; light?: boolean };

/** Whether a text colour is light (its pieces are screened) or dark (multiplied), from its
 *  computed value: rgb()/rgba(), color(srgb ...) or oklch(). Its alpha doesn't count. */
export function inkIsLight(colour: string): boolean {
  const n = (colour.match(/-?[\d.]+(?:e-?\d+)?%?/g) ?? []).map((v) => (v.endsWith("%") ? parseFloat(v) / 100 : parseFloat(v)));
  if (colour.startsWith("oklch") || colour.startsWith("oklab")) return (n[0] ?? 0) > 0.62;
  const [r = 0, g = 0, b = 0] = colour.startsWith("color(") ? n : n.map((v) => v / 255);
  const lin = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b) > 0.18;
}

/** Most pieces drawn at once (a select-all on a long page draws the ones in view). */
const MAX = 240;
const PAD_X = 2;
const PAD_Y = 0; // the line boxes already meet on tight headings; no seam where they touch

/**
 * Merges a range's line boxes into one box per run of text on a line: boxes on the same line
 * (their middles closer than half the shorter's height) that touch or nearly touch become one.
 */
export function mergeLines(rects: Box[], gap = 6): Box[] {
  const sorted = rects
    .filter((r) => r.right - r.left >= 1 && r.bottom - r.top >= 1)
    .sort((a, b) => a.top - b.top || a.left - b.left);
  const out: Box[] = [];
  for (const r of sorted) {
    const mid = (r.top + r.bottom) / 2;
    const h = r.bottom - r.top;
    const same = out.find((o) => {
      const oh = o.bottom - o.top;
      return !o.light === !r.light && Math.abs((o.top + o.bottom) / 2 - mid) < Math.min(h, oh) / 2 && r.left <= o.right + gap && r.right >= o.left - gap;
    });
    if (same) {
      same.left = Math.min(same.left, r.left);
      same.right = Math.max(same.right, r.right);
      same.top = Math.min(same.top, r.top);
      same.bottom = Math.max(same.bottom, r.bottom);
    } else out.push({ ...r });
  }
  return out;
}

const isField = (el: Element | null) => !!el && (el.matches("input, textarea") || (el as HTMLElement).isContentEditable);
/** What floats over the page (the top layer, the bar): its text keeps the native wash. */
const FLOATS = "[popover], dialog, [data-site-bar]";

type Clip = { left: number; top: number; right: number; bottom: number };
/** What an element's ancestors leave of it on screen (overflow other than visible clips),
 *  or null where nothing clips it. Cached per ancestor for one drawing. */
function clipOf(el: Element, cache: Map<Element, Clip | null>): Clip | null {
  const up = el.parentElement;
  if (!up || up === document.body) return null;
  if (cache.has(up)) return cache.get(up)!;
  const outer = clipOf(up, cache);
  const cs = getComputedStyle(up);
  let clip = outer;
  if (cs.overflowX !== "visible" || cs.overflowY !== "visible") {
    const r = up.getBoundingClientRect();
    clip = outer
      ? { left: Math.max(r.left, outer.left), top: Math.max(r.top, outer.top), right: Math.min(r.right, outer.right), bottom: Math.min(r.bottom, outer.bottom) }
      : { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
  }
  cache.set(up, clip);
  return clip;
}

/** What a drawing keeps from the last change of selection (a scroll changes no node, line
 *  height or ink): each range's text nodes, and each element's line height and ink. */
type Kept = { ranges: Range[]; nodes: Text[][]; ink: Map<Element, { lh: number; light: boolean } | null> };

/** The selection's text line boxes (viewport pixels), only those near the viewport and only
 *  what their clipping ancestors show. `kept` is reused while the selection is unchanged. */
function lineBoxes(sel: Selection, kept: Kept): Box[] {
  const clips = new Map<Element, Clip | null>();
  const boxes: Box[] = [];
  const vh = innerHeight;
  const vw = innerWidth;
  for (let i = 0; i < sel.rangeCount; i++) {
    const range = sel.getRangeAt(i);
    if (range.collapsed) continue;
    const root = range.commonAncestorContainer;
    // One text node: its own boxes. Otherwise each text node in the range, so whole elements
    // in the range give their lines, not their block boxes. Walked once a change of selection.
    let nodes = kept.ranges[i] === range ? kept.nodes[i] : undefined;
    if (!nodes) {
      nodes = [];
      if (root.nodeType === Node.TEXT_NODE) nodes.push(root as Text);
      else {
        const walk = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
          acceptNode: (n) =>
            n.nodeValue && n.nodeValue.trim() && range.intersectsNode(n) ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT,
        });
        for (let n = walk.nextNode(); n; n = walk.nextNode()) nodes.push(n as Text);
      }
      kept.ranges[i] = range;
      kept.nodes[i] = nodes;
    }
    const sub = document.createRange();
    const lineOf = kept.ink;
    for (const n of nodes) {
      const parent = n.parentElement;
      if (!parent || isField(parent)) continue;
      // Large type set tight (the headings) has glyph boxes taller than its lines: a piece is
      // never taller than the line it's on, so lines don't overlap. Read once an element.
      let ink = lineOf.get(parent);
      if (ink === undefined) {
        if (parent.closest(FLOATS)) ink = null;
        else {
          const cs = getComputedStyle(parent);
          ink = { lh: parseFloat(cs.lineHeight) || parseFloat(cs.fontSize) * 1.3, light: inkIsLight(cs.color) };
        }
        lineOf.set(parent, ink);
      }
      if (!ink) continue;
      const lh = ink.lh;
      // Skip text whose element is well off screen (cheap: layout is clean here).
      const pr = parent.getBoundingClientRect();
      if (pr.bottom < -vh || pr.top > vh * 2 || pr.width === 0) continue;
      const clip = clipOf(parent, clips);
      sub.setStart(n, n === range.startContainer ? range.startOffset : 0);
      sub.setEnd(n, n === range.endContainer ? range.endOffset : n.length);
      for (const r of sub.getClientRects()) {
        if (r.bottom < -40 || r.top > vh + 40 || r.right < 0 || r.left > vw) continue;
        // The parent's line height is in its own pixels; a scaled ancestor (the laptop's screen)
        // draws it smaller, as the rect is.
        const scale = parent.offsetHeight ? Math.min(1, pr.height / parent.offsetHeight) || 1 : 1;
        const h = Math.min(r.height, lh * scale);
        const mid = (r.top + r.bottom) / 2;
        const b = { left: r.left, top: mid - h / 2, right: r.right, bottom: mid + h / 2, light: ink.light };
        const c = clip;
        if (c) {
          b.left = Math.max(b.left, c.left);
          b.right = Math.min(b.right, c.right);
          b.top = Math.max(b.top, c.top);
          b.bottom = Math.min(b.bottom, c.bottom);
          // Mostly clipped away: not drawn at all (a sliver at the window's edge reads as a mark).
          if (b.right - b.left < 2 || b.bottom - b.top < h * 0.5) continue;
        }
        boxes.push(b);
        if (boxes.length > MAX * 4) return boxes;
      }
    }
  }
  return boxes;
}

export function bindSelection(): () => void {
  // Reduced motion and forced colours keep the native highlight.
  if (matchMedia("(prefers-reduced-motion: reduce), (forced-colors: active)").matches) return () => {};
  const root = document.documentElement;
  // Under dark ink, multiplied; under light ink, screened (base.css).
  const layers = (["multiply", "screen"] as const).map((blend) => {
    const layer = document.createElement("div");
    layer.className = "sel-layer";
    layer.dataset.blend = blend;
    layer.setAttribute("aria-hidden", "true");
    document.body.append(layer);
    // Faded out: cleared, so the next selection grows in fresh.
    layer.addEventListener("transitionend", (e) => {
      if (e.target === layer && e.propertyName === "opacity" && layer.dataset.out === "true") {
        layer.replaceChildren();
        delete layer.dataset.out;
      }
    });
    return layer;
  });
  root.classList.add("sel-drawn");

  let frame = 0;
  let glide = false; // this frame follows a change of selection (it may glide), not a scroll
  const fresh = (): Kept => ({ ranges: [], nodes: [], ink: new Map() });
  let kept = fresh();

  const draw = () => {
    frame = 0;
    const sel = document.getSelection();
    const field = isField(document.activeElement);
    const all = !sel || sel.isCollapsed || field ? [] : mergeLines(lineBoxes(sel, kept)).slice(0, MAX);
    const x0 = scrollX;
    const y0 = scrollY;
    layers.forEach((layer, k) => {
      const boxes = all.filter((b) => !!b.light === (k === 1));
      layer.dataset.glide = String(glide);
      const pieces = layer.children;
      // Gone: the pieces fade out together, then leave.
      if (!boxes.length) {
        if (pieces.length) layer.dataset.out = "true";
        return;
      }
      delete layer.dataset.out;
      boxes.forEach((b, i) => {
        let el = pieces[i] as HTMLElement | undefined;
        if (!el) {
          el = document.createElement("span");
          layer.append(el);
        }
        el.style.translate = `${Math.round(b.left - PAD_X + x0)}px ${Math.round(b.top - PAD_Y + y0)}px`;
        el.style.width = `${Math.round(b.right - b.left + PAD_X * 2)}px`;
        el.style.height = `${Math.round(b.bottom - b.top + PAD_Y * 2)}px`;
      });
      while (pieces.length > boxes.length) pieces[pieces.length - 1].remove();
    });
    glide = false;
  };

  const schedule = (moving: boolean) => {
    if (moving) glide = true;
    if (!frame) frame = requestAnimationFrame(draw);
  };
  const onSelection = () => {
    kept = fresh();
    schedule(true);
  };
  const onScroll = () => {
    if (layers.some((l) => l.children.length)) schedule(false);
  };
  // A resize or a theme may change the lines and the inks: read again.
  const onResize = () => {
    kept = fresh();
    onScroll();
  };

  document.addEventListener("selectionchange", onSelection);
  addEventListener("scroll", onScroll, { passive: true });
  addEventListener("resize", onResize, { passive: true });
  addEventListener("themechange", onResize);
  return () => {
    cancelAnimationFrame(frame);
    document.removeEventListener("selectionchange", onSelection);
    removeEventListener("scroll", onScroll);
    removeEventListener("resize", onResize);
    removeEventListener("themechange", onResize);
    root.classList.remove("sel-drawn");
    for (const l of layers) l.remove();
  };
}
