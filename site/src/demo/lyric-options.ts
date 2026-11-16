// The app's lyric options beside the lyric card (Lyrics.astro), each doing here what it does
// in the app: Text size sets the card's lines (lyrics_menu.slint, three A's); Timing moves the
// song's lines half a second sooner or later, for that song, with Reset (the store keeps it,
// so the window's lyrics follow it too); Desktop lyrics shows the line being sung and the next
// in a strip over the card's edge (desktop_lyrics.slint), with its A−, A+ and ×; Show lyrics
// off (Settings) takes the lyrics away everywhere and shows the cover instead. Nothing here
// starts a song.

import type { Song } from "../data/songs";
import { MAX_NUDGE_MS, lineNow, lyricsSource, nudgeOf, type Player, type PlayerState } from "./player";

/** Timing moves the lines half a second a step (the app's lyrics.rs). */
const NUDGE_STEP_MS = 500;

/** The timing as the lyrics menu says it: "On time", "0.5 s later", "1 s sooner". */
export function timingLabel(ms: number): string {
  return ms ? `${Math.abs(ms) / 1000} s ${ms > 0 ? "later" : "sooner"}` : "On time";
}

/** The credit under the lines, with the timing once it was moved (the app's credit_line):
 *  "Lyrics: Josh Woodward, CC BY 4.0 · 0.5 s later for this song". */
export function creditLine(song: Song, nudgeMs = 0): string {
  return nudgeMs ? `${lyricsSource(song)} · ${timingLabel(nudgeMs)} for this song` : lyricsSource(song);
}

/** What the strip shows (desktop_lyrics.rs `shown`): the line being sung and the next; "♪"
 *  with the first line under it before the singing starts; for a song without lyrics, its
 *  title and a quiet note under it. */
export function stripLines(song: Song, line: number): { line: string; under: string; quiet: boolean } {
  if (!song.lyrics.length) return { line: song.title, under: `${song.artist} · No lyrics for this song`, quiet: true };
  return { line: line < 0 ? "♪" : song.lyrics[line].line, under: song.lyrics[line + 1]?.line ?? "", quiet: false };
}

export function bindLyricOptions(root: HTMLElement, player: Player): void {
  const card = root.querySelector<HTMLElement>("[data-piece='lyrics']");
  const opts = root.querySelector<HTMLElement>("[data-lyr-opts]");
  const strip = root.querySelector<HTMLElement>("[data-strip]");
  if (!card || !opts || !strip) return;
  opts.inert = false;
  const $ = <T extends Element = HTMLElement>(sel: string, from: ParentNode = opts) => from.querySelector<T>(sel)!;
  const set = $<HTMLFieldSetElement>("[data-lo-set]");
  const sizes = [...opts.querySelectorAll<HTMLButtonElement>("[data-lo-size]")];
  const steps = [...opts.querySelectorAll<HTMLButtonElement>("[data-lo-nudge]")];
  const value = $("[data-lo-timing]");
  const hint = $("[data-lo-hint]");
  const moved = $("[data-lo-moved]");
  const desk = $("[data-lo-desk]");
  const show = $("[data-lo-show]");
  const credit = card.querySelector("[data-lc-source]");
  const now = $("[data-st-now]", strip);
  const was = $("[data-st-was]", strip);
  const grow = [...strip.querySelectorAll<HTMLButtonElement>("[data-st-step]")];
  const section = root.closest("section") ?? root;
  const reduced = matchMedia("(prefers-reduced-motion: reduce)");

  let deskOn = false;
  let stripSize = 1;
  let shown = ""; // the strip's lines as last set

  // ---- Text size: the card's lines at once, the line sung placed again ----
  sizes.forEach((b, i) =>
    b.addEventListener("click", () => {
      card.dataset.size = String(i);
      sizes.forEach((s) => s.setAttribute("aria-pressed", String(s === b)));
      card.dispatchEvent(new Event("lc-relayout"));
    }),
  );

  // ---- Timing, Desktop lyrics, Show lyrics ----
  opts.addEventListener("click", (e) => {
    const t = e.target as Element;
    const step = t.closest<HTMLElement>("[data-lo-nudge]");
    if (step) player.setNudge(nudgeOf(player.state) + Number(step.dataset.loNudge) * NUDGE_STEP_MS);
    else if (t.closest("[data-lo-reset]")) {
      player.setNudge(0);
      // The link goes with the moved state: the keys stay in the row, on the − button.
      steps[0]?.focus();
    } else if (t.closest("[data-lo-desk]")) setDesk(!deskOn);
    else if (t.closest("[data-lo-show]")) player.setLyrics(!player.state.lyricsOn);
  });
  strip.addEventListener("click", (e) => {
    const t = e.target as Element;
    const b = t.closest<HTMLElement>("[data-st-step]");
    if (b) {
      stripSize = Math.max(0, Math.min(2, stripSize + Number(b.dataset.stStep)));
      strip.dataset.stSize = String(stripSize);
      grow.forEach((g) => (g.disabled = (Number(g.dataset.stStep) < 0 ? 0 : 2) === stripSize));
    } else if (t.closest("[data-st-close]")) {
      setDesk(false);
      desk.focus();
    }
  });

  function setDesk(on: boolean) {
    deskOn = on;
    desk.setAttribute("aria-checked", String(on));
    shown = "";
    render(player.state);
  }

  /** The strip's lines, the new ones rising into place as the last ones go up (t-strip-line). */
  function fillStrip(s: PlayerState) {
    const song = s.song!;
    const l = stripLines(song, lineNow(s));
    const key = `${l.line}\n${l.under}`;
    if (key === shown) return;
    const first = !shown;
    shown = key;
    const [line, under] = now.children as HTMLCollectionOf<HTMLElement>;
    const glide = !first && !reduced.matches && !section.hasAttribute("data-away");
    if (glide) {
      const [wl, wu] = was.children as HTMLCollectionOf<HTMLElement>;
      wl.textContent = line.textContent;
      wu.textContent = under.textContent;
      wu.toggleAttribute("data-quiet", under.hasAttribute("data-quiet"));
    }
    line.textContent = l.line;
    under.textContent = l.under;
    under.toggleAttribute("data-quiet", l.quiet);
    if (!glide) return;
    const timing = { duration: 230, easing: "cubic-bezier(0.23, 1, 0.32, 1)" };
    was.animate(
      [
        { visibility: "visible", transform: "none", opacity: 1 },
        { visibility: "visible", transform: "translateY(-14px)", opacity: 0 },
      ],
      timing,
    );
    now.animate([{ transform: "translateY(14px)", opacity: 0 }, { transform: "none", opacity: 1 }], timing);
  }

  let last: { song?: string; nudge?: number; on?: boolean; desk?: boolean } = {};
  const render = (s: PlayerState) => {
    const song = s.song;
    if (!song) return;
    const nudge = nudgeOf(s);
    const has = song.lyrics.length > 0;
    if (song.id !== last.song || nudge !== last.nudge) {
      value.textContent = timingLabel(nudge);
      hint.hidden = nudge !== 0;
      moved.hidden = nudge === 0;
      // Synced lines only: a song without them has no timing to move.
      steps[0].disabled = !has || nudge <= -MAX_NUDGE_MS;
      steps[1].disabled = !has || nudge >= MAX_NUDGE_MS;
    }
    // The card fills its credit with each song (bind-pieces.ts): the timing's note goes after it.
    const said = has ? creditLine(song, nudge) : "";
    if (credit && credit.textContent !== said) credit.textContent = said;
    if (s.lyricsOn !== last.on) {
      set.disabled = !s.lyricsOn;
      show.setAttribute("aria-checked", String(s.lyricsOn));
      card.toggleAttribute("data-off", !s.lyricsOn);
    }
    // Desktop lyrics follow Show lyrics: off, the strip goes too.
    const strips = deskOn && s.lyricsOn;
    if (strips !== last.desk) {
      strip.hidden = !strips;
      if (strips) {
        shown = "";
        if (!reduced.matches) strip.animate([{ opacity: 0, translate: "0 calc(-50% + 4px)" }, { opacity: 1, translate: "0 -50%" }], { duration: 180, easing: "ease-out" });
      }
    }
    if (strips) fillStrip(s);
    last = { song: song.id, nudge, on: s.lyricsOn, desk: strips };
  };
  player.subscribe(render);
}
