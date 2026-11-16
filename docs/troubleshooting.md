# Troubleshooting

If playback breaks, update first: YouTube changes things, and a new version is usually the
fix. Two entries are in the README, because most people meet them there:
["Windows protected your PC"](../README.md#windows-protected-your-pc) and
[many songs "can't play"](../README.md#many-songs-cant-play-but-they-play-in-a-browser-or-in-pear).
The questions people ask before installing are answered on
[the website](https://featherplay.candygoogy.dev/#questions).

## Playback stopped working

Update featherplay first. If the latest version fails too,
[open an issue](https://github.com/candygoogy/featherplay/issues) with the text from
**Copy diagnostics**.

## Signing in doesn't work

- **"Not available on this PC" for a browser:** a policy on your PC turns off the
  connection featherplay uses (`RemoteDebuggingAllowed`). **Why?** explains, and another
  browser that works is offered instead.
- **On Linux, a browser isn't offered:** Snap and Flatpak browsers can't be used, because
  their sandbox keeps them out of the temporary profile (Ubuntu's Chromium is a Snap). Install
  one from a `.deb` or `.rpm`, such as Google Chrome, Microsoft Edge, Brave or Vivaldi, or
  paste a cookie header.
- **Nothing happens once you've signed in in a browser:** close the browser window
  featherplay opened. featherplay reads the session only once that browser has closed.
- **On Linux, "Signed in for this run only":** no keyring is running to keep the session, so
  featherplay holds it in memory until it quits and never writes it to a file. Start GNOME
  Keyring, KWallet or another Secret Service, then sign in again.

## Boxes instead of text, or a tiny window (Linux)

Boxes mean the system has no font for a script or for emoji; featherplay's log names the
packages it missed, and [What it needs](linux.md#what-it-needs) lists the Noto fonts to install. In
WSL the window opens tiny because WSL doesn't pass on Windows' display scale: set
`WINIT_X11_SCALE_FACTOR` to that scale, for example `WINIT_X11_SCALE_FACTOR=2` for 200 %.
Under WSLg the first song can also stay silent for several seconds, which is WSLg's sound.

## How do I report a bug?

[Open an issue](https://github.com/candygoogy/featherplay/issues) with what you did, what you
expected and what happened, and the version from the About page (Settings > Account > About
featherplay). **Copy diagnostics**, on
the About page, copies the versions, which services are on, the DNS check and the last
playback errors as plain text. featherplay also keeps a log, with a `crash-….txt` file for
each crash (folders under [Privacy](privacy.md#where-it-keeps-things)); **Open log folder** opens it. Cookies, tokens
and keys are masked in both, and neither names your account.

## Behind a proxy

Behind a proxy, featherplay and its helpers go through the one Windows is set to use
(Settings > Network > Proxy): the one "Automatically detect settings" finds, else the setup
script's, else the proxy set by hand. `HTTPS_PROXY` comes before all three. On Linux,
featherplay follows `HTTPS_PROXY` or `ALL_PROXY`.
