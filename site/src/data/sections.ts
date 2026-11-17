// The home page's five sections as the bar names them (Bar.astro), and the 404's queue of
// where to go (pages/404.astro): each its anchor, its word, its section's own title, its icon.
/** One stroke of an icon; `part` the one that moves (src/scripts/moves.ts), `len` drawn on. */
export type IconPart = { d: string; part?: string; len?: boolean };
/** `tip` the bar's tooltip (the section's own title); `about` what is there, in a few words
 *  (the 404's queue, under the name). */
export type Section = { id: string; label: string; tip: string; about: string; parts: IconPart[] };

// Each icon in parts: the part that moves when its section comes into view or the pointer
// rests on it (the Lottie files' motion, src/scripts/moves.ts) is its own path. The question
// mark has none: at 18 px its rewritten hook read as a broken icon. Features (what it keeps,
// and Make it yours before it: Yours.astro's data-bar-sec) is four tiles, the last one the
// piece that is already in.
export const sections: Section[] = [
  {
    id: "app",
    label: "The app",
    tip: "The app, stepping back",
    about: "Whole, narrow, or in the tray",
    parts: [
      { d: "M4 6.5 a2 2 0 0 1 2 -2 h12 a2 2 0 0 1 2 2 v11 a2 2 0 0 1 -2 2 h-12 a2 2 0 0 1 -2 -2 Z" },
      { d: "M4 8.8 H20", part: "titlebar" },
    ],
  },
  {
    id: "light",
    label: "Numbers",
    tip: "Light, and it shows",
    about: "Measured beside Pear and Edge",
    parts: [
      { d: "M4.5 17 A8 8 0 1 1 19.5 17 M10.6 14.2 a1.4 1.4 0 1 0 2.8 0 a1.4 1.4 0 1 0 -2.8 0" },
      { d: "M12 14.2 L15.4 9.8", part: "needle" },
    ],
  },
  {
    id: "lyrics",
    label: "Lyrics",
    tip: "Lyrics, up close",
    about: "Synced lines you can click",
    parts: [
      { d: "M12 3.5 a3 3 0 0 0 -3 3 V11.5 a3 3 0 0 0 6 0 V6.5 a3 3 0 0 0 -3 -3 Z", part: "capsule" },
      { d: "M6 11 A6 6 0 0 0 18 11 M12 17 V20.5" },
      { d: "M17.516 2.572 C 18.772 4.068, 19.365 6.01, 19.161 7.953", part: "arc", len: true },
      { d: "M6.484 2.572 C 5.228 4.068, 4.635 6.01, 4.839 7.953", part: "arc", len: true },
    ],
  },
  {
    id: "kept",
    label: "Features",
    tip: "Everything you’d miss, kept",
    about: "Media keys, Discord, scrobbling",
    parts: [{ d: "M5.85 4.25 h3.3 a1.6 1.6 0 0 1 1.6 1.6 v3.3 a1.6 1.6 0 0 1 -1.6 1.6 h-3.3 a1.6 1.6 0 0 1 -1.6 -1.6 v-3.3 a1.6 1.6 0 0 1 1.6 -1.6 Z M14.85 4.25 h3.3 a1.6 1.6 0 0 1 1.6 1.6 v3.3 a1.6 1.6 0 0 1 -1.6 1.6 h-3.3 a1.6 1.6 0 0 1 -1.6 -1.6 v-3.3 a1.6 1.6 0 0 1 1.6 -1.6 Z M5.85 13.25 h3.3 a1.6 1.6 0 0 1 1.6 1.6 v3.3 a1.6 1.6 0 0 1 -1.6 1.6 h-3.3 a1.6 1.6 0 0 1 -1.6 -1.6 v-3.3 a1.6 1.6 0 0 1 1.6 -1.6 Z" }, { d: "M14.85 13.25 h3.3 a1.6 1.6 0 0 1 1.6 1.6 v3.3 a1.6 1.6 0 0 1 -1.6 1.6 h-3.3 a1.6 1.6 0 0 1 -1.6 -1.6 v-3.3 a1.6 1.6 0 0 1 1.6 -1.6 Z", part: "tile" }],
  },
  {
    id: "questions",
    label: "Questions",
    tip: "Questions and answers",
    about: "Sign-in, privacy, uninstalling",
    parts: [{ d: "M12 3.5 a8.5 8.5 0 1 0 0.01 0 Z M9.7 9.6 a2.4 2.4 0 1 1 3.3 2.2 c-0.6 0.3 -1 0.8 -1 1.5 V14 M12 16.9 V17" }],
  },
];
