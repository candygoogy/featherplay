# Privacy

featherplay sends no telemetry and has no server, only a static website. It connects to:

- **YouTube Music and Google**, for your music, covers, streams and sign-in;
- **GitHub**, to download yt-dlp and Deno in the background on the first run and to update
  yt-dlp; **featherplay's site and dl.deno.land** only when GitHub can't be reached on the
  first run (on Linux, yt-dlp comes from GitHub only);
- **GitHub, for featherplay's own updates**, with Check for updates on (the default): once a
  day, and when songs keep failing to play, it asks whether a newer featherplay is out or a
  known playback problem is open, and sends nothing but the app's version. The installed copy
  and the AppImage then download the new version from the release;
- **LRCLIB**, for lyrics, only while lyrics are on screen (desktop lyrics count). It gets the
  song's title, artist, album and length, and in **Choose other lyrics** the words you search
  for;
- **SponsorBlock**, on by default, only for music videos and uploads (songs have nothing to
  skip). It asks by the first four characters of a hash of the video's ID, so the server
  doesn't learn which video you play;
- **Google's or Cloudflare's DNS over HTTPS**, only with Look up YouTube directly on, and only
  for YouTube's server names;
- **Last.fm and ListenBrainz**, only if you connect them;
- **Discord**, only if you turn on the status, through the Discord app on your own PC.

The DNS check in Copy diagnostics asks only your system's own resolver, and the
[local API](local-api.md), off by default, sends nothing anywhere: it listens on 127.0.0.1 for
apps on your PC that have its token. The [Amuse](local-api.md#amuse) endpoint, off by default too, has no
token: while it's on, any app on your PC, and the Amuse widget's page, can see what's playing.
Other websites can't.

## The helpers

The helpers are [yt-dlp](https://github.com/yt-dlp/yt-dlp) and
[Deno](https://github.com/denoland/deno) (yt-dlp's JavaScript runtime), from their GitHub
releases, or when GitHub can't be reached, Deno from its own download server and, on Windows,
yt-dlp from featherplay's site. Each file is checked against a SHA256 written into
featherplay's code, and one that doesn't match is deleted, not run. A song waits for them only
if it needs them before they're there. featherplay then updates yt-dlp at most once a day, and
once more the first time a song fails after it starts,
because YouTube changes often. They live in `%LOCALAPPDATA%\featherplay\sidecars`, or
`~/.cache/featherplay/sidecars` on Linux.

## Where it keeps things

Your YouTube Music session, your Last.fm session, your ListenBrainz token and the local API's
token are kept in Windows Credential Manager, or on Linux in your keyring.

| | Windows | Linux |
|---|---|---|
| Settings, the play queue, scrobbles waiting to be sent, the lyrics' timing and picks per song | `%APPDATA%\featherplay` | `~/.config/featherplay` |
| Covers, the library's pages, the helpers, quick search's last picks (on Windows, the sign-in window's data too) | `%LOCALAPPDATA%\featherplay` | `~/.cache/featherplay` |
| Logs | `%LOCALAPPDATA%\featherplay\logs` | `~/.local/state/featherplay/logs` |

Signing in with your own browser uses a temporary profile in the cache folder, deleted as soon
as the sign-in ends (or at the next start, if featherplay was killed meanwhile); featherplay
keeps only its YouTube Music cookies. The song links looked up in the last session are kept
there until they expire (a few hours), so songs start sooner after a restart; like every
YouTube stream link, each one contains your public IP address.

**Start when you sign in** adds the entry your system runs at sign-in: a `featherplay` value
under `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run` on Windows, or
`~/.config/autostart/dev.candygoogy.featherplay.desktop` on Linux. Turning it off removes it.

## Removing featherplay

**Installed, on Windows.** Uninstalling from Settings > Apps removes everything featherplay
keeps: settings, cache, helpers, the sign-in window's data, its entries in Credential Manager,
and its entry for starting when you sign in.

**The portable exe.** Turn off **Start when you sign in to Windows** (Settings > Window) if
it's on, delete the exe and its two folders, then remove the Windows Credentials whose names
end in `.featherplay` in Credential Manager.

**Linux.** [featherplay on Linux](linux.md) has the steps for the AppImage and for the
`.tar.gz`.
