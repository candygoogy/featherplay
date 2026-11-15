// A song's synced lines as buttons in a list, and which of them is sung now: one module for
// the replica's lyrics (and its side panel), the lyrics showcase and the lyric card. Each line
// is a button that seeks to it (its time in data-at); its state is "past", "current" or "next".

import type { Song } from "../data/songs";

export type LineOptions = {
  /** The button's class (the replica's and the showcase's "line", the card's "lc-line"). */
  className: string;
  /** Marks a line after a long gap as a new verse (the showcase's spacing). */
  verses?: boolean;
};

/** A long gap before a line: a new verse. */
const VERSE_GAP_MS = 5200;

/** The song's lines as list items, each holding its button, all "next" to start with. */
export function lineItems(song: Song, { className, verses = false }: LineOptions): HTMLLIElement[] {
  return song.lyrics.map((l, i) => {
    const li = document.createElement("li");
    if (verses && i > 0 && l.atMs - song.lyrics[i - 1].atMs > VERSE_GAP_MS) li.className = "verse";
    const b = document.createElement("button");
    b.type = "button";
    b.className = className;
    b.dataset.at = String(l.atMs);
    b.dataset.state = "next";
    b.textContent = l.line;
    li.append(b);
    return li;
  });
}

/** Marks line `index` the current one in `list` (its buttons of `selector`), the ones before
 *  it past, the rest next; the current one is aria-current. -1: none sung yet. */
export function markLines(list: ParentNode, selector: string, index: number): void {
  list.querySelectorAll<HTMLElement>(selector).forEach((b, i) => {
    b.dataset.state = i < index ? "past" : i === index ? "current" : "next";
    if (i === index) b.setAttribute("aria-current", "true");
    else b.removeAttribute("aria-current");
  });
}
