# Security

## Reporting a vulnerability

Please report security problems privately, not in a public issue. On GitHub, open the
repository's **Security** tab and choose **Report a vulnerability**, or go straight to
<https://github.com/candygoogy/featherplay/security/advisories/new>. Only you and the
maintainer can see the report.

Say which version you used (Settings > About), what you did and what happened, and how to
reproduce it if you can. Leave out your own cookies, sessions and tokens; a made-up or
expired value shows the problem just as well.

I'll reply in the report. A fix ships in a new release, and the advisory is published with
it.

## What's in scope

- The app, `featherplay.exe`, and the code in this repository.
- Credential storage. featherplay keeps your YouTube Music session, your Last.fm session
  (and your own Last.fm API key, if you set one), your ListenBrainz token and the local API's
  token in Windows Credential Manager. A way to read them from outside your Windows account, or to make
  featherplay write them somewhere else (a file, a log, a request to the wrong host), is in
  scope.
- The one file a secret goes in. yt-dlp takes cookies as a file, so for each song it looks
  up while you're signed in, featherplay writes the YouTube Music session to a temporary
  `cookies-*.txt` in `%LOCALAPPDATA%\featherplay\tmp` and deletes it when yt-dlp is done
  (`CookieFile` in `crates/streams/src/ytdlp.rs`). If featherplay was killed in between, it
  deletes the leftover the next time it starts signed in. A way to make that file outlive
  the lookup, land somewhere else, or be read by another Windows user is in scope.
- The Edge sign-in's temporary profile. "Use Microsoft Edge" runs Edge with a new profile in
  `%LOCALAPPDATA%\featherplay\signin-*`, keeps only the YouTube Music session from it
  and deletes the folder afterwards; a start deletes any that a crash left behind
  (`apps/featherplay/src/pages/sign_in/browser/profile.rs`). A way to make it keep more than
  that session, or outlive the sign-in, is in scope.
- The local listeners, both off by default and on 127.0.0.1 only. The local API (port 26538)
  answers only requests that carry its token, name the loopback address as their host, and
  don't come from a web page (`crates/integrations/src/local_api/gate.rs`). The Amuse
  endpoint (port 9863) has no token, as Pear Desktop's Amuse plugin has none: it only says
  what's playing, and any app on the PC can read that while it's on. It checks the `Host` the
  same way, and of web pages it answers only the Amuse widget's origin, `https://6klabs.com`,
  matched exactly; any other `Origin` gets a 403 with no CORS headers
  (`crates/integrations/src/local_api/amuse.rs`). Reaching either from another PC, reading
  the Amuse endpoint from any other web page, controlling playback through it, or using the
  local API without its token is in scope.
- The helpers. featherplay downloads yt-dlp and Deno on the first run and checks each file
  against a SHA256 hash pinned in its code (`crates/streams/src/sidecars.rs`), then lets
  yt-dlp update itself. A way to get featherplay to run a file that wasn't checked is in scope.
- Automatic updates. The installed copy downloads a newer release's setup only from the
  project's GitHub releases, over HTTPS, and only if the release's `SHA256SUMS.txt` carries
  a minisign signature (`SHA256SUMS.txt.minisig`) by one of the two public keys built into
  featherplay (`apps/featherplay/keys/`), with the trusted comment `featherplay vX.Y.Z` for
  that release. Then the setup must match its SHA256 in that file, and the version must be
  newer than the one running. The private keys never reach GitHub: the maintainer signs
  each release by hand on their own PC. A missing, invalid or mismatched signature means no
  download, and About says the update couldn't be verified; there's no fallback to an
  unsigned file (`apps/featherplay/src/updates/install.rs`). A way to make featherplay
  install a file that's unsigned, signed for another release, or doesn't match its
  checksum, or an older version, is in scope. Versions up to 0.1.2 check only the SHA256.
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
  rather use your own, set it in Settings > Scrobbling > API settings.

## Supported versions

Only the latest release gets security fixes. If you run an older version, update to the
latest one first.
