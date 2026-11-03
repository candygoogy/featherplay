// Selected text, drawn: rounded highlights behind the selected lines that fade and grow in
// and follow the selection as it changes. Browsers can't round or animate ::selection, so
// native selection stays (copy, the keyboard, screen readers all work as ever) with its
// background made transparent (base.css, under html.sel-drawn), and this draws the wash
// from the selection's own line boxes.
//
// Inputs and text fields keep the native highlight. Under reduced motion none of this runs:
// the native soft wash stays.

export type Box = { left: number; top: number; right: number; bottom: number };

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
      return Math.abs((o.top + o.bottom) / 2 - mid) < Math.min(h, oh) / 2 && r.left <= o.right + gap && r.right >= o.left - gap;
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

/** The selection's text line boxes (viewport pixels), only those near the viewport. */
function lineBoxes(sel: Selection): Box[] {
  const boxes: Box[] = [];
  const vh = innerHeight;
  const vw = innerWidth;
  for (let i = 0; i < sel.rangeCount; i++) {
    const range = sel.getRangeAt(i);
    if (range.collapsed) continue;
    const root = range.commonAncestorContainer;
    // One text node: its own boxes. Otherwise each text node in the range, so whole elements
    // in the range give their lines, not their block boxes.
    const nodes: Text[] = [];
    if (root.nodeType === Node.TEXT_NODE) nodes.push(root as Text);
    else {
      const walk = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
        acceptNode: (n) =>
          n.nodeValue && n.nodeValue.trim() && range.intersectsNode(n) ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT,
      });
      for (let n = walk.nextNode(); n; n = walk.nextNode()) nodes.push(n as Text);
    }
    const sub = document.createRange();
    const lineOf = new Map<Element, number>();
    for (const n of nodes) {
      const parent = n.parentElement;
      if (!parent || isField(parent)) continue;
      // Large type set tight (the headings) has glyph boxes taller than its lines: a piece is
      // never taller than the line it's on, so lines don't overlap.
      let lh = lineOf.get(parent);
      if (lh === undefined) {
        const cs = getComputedStyle(parent);
        lh = parseFloat(cs.lineHeight) || parseFloat(cs.fontSize) * 1.3;
        lineOf.set(parent, lh);
      }
      // Skip text whose element is well off screen (cheap: layout is clean here).
      const pr = parent.getBoundingClientRect();
      if (pr.bottom < -vh || pr.top > vh * 2 || pr.width === 0) continue;
      sub.setStart(n, n === range.startContainer ? range.startOffset : 0);
      sub.setEnd(n, n === range.endContainer ? range.endOffset : n.length);
      for (const r of sub.getClientRects()) {
        if (r.bottom < -40 || r.top > vh + 40 || r.right < 0 || r.left > vw) continue;
        // The parent's line height is in its own pixels; a scaled ancestor (the laptop's screen)
        // draws it smaller, as the rect is.
        const scale = parent.offsetHeight ? Math.min(1, pr.height / parent.offsetHeight) || 1 : 1;
        const h = Math.min(r.height, lh * scale);
        const mid = (r.top + r.bottom) / 2;
        boxes.push({ left: r.left, top: mid - h / 2, right: r.right, bottom: mid + h / 2 });
        if (boxes.length > MAX * 4) return boxes;
      }
    }
  }
  return boxes;
}

export function bindSelection(): () => void {
  if (matchMedia("(prefers-reduced-motion: reduce)").matches) return () => {};
  const root = document.documentElement;
  const layer = document.createElement("div");
  layer.className = "sel-layer";
  layer.setAttribute("aria-hidden", "true");
  document.body.append(layer);
  root.classList.add("sel-drawn");

  let frame = 0;
  let glide = false; // this frame follows a change of selection (it may glide), not a scroll

  const draw = () => {
    frame = 0;
    const sel = document.getSelection();
    const field = isField(document.activeElement);
    const boxes = !sel || sel.isCollapsed || field ? [] : mergeLines(lineBoxes(sel)).slice(0, MAX);
    layer.dataset.glide = String(glide);
    glide = false;
    const pieces = layer.children;
    // Gone: the pieces fade out together, then leave.
    if (!boxes.length) {
      if (pieces.length) layer.dataset.out = "true";
      return;
    }
    delete layer.dataset.out;
    const x0 = scrollX;
    const y0 = scrollY;
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
  };

  const schedule = (moving: boolean) => {
    if (moving) glide = true;
    if (!frame) frame = requestAnimationFrame(draw);
  };
  const onSelection = () => schedule(true);
  const onScroll = () => {
    if (layer.children.length) schedule(false);
  };
  // Faded out: clear the pieces so the next selection grows in fresh.
  layer.addEventListener("transitionend", (e) => {
    if (e.target === layer && layer.dataset.out === "true") {
      layer.replaceChildren();
      delete layer.dataset.out;
    }
  });

  document.addEventListener("selectionchange", onSelection);
  addEventListener("scroll", onScroll, { passive: true });
  addEventListener("resize", onScroll, { passive: true });
  return () => {
    cancelAnimationFrame(frame);
    document.removeEventListener("selectionchange", onSelection);
    removeEventListener("scroll", onScroll);
    removeEventListener("resize", onScroll);
    root.classList.remove("sel-drawn");
    layer.remove();
  };
}
