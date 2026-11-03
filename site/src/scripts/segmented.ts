// The segmented controls' pill (src/components/Segmented.astro): a raised pill moves to the
// picked segment's box (a translate and a width), and the lit copy of the labels is clipped
// to the same box, both easing there on the app's glide curve. It follows the picked radio
// however it was picked (a click, the arrows, or code that sets `checked` and calls
// refreshSegmented), and the control's size (fonts, resizes).

const PAD = 3; // the track's padding (Segmented.astro)

function place(track: HTMLElement) {
  const lit = track.querySelector<HTMLElement>("[data-seg-lit]");
  const pill = track.querySelector<HTMLElement>("[data-seg-pill]");
  const input = track.querySelector<HTMLInputElement>("input:checked");
  if (!lit || !pill) return;
  // Inside a block that sits out a theme switch (content-visibility: hidden) it would force
  // that block's layout; theme.ts places it again when the block is shown.
  if (!track.checkVisibility()) return;
  if (!input) {
    lit.style.clipPath = `inset(${PAD}px 100% ${PAD}px 0 round 999px)`;
    pill.style.opacity = "0";
    return;
  }
  const label = input.closest("label")!;
  const left = label.offsetLeft;
  const right = track.clientWidth - left - label.offsetWidth;
  const top = label.offsetTop;
  const bottom = track.clientHeight - top - label.offsetHeight;
  lit.style.clipPath = `inset(${top}px ${right}px ${bottom}px ${left}px round 999px)`;
  pill.style.opacity = "";
  pill.style.width = `${label.offsetWidth}px`;
  pill.style.transform = `translateX(${left}px)`;
}

const tracks = new Set<HTMLElement>();
let resize: ResizeObserver | undefined;

/** Moves every control's pill (or those inside `root`) to its picked radio. */
export function refreshSegmented(root: ParentNode = document): void {
  for (const t of root.querySelectorAll<HTMLElement>("[data-seg][data-glide]")) place(t);
}

/** Wires every segmented control on the page, once each. */
export function initSegmented(): void {
  // A new size (shown, fonts in, a resize) moves the pill at once; only a pick travels.
  resize ??= new ResizeObserver((entries) => {
    for (const e of entries) {
      const t = e.target as HTMLElement;
      const moving = [...t.querySelectorAll<HTMLElement>("[data-seg-lit], [data-seg-pill]")];
      for (const m of moving) m.style.transition = "none";
      place(t);
      t.getBoundingClientRect();
      for (const m of moving) m.style.transition = "";
    }
  });
  for (const track of document.querySelectorAll<HTMLElement>("[data-seg]")) {
    if (tracks.has(track)) continue;
    tracks.add(track);
    track.dataset.glide = "";
    place(track);
    track.addEventListener("change", () => place(track));
    resize.observe(track);
    // The first placement doesn't travel; every pick after it glides.
    requestAnimationFrame(() => requestAnimationFrame(() => (track.dataset.glideOn = "")));
  }
}
