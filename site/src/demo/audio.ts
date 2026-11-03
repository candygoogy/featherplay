// Sound for the demo: nothing loads until the visitor presses
// play, and then only the playing song's clip. It follows the player's clock (play, pause,
// the song) and its volume and mute; every `[data-sound]` switch on the page shows while a
// song with sound is current and mutes or unmutes it.
//
// It re-syncs to the clock only when the position jumps (a seek, another song, a restart:
// `state.jumps`), never on the clock's ticks, so a clip that's still loading isn't seeked
// over and over. When the browser refuses to start it (its autoplay rules), the demo goes
// on muted and the Sound switch shows it; pressing Sound is a gesture, and tries again.
// A hidden tab, or the demo scrolled out of view, pauses the player and so the sound.

import type { Player, PlayerState } from "./player";

export function bindAudio(player: Player): void {
  if (typeof Audio === "undefined") return;
  let audio: HTMLAudioElement | null = null;
  let src = "";
  let jumps = -1;
  let wasPlaying = false;
  let wasSilent = false;
  /** The clip ended before the clock did: wait for the next jump or song. */
  let ended = false;
  const switches = [...document.querySelectorAll<HTMLButtonElement>("[data-sound]")];
  for (const b of switches) b.addEventListener("click", () => player.toggleMute());

  const pick = (clip: { opus: string; m4a: string }, el: HTMLAudioElement) =>
    el.canPlayType('audio/webm; codecs="opus"') ? clip.opus : clip.m4a;

  const start = (el: HTMLAudioElement) => {
    el.play().catch((e: unknown) => {
      // The browser said no (no gesture it accepts): carry on silently, and say so.
      if ((e as DOMException)?.name === "NotAllowedError" && !player.state.muted) player.toggleMute();
    });
  };

  const create = () => {
    const el = new Audio();
    el.preload = "auto";
    // The clip ran out a moment before the clock: the clock ends the song (repeat and the
    // end of the list behave as the app's); until then, don't start the clip again.
    el.addEventListener("ended", () => {
      ended = true;
    });
    return el;
  };

  const render = (s: PlayerState) => {
    const clip = s.song?.clip;
    const silent = s.muted || s.volume === 0;
    for (const b of switches) {
      b.hidden = !clip;
      b.setAttribute("aria-pressed", String(!silent));
      b.dataset.on = String(!silent);
    }
    if (!clip) {
      audio?.pause();
      wasPlaying = s.playing;
      return;
    }
    // The visitor pressed play: from here on, sound may load.
    if (!audio) {
      if (!s.playing) return;
      audio = create();
    }
    const want = pick(clip, audio);
    const jumped = s.jumps !== jumps || want !== src;
    if (want !== src) {
      src = want;
      audio.src = want;
    }
    // The app's slider is 0..1 and the engine applies a perceptual curve.
    audio.volume = silent ? 0 : Math.pow(s.volume, 2);
    audio.muted = silent;
    if (jumped) {
      jumps = s.jumps;
      ended = false;
      audio.currentTime = s.positionMs / 1000;
    }
    if (!s.playing) {
      if (!audio.paused) audio.pause();
    } else if (audio.paused && !ended) {
      // Start only on a change: play pressed, a jump, or sound turned back on; never every
      // frame, so a refused or ended clip isn't retried on each tick.
      const unmuted = wasSilent && !silent;
      if (!wasPlaying || jumped || unmuted) {
        // Started again after a pause or a refused start: from where the clock is now, not
        // from where the clip stopped.
        if (!jumped) audio.currentTime = s.positionMs / 1000;
        start(audio);
      }
    }
    wasPlaying = s.playing;
    wasSilent = silent;
  };
  player.subscribe(render);

  // The clock follows what's heard: the clip's own position while it's really sounding
  // (loaded, playing, not seeking, the current song's clip). Before that, and if it never
  // starts (refused, muted by the browser), the clock runs on its own.
  player.followSound(() => {
    const el = audio;
    if (!el || el.paused || el.seeking || el.ended || ended || el.readyState < 3) return null;
    const clip = player.state.song?.clip;
    if (!clip || pick(clip, el) !== src) return null;
    return el.currentTime * 1000;
  });

  // A hidden tab pauses the demo (the clock already stops; the sound must too).
  document.addEventListener("visibilitychange", () => {
    if (document.hidden && player.state.playing) player.pause();
  });

  // The demo scrolled out of view (every player on the page off screen) pauses it.
  if (typeof IntersectionObserver !== "undefined") {
    const shown = new Set<Element>();
    const io = new IntersectionObserver((entries) => {
      for (const e of entries) {
        if (e.isIntersecting) shown.add(e.target);
        else shown.delete(e.target);
      }
      if (!shown.size && player.state.playing && audio) player.pause();
    });
    for (const el of document.querySelectorAll("[data-replica], [data-showcase-panel]")) io.observe(el);
  }
}
