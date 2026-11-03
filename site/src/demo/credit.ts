// Keeps every `[data-credit]` on the page (src/components/PlayerCredit.astro) on the song
// the demo has loaded: its title, artist, site and licence.

import type { Player } from "./player";

export function bindCredits(player: Player): void {
  const credits = [...document.querySelectorAll<HTMLElement>("[data-credit]")];
  if (!credits.length) return;
  let shown: string | null = null;
  player.subscribe((s) => {
    const song = s.song;
    if (!song || song.id === shown) return;
    shown = song.id;
    for (const el of credits) {
      // A credit kept by its own section (the lyrics showcase names the song it shows).
      if (el.hasAttribute("data-credit-own")) continue;
      el.hidden = !song.credit;
      if (!song.credit) continue;
      const title = el.querySelector("[data-credit-title]");
      const artist = el.querySelector("[data-credit-artist]");
      const site = el.querySelector<HTMLAnchorElement>("[data-credit-site]");
      const licence = el.querySelector<HTMLAnchorElement>("[data-credit-licence]");
      const all = el.querySelector<HTMLAnchorElement>("[data-credit-all]");
      if (all) all.href = `/credits#${song.id}`;
      if (title) title.textContent = `“${song.title}”`;
      if (artist) artist.textContent = song.artist;
      if (site) {
        site.href = song.credit.site.url;
        site.textContent = song.credit.site.label;
      }
      if (licence) {
        licence.href = song.credit.licenceUrl;
        licence.textContent = song.credit.licence;
      }
    }
  });
}
