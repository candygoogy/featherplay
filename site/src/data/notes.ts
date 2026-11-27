// The maker's handwritten notes on the page: asides, never arguments, in a plain lowercase
// voice (mockup 10). Each is decoration where the page says the same thing in its own words, so
// it is hidden from assistive technology (Note.astro). They are set in a subset of Reenie Beanie
// cut to exactly these characters (public/fonts/reenie-beanie-subset.woff2, its licence beside
// it): a new or changed note needs PEN_CHARS and the subset made again (tests/unit/notes.test.ts).

export const notes = {
  /** The first screen, over the window. */
  screenshot: "not a screenshot",
  /** The first screen, over the sleeves. */
  sleeves: "drop one on the player",
  /** By the tray icon, the first time the window comes back from the tray. */
  opens: "opens in 0.31 s",
  /** The story's last hold, by the tray's flyout. */
  playing: "still playing",
  /** Numbers, at featherplay's column. */
  played: "while a song played",
  /** What it leaves out, at featherplay's exe. */
  onefile: "one file",
  /** Lyrics, at the card. */
  line: "click a line",
  /** Make it yours, at the window. */
  actual: "this one’s actual size",
  /** What it keeps, at the media keys, once a song plays with sound (the visitor's own keys
   *  reach the page only then). */
  keys: "try your own media keys",
  /** The same place before that: what to do first. */
  keysFirst: "press play first",
  /** The end of Download. */
  end: "that’s it. go listen to something",
  /** Where the window stood, once the visitor has closed featherplay (the headline's own words). */
  closed: "now it really isn’t running",
  /** The same place, with the window sent to the tray. */
  tray: "still running. easy to forget",
  /** The same place, with the window only moved off it (the shortcut under the note brings it back). */
  moved: "this takes it home",
  /** There too, by one of the desk's other icons (as Lyrics has "click a line"). */
  pick: "click one",
} as const;

export type NoteId = keyof typeof notes;

/** Every character the notes use, and only those: what the font subset holds. */
export const PEN_CHARS = " .013’acdefghiklmnoprstuwyz";
