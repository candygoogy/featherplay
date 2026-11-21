# Signing in

You don't need an account to listen. Signed out, featherplay searches, opens artists, albums
and public playlists, shows Home and Explore, and plays songs with their radio and lyrics.
Your library, liked songs, history, likes and playlists need your account. There are three
ways in:

- **Sign in with Google**, the default on Windows, opens Google's own sign-in page in a small
  Microsoft Edge WebView2 window, which Windows 11 includes and most Windows 10 PCs have with
  Edge. Its address bar comes from WebView2, not from the page, and reads "Google's page" on
  Google's sign-in pages. **Switch Google account** always uses it, because it remembers your
  accounts. Linux has no such window.
- **Use Microsoft Edge**, **Use Google Chrome**, **Use Brave** or **Use Vivaldi** (on Linux,
  Chromium and Firefox too) opens that browser itself, with its own address bar, your passkeys
  and Windows Hello, in a temporary profile, never your own. **Close the browser once you're
  signed in.** While Google's page is open it's just the browser, as you'd start it yourself.
  Once you've closed it, featherplay reads the cookies it kept. On Windows it starts the
  browser again on the same profile, headless (no window) and on a blank page, and talks to it
  over a private pipe, not a network port: it can only ask for that profile's cookies and for
  the browser to close. On Linux it reads them from the profile's cookie file itself; nothing
  talks to the browser. Either way it keeps the YouTube Music session, drops every other
  cookie and deletes the profile. If you close it before you've signed in, nothing is kept; if
  you leave it open, featherplay closes it after 15 minutes and still reads the profile, so a
  sign-in you finished isn't lost. The option offers the browser you used last, else your
  default one, else the first installed (**Change** lists the others). On Linux a Snap or
  Flatpak browser works too: its temporary profile goes in the folder its sandbox may use.
  Opera isn't offered, as it starts through a launcher, nor are Firefox's forks (LibreWolf,
  Floorp, Zen) for now.
- **Paste a cookie header**, for advanced users, takes a `Cookie` header copied from a
  signed-in browser. It also takes what's easier to copy: a whole request from the browser's
  developer tools (Network, right-click a request to `music.youtube.com`, **Copy as cURL**,
  bash or cmd, or **Copy as PowerShell**, in Chrome, Edge, Brave or Firefox), or the
  contents of a `cookies.txt` file, as cookie-export extensions and yt-dlp's `--cookies`
  write it; featherplay keeps only the YouTube cookies that haven't expired. It can stop
  working after a while, because the browser keeps renewing its own copy.

featherplay never reads your password, and the main window never contains a browser. It keeps
the session in Windows Credential Manager, or on Linux in your desktop's keyring; the session
gives the same access to your account as a signed-in browser, so treat it like a password.
**Sign out**, in Settings > Account or the account menu, removes it. While yt-dlp looks up a
song, featherplay hands it the session in a temporary file in the cache folder, deleted when
yt-dlp is done (or at the next start, if featherplay was killed first).

## Is my account safe with it?

The session stays on your PC, in Credential Manager or your keyring, and the code is public.
It's still an unofficial client using YouTube Music in a way Google doesn't support; if that
worries you, sign in with a second account. To keep your password out of featherplay's
windows entirely, choose **Use Microsoft Edge** (or another browser) and sign in there.

## When it doesn't work

[Troubleshooting](troubleshooting.md#signing-in-doesnt-work) covers a browser marked
"Not available on this PC", a browser that isn't offered on Linux, a sign-in that
isn't picked up, and "Signed in for this run only".
