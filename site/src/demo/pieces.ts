// The small live pieces' own rules, apart from the page so they can be tested: the tray's
// two-line tooltip, scrobbling by the time actually heard, SponsorBlock's playhead and the
// Media Session's metadata. src/demo/bind-pieces.ts draws them from the player store.

import type { Song } from "../data/songs";
import { formatTime } from "./player";

type Now = { song: Song | null; playing: boolean; positionMs: number };

/** Windows' tooltip on featherplay's tray icon: the title, then the artist and the time. */
export function trayLines({ song, playing, positionMs }: Now): [string, string] {
  if (!song) return ["featherplay", "Nothing playing"];
  const at = formatTime(positionMs);
  return [song.title, playing ? `${song.artist}, ${at} of ${formatTime(song.durationMs)}` : `${song.artist}, paused at ${at}`];
}

/** Last.fm's rule: a song is scrobbled once half of it has been heard, four minutes at most. */
export function scrobbleAfterMs(durationMs: number): number {
  return Math.min(durationMs / 2, 240_000);
}

/** A step forward larger than this is a seek (or a stall), not time heard. */
const HEARD_STEP_MS = 1500;

export type Listen = {
  heardMs: number;
  /** 0..1 of the way to the scrobble. */
  progress: number;
  scrobbled: boolean;
  /** True on the one step the scrobble happens. */
  justScrobbled: boolean;
  /** On the step a new song starts: the one before, if it was scrobbled. */
  finished: Song | null;
};

/** Counts the time heard of the song playing, as the app's scrobbler does, step by step. */
export function createListen() {
  let song: Song | null = null;
  let last = 0;
  let heard = 0;
  let scrobbled = false;
  return {
    step({ song: now, playing, positionMs }: Now): Listen {
      let finished: Song | null = null;
      if (now?.id !== song?.id) {
        if (scrobbled) finished = song;
        song = now;
        heard = 0;
        scrobbled = false;
      } else {
        const d = positionMs - last;
        if (playing && d > 0 && d < HEARD_STEP_MS) heard += d;
      }
      last = positionMs;
      const need = song ? scrobbleAfterMs(song.durationMs) : Infinity;
      const was = scrobbled;
      if (heard >= need) scrobbled = true;
      return {
        heardMs: heard,
        progress: Math.min(1, heard / need),
        scrobbled,
        justScrobbled: scrobbled && !was,
        finished,
      };
    },
  };
}

/** Where a song (0..1 played) is in its music video, in %: the video talks over its first 7 %
 *  and last 12 %, and SponsorBlock skips both. */
export function sponsorPlayhead(fraction: number): number {
  return 7 + 81 * Math.min(1, Math.max(0, fraction));
}

/** The system's media controls show the song as the app gives it. */
export function mediaMetadataFor(song: Song, origin: string) {
  return {
    title: song.title,
    artist: song.artist,
    album: song.album?.title ?? "",
    artwork: [{ src: new URL(song.cover, origin).href, sizes: "600x600", type: "image/webp" }],
  };
}
