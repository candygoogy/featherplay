# Security

## Reporting a vulnerability

Please report security problems privately, not in a public issue. On GitHub, open the
repository's **Security** tab and choose **Report a vulnerability**, or go straight to
<https://github.com/candygoogy/featherplay/security/advisories/new>. Only you and the
maintainer can see the report.

Say which version you used (Settings, About), what you did and what happened, and how to
reproduce it if you can. Leave out your own cookies, sessions and tokens; a made-up or
expired value shows the problem just as well.

I'll reply in the report. A fix ships in a new release, and the advisory is published with
it.

## What's in scope

- The app, `featherplay.exe`, and the code in this repository.
- Credential storage. featherplay keeps your YouTube Music session, your Last.fm session
  (and your own Last.fm API key, if you set one) and your ListenBrainz token in Windows
  Credential Manager. A way to read them from outside your Windows account, or to make
  featherplay write them somewhere else (a file, a log, a request to the wrong host), is in
  scope.
- The one file a secret goes in. yt-dlp takes cookies as a file, so for each song it looks
  up while you're signed in, featherplay writes the YouTube Music session to a temporary
  `cookies-*.txt` in `%LOCALAPPDATA%\featherplay\tmp` and deletes it when yt-dlp is done
  (`CookieFile` in `crates/streams/src/ytdlp.rs`). If featherplay was killed in between, it
  deletes the leftover the next time it starts signed in. A way to make that file outlive
  the lookup, land somewhere else, or be read by another Windows user is in scope.
- The Edge sign-in's temporary profile. "Use Microsoft Edge" runs Edge with a new profile in
  `%LOCALAPPDATA%\featherplay\cache\signin-*`, keeps only the YouTube Music session from it
  and deletes the folder afterwards; a start deletes any that a crash left behind
  (`apps/featherplay/src/pages/sign_in/edge/profile.rs`). A way to make it keep more than
  that session, or outlive the sign-in, is in scope.
- The local listeners, both off by default and on 127.0.0.1 only. The local API (port 26538)
  answers only requests that carry its token, name the loopback address as their host, and
  don't come from a web page (`crates/integrations/src/local_api/gate.rs`). The Amuse
  endpoint (port 9863) has no token, as Pear Desktop's Amuse plugin has none: it only says
  what's playing, and any app or web page on the PC can read that while it's on. Reaching
  either from another PC, controlling playback through the Amuse endpoint, or using the local
  API without its token is in scope.
- The helpers. featherplay downloads yt-dlp and Deno on the first run and checks each file
  against a SHA256 hash pinned in its code (`crates/streams/src/sidecars.rs`), then lets
  yt-dlp update itself. A way to get featherplay to run a file that wasn't checked is in scope.
- The installer and uninstaller (`packaging/windows/`), including what the uninstaller
  deletes and which processes it closes.

Out of scope:

- Problems in YouTube Music, Google sign-in, WebView2, yt-dlp or Deno themselves. Report
  those to their projects.
- Programs that already run as your Windows user. They can read Credential Manager entries
  like any saved password, and the temporary cookie file too, and featherplay can't prevent
  it.
- The Last.fm API key and secret built into the release. Any desktop Last.fm client has to
  ship its pair inside the program, where anyone can extract it, so they aren't secret. The
  pair belongs to an API account used only by featherplay and can be rotated. If you'd
  rather use your own, set it in Settings, Scrobbling, API settings.

## Supported versions

Only the latest release gets security fixes. Older versions don't; update to the latest one.
