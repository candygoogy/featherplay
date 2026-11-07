// The demo's songs: six real tracks under free licences (CC BY), each a
// short clip that loads only after the visitor presses play (src/demo/audio.ts). The covers
// are our own art (none of the tracks' artwork is licensed). The lyrics are the artists'
// own, under the song's licence, shipped only for the three Josh Woodward songs; the timings
// are ours, relative to the clip's start. Each credit's `sourceUrl` is where its licence
// is stated.

export type LyricLine = { atMs: number; line: string };

export type Credit = {
  /** "CC BY 4.0", "CC BY 3.0". */
  licence: string;
  licenceUrl: string;
  /** The track's own page, where the licence is stated. */
  sourceUrl: string;
  /** The artist's site, as the short credit by the controls names it. */
  site: { label: string; url: string };
};

export type Song = {
  id: string;
  title: string;
  artist: string;
  /**
   * The album it's from, as Now playing's third line shows it ("album · year"), the year as
   * the artist's page gives it. None for a track that isn't on one (a remix upload, a single
   * of its own): the app leaves the line out for a track without an album.
   */
  album?: { title: string; year: string };
  /** Under public/, 600 px webp. */
  cover: string;
  /** The cover's colour: the glow and the lyrics panel. AA-checked in tests/unit/data.test.ts. */
  colour: string;
  /** The clip's length. */
  durationMs: number;
  /** Synced lines; empty shows the app's calm no-lyrics layout. */
  lyrics: LyricLine[];
  /** The clip, two encodings under public/: Opus in WebM first, AAC for Safari. */
  clip?: { opus: string; m4a: string };
  credit?: Credit;
};

const lines = (pairs: [number, string][]): LyricLine[] => pairs.map(([atMs, line]) => ({ atMs, line }));

const ccBy4 = { licence: "CC BY 4.0", licenceUrl: "https://creativecommons.org/licenses/by/4.0/" };
const ccBy3 = { licence: "CC BY 3.0", licenceUrl: "https://creativecommons.org/licenses/by/3.0/" };
const joshWoodward = { label: "joshwoodward.com", url: "https://www.joshwoodward.com/" };
const scottBuckley = { label: "scottbuckley.com.au", url: "https://www.scottbuckley.com.au/" };

const media = (id: string) => ({
  cover: `/covers/${id}.webp`,
  clip: { opus: `/music/${id}.webm`, m4a: `/music/${id}.m4a` },
});

export const songs: Song[] = [
  {
    id: "california-lullabye",
    title: "California Lullabye",
    artist: "Josh Woodward",
    album: { title: "The Beautiful Machine", year: "2014" },
    ...media("california-lullabye"),
    colour: "#E8894A",
    durationMs: 54_500,
    lyrics: lines([
      [700, "But distance lost its fight"],
      [6_200, "When you laid down at night"],
      [12_010, "Without a map, you chose your heart"],
      [14_230, "Now we are one, but you're still far from home"],
      [20_450, "So let me shake away the cloudy skies"],
      [25_690, "With a California lullabye"],
      [30_700, "And when you're frozen with desire"],
      [33_650, "We'll put our toes up to the fire"],
      [36_440, "And I will sing your cares away"],
      [42_550, "And when you're frozen with desire"],
      [44_600, "We'll put our toes up to the fire"],
      [47_700, "And I will sing your cares away"],
    ]),
    credit: { ...ccBy4, sourceUrl: "https://www.joshwoodward.com/song/CaliforniaLullabye", site: joshWoodward },
  },
  {
    id: "invisible-light",
    title: "Invisible Light",
    artist: "Josh Woodward",
    album: { title: "The Wake", year: "2012" },
    ...media("invisible-light"),
    colour: "#3E4FA8",
    durationMs: 59_900,
    lyrics: lines([
      [1_990, "You come, from outer space"],
      [7_870, "And I, I sprung out from the earth"],
      [13_500, "We lay down, face to face"],
      [19_820, "To witness our rebirth"],
      [25_790, "And I'm not leaving while you're breathing"],
      [31_800, "I won't run away"],
      [37_500, "They say it won't last, and you'll move along"],
      [40_100, "And that we're moving too fast, but I know they're wrong"],
      [43_700, "Cuz we are like invisible light in the dark"],
      [49_000, "There is an endless road on the edge of the map"],
      [52_100, "So let's pack our hats in a burlap sack and go"],
    ]),
    credit: { ...ccBy4, sourceUrl: "https://www.joshwoodward.com/song/InvisibleLight", site: joshWoodward },
  },
  {
    id: "she-dreams-in-blue",
    title: "She Dreams in Blue",
    artist: "Josh Woodward",
    album: { title: "Not Quite Connected", year: "2007" },
    ...media("she-dreams-in-blue"),
    colour: "#5B7FB8",
    durationMs: 60_500,
    lyrics: lines([
      [970, "The secret wish that none will know"],
      [3_550, "She keeps it locked up in her pale heart"],
      [11_870, "Wait for it, it's tired and it's true"],
      [22_430, "Wait for it, it's all she ever knew"],
      [33_170, "She dreams in blue"],
      [43_700, "She dreams in blue"],
      [51_870, "Wait for it, it's all she ever knew"],
    ]),
    credit: { ...ccBy4, sourceUrl: "https://www.joshwoodward.com/song/SheDreamsinBlue", site: joshWoodward },
  },
  {
    // Vocals, but no lyrics on the site: they're published only under a licence we can't use.
    id: "watching-seabirds-soar",
    title: "Watching Seabirds Soar (Harmony)",
    artist: "DoKashiteru feat. Snowflake",
    ...media("watching-seabirds-soar"),
    colour: "#3F8F92",
    durationMs: 58_300,
    lyrics: [],
    credit: {
      ...ccBy3,
      sourceUrl: "https://ccmixter.org/files/DoKashiteru/24925",
      site: { label: "ccmixter.org", url: "https://ccmixter.org/files/DoKashiteru/24925" },
    },
  },
  {
    id: "origami",
    title: "Origami",
    artist: "Scott Buckley",
    ...media("origami"),
    colour: "#D4A23A",
    durationMs: 56_000,
    lyrics: [],
    credit: { ...ccBy4, sourceUrl: "https://www.scottbuckley.com.au/library/origami/", site: scottBuckley },
  },
  {
    id: "in-this-moment",
    title: "In This Moment",
    artist: "Scott Buckley",
    ...media("in-this-moment"),
    colour: "#B98E78",
    durationMs: 56_000,
    lyrics: [],
    credit: { ...ccBy4, sourceUrl: "https://www.scottbuckley.com.au/library/in-this-moment/", site: scottBuckley },
  },
];

/** Where every player on the page starts: paused in the first song's chorus, on the lyrics. */
export const demoStart = { songId: songs[0].id, positionMs: 20_600 };

/**
 * The footer's full credit lines, in each artist's own requested wording (music/README.md),
 * as runs of text and links.
 */
export type CreditPart = string | { text: string; href: string };
export const musicCredits: CreditPart[][] = [
  [
    "“California Lullabye”, “Invisible Light” and “She Dreams in Blue” by Josh Woodward. Free download: ",
    { text: "joshwoodward.com", href: "https://www.joshwoodward.com/" },
    ". Licensed under ",
    { text: "CC BY 4.0", href: ccBy4.licenceUrl },
    ".",
  ],
  [
    "“Watching Seabirds Soar (Harmony)” by ",
    { text: "DoKashiteru", href: "https://ccmixter.org/files/DoKashiteru/24925" },
    ", featuring “Harmony” by ",
    { text: "Snowflake", href: "https://ccmixter.org/files/snowflake/24553" },
    ". Licensed under ",
    { text: "CC BY 3.0", href: ccBy3.licenceUrl },
    ".",
  ],
  [
    "‘Origami’ by Scott Buckley - released under ",
    { text: "CC-BY 4.0", href: ccBy4.licenceUrl },
    ". ",
    { text: "www.scottbuckley.com.au", href: "https://www.scottbuckley.com.au/library/origami/" },
  ],
  [
    "‘In This Moment’ by Scott Buckley - released under ",
    { text: "CC-BY 4.0", href: ccBy4.licenceUrl },
    ". ",
    { text: "www.scottbuckley.com.au", href: "https://www.scottbuckley.com.au/library/in-this-moment/" },
  ],
];
export const musicCreditsNote =
  "All tracks are short excerpts, faded in and out and levelled for the web. Lyric timings are our own.";

export const songById = (id: string) => songs.find((s) => s.id === id) ?? null;
