# featherplay on Linux (experimental)

The Linux build is new. Automatic tests start the AppImage, play a song and let it update
itself on Ubuntu 22.04 and 24.04, Debian stable, the latest Fedora and Arch (x86_64), and the
aarch64 build on Ubuntu 22.04 and 24.04, Debian and Fedora. Few people have used it on a real
desktop yet, so reports help, good or bad
([open an issue](https://github.com/candygoogy/featherplay/issues)). It's the same app as on
Windows, except that there's no sign-in window (you sign in in your browser and close it
when you're done) and no taskbar buttons, the session goes in your keyring, media keys work
through MPRIS, and the body text is in Inter. Under Wayland there are no global hotkeys, no
quick search shortcut and no desktop lyrics (see [Known limits](#known-limits)).

**The AppImage.** Download `featherplay-x86_64.AppImage` (or `featherplay-aarch64.AppImage`)
from the [latest release](https://github.com/candygoogy/featherplay/releases/latest), then:

```sh
chmod +x featherplay-x86_64.AppImage
./featherplay-x86_64.AppImage
```

It needs FUSE's `fusermount`, which most desktops have; without it, start it with
`--appimage-extract-and-run`. It carries the keyboard and X11 libraries a minimal system may
lack, so it starts there too. In a folder you can write to, it updates itself as the Windows
installer does. From 0.4.1 it also carries update information, and each release a `.zsync`
file, so AppImageUpdate and the tools built on it can update it too, downloading only the
parts that changed. To remove it, turn off **Start when you log in** (Settings > Window) if
it's on, delete the file and the three folders listed under [Privacy](privacy.md#where-it-keeps-things), then remove the
items whose names end in `@featherplay` from your keyring (in Seahorse, for example).

**The .tar.gz.** Unpack it and run `./featherplay` in the `featherplay` folder, or install it
for your user, so it shows in your desktop's app list:

```sh
tar -xzf featherplay-x86_64.tar.gz && cd featherplay
install -Dm755 featherplay ~/.local/bin/featherplay && cp -r share/. ~/.local/share/
```

The desktop entry starts `featherplay` from your `PATH` (`~/.local/bin` is on it in most
distributions). This copy tells you of new versions but doesn't install them. To remove it,
delete `~/.local/bin/featherplay` and the `dev.candygoogy.featherplay.*` files under
`~/.local/share`, then the folders and keyring items as for the AppImage.

## What it needs

A desktop install has everything already. A minimal one needs glibc 2.35 or newer, a Wayland
or X11 session, PipeWire or PulseAudio behind ALSA, and these packages, plus the Noto fonts
for song titles in other scripts and for emoji. The AppImage brings its own libxkbcommon,
libxkbcommon-x11, libXcursor and libXi (and the few libraries they use), so it doesn't need
those four; the `.tar.gz` needs all of them:

| | Packages | Fonts for other scripts and emoji |
|---|---|---|
| Ubuntu 24.04, Debian 13 | `libfontconfig1 libfreetype6 libasound2t64 libx11-6 libx11-xcb1 libxcursor1 libxi6 libxcb1 libxkbcommon0 libxkbcommon-x11-0 libwayland-client0 fonts-dejavu-core` | `fonts-noto-core fonts-noto-cjk fonts-noto-color-emoji` |
| Ubuntu 22.04, Debian 12 | the same, with `libasound2` | the same |
| Fedora | `fontconfig freetype alsa-lib libX11 libX11-xcb libXcursor libXi libxcb libxkbcommon libxkbcommon-x11 libwayland-client dejavu-sans-fonts` | `google-noto-sans-cjk-fonts google-noto-color-emoji-fonts google-noto-sans-math-fonts google-noto-sans-symbols-fonts google-noto-sans-arabic-fonts google-noto-sans-hebrew-fonts google-noto-sans-thai-fonts google-noto-sans-devanagari-fonts google-noto-sans-armenian-fonts` |
| Arch | `fontconfig freetype2 alsa-lib libx11 libxcursor libxi libxcb libxkbcommon libxkbcommon-x11 wayland ttf-dejavu` | `noto-fonts noto-fonts-cjk noto-fonts-emoji` |

It needs no OpenSSL: its TLS is built in. With `ca-certificates` installed, it trusts the
certificates your system trusts, a company's own included; without it, it still starts and
uses the Mozilla roots it carries.

## Known limits

- **No tray on stock GNOME**, which shows no tray icons without an extension. Closing the
  window quits, a mini player button (`Ctrl+M`) shows in the player bar instead, and **Start
  in the tray** is **Start minimised** there.
- **No global hotkeys on Wayland**, which doesn't let an app take keys for the whole desktop;
  the setting is hidden there. Media keys still work through MPRIS. Quick search's row says
  it isn't available and gives the command `featherplay --quick-search`: add it as a shortcut
  in your desktop's keyboard settings, and it opens the box in the running featherplay.
- **No desktop lyrics on Wayland**, which leaves where windows sit and what stays on top to
  the desktop; Settings says so. In an X11 session they work.
- **The mini player on Wayland** opens as an ordinary window, where the desktop puts it.
- **Boxes instead of text** when the system has no font for a script or for emoji: install
  the fonts above.
- **Proxies** come from `HTTPS_PROXY` or `ALL_PROXY` only, not the desktop's settings.
- **In WSL**, set `WINIT_X11_SCALE_FACTOR` to Windows' display scale (`2` for 200 %).

**Not yet checked on a real desktop**, only in automatic tests: the tray on KDE and on GNOME
with the AppIndicator extension; notifications; media keys and the desktop's media widgets
(MPRIS); global hotkeys, quick search and desktop lyrics on X11; starting at login; title bars
and scaling on GNOME under Wayland; signing in with the desktop's own browser; the AppImage
updating itself. If you try one, say how it went.

## How light, on Linux

Measured in WSL on the same PC (not a real desktop), featherplay 0.3.0's AppImage
used 85 MB of memory both playing and paused, 0.26 % CPU while playing and 0.001 % while paused,
as a share of the 12 processors WSL had (three runs, the median; the queue played on rather than
repeating one song). It runs as 2 processes: the app and the AppImage's own file server.
