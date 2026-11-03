// The cover glow: the page takes the playing song's colour. `--glow` on :root changes at
// once (the replica and the wallpaper read it); every `[data-glow]` field on the page then
// eases to the new colour over --t-glow (900 ms) by crossfading two layers, so only
// opacity moves. Under reduced motion --t-glow is 0 and it's instant.

export function setGlow(colour: string): void {
  const root = document.documentElement;
  if (root.style.getPropertyValue("--glow") === colour) return;
  root.style.setProperty("--glow", colour);
  for (const field of document.querySelectorAll<HTMLElement>("[data-glow]")) {
    const layers = [...field.querySelectorAll<HTMLElement>("[data-glow-layer]")];
    if (layers.length < 2) {
      layers[0]?.style.setProperty("--c", colour);
      continue;
    }
    const front = layers.find((l) => l.dataset.on === "true") ?? layers[0];
    if (front.style.getPropertyValue("--c").trim().toLowerCase() === colour.toLowerCase()) continue;
    const back = layers.find((l) => l !== front)!;
    back.style.setProperty("--c", colour);
    back.dataset.on = "true";
    front.dataset.on = "false";
  }
}
