featherplay for Linux (experimental)
====================================

featherplay is a light native player for YouTube Music: https://featherplay.candygoogy.dev/
This archive holds the same program as featherplay-<arch>.AppImage, as plain files.

  featherplay       the program; run it from here, or install it as below
  share/            its desktop entry, icons and AppStream metadata
  LICENSE           featherplay's licence (MIT)
  THIRD_PARTY.md    the licences of the code it includes

Run it where it is:

  ./featherplay

Or install it for your user, so it shows in your desktop's app list:

  install -Dm755 featherplay ~/.local/bin/featherplay
  cp -r share/. ~/.local/share/

The desktop entry starts `featherplay` from your PATH; ~/.local/bin is on it in most
distributions. To remove it, delete ~/.local/bin/featherplay and the files named
dev.candygoogy.featherplay.* under ~/.local/share.

Updates: this copy tells you when a new version is out, but doesn't install it. The AppImage
updates itself.

What it needs: glibc 2.35 or newer (Ubuntu 22.04, Debian 12, Fedora 36 or newer),
fontconfig, FreeType and ALSA (with PipeWire or PulseAudio behind it, as desktops have), and
a Wayland or X11 session (libwayland-client, libxkbcommon, or libX11 and libxcb), and the
system's CA certificates (ca-certificates on Debian, Ubuntu, Fedora and Arch). A desktop
distribution has all of these. Its TLS is built in (no OpenSSL needed); it trusts the
certificates your system trusts, and with none installed, the Mozilla roots it carries.
Settings go in ~/.config/featherplay, the cache in ~/.cache/featherplay.
