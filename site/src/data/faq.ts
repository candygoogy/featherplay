// The questions people ask before installing an unofficial player, in three groups. Facts
// from the repo's README; the sign-in answer follows the Windows browser sign-in as it is on
// main since November 2026 (the browser opens plainly, you close it, featherplay reads the
// session afterwards), as Linux has done since 0.4.1.
import { links, repo } from "./links";

/** An answer's paragraphs. Two marks, nothing else: `**…**` the run-in that names a paragraph's
 *  case (set bold), `*…*` the name of a thing the visitor will look for (set in the medium
 *  weight). The words are the README's; the marks only set them. */
export type Faq = { id?: string; q: string; a: string[]; link?: { href: string; text: string } };
export type FaqGroup = { id: string; title: string; items: Faq[] };

export const faqGroups: FaqGroup[] = [
  {
    id: "q-about",
    title: "About featherplay",
    items: [
      {
        id: "official",
        q: "Is featherplay official?",
        a: [
          "No. featherplay is an independent, unofficial player for YouTube Music. It isn’t made by Google or YouTube and isn’t affiliated with them.",
          "Because it’s unofficial, a change on YouTube’s side can break playback until featherplay is updated. If something stops working, update first.",
        ],
      },
      {
        q: "Is it free?",
        a: ["Yes. It’s free and open source under the MIT licence, with no paid tier and no account to create."],
      },
      {
        id: "systems",
        q: "Does it run on Linux or macOS?",
        a: [
          "**Windows is the main one.** featherplay is built on Windows’ own parts there: the media overlay, the taskbar buttons, the notification area and Credential Manager. That’s a lot of what keeps it light.",
          "**There’s also an experimental Linux build:** an AppImage and a .tar.gz, for x86_64 and aarch64, on glibc 2.35 or newer (Ubuntu 22.04, Debian 12, Fedora 36 or later). Automatic tests run it on Ubuntu, Debian, Fedora and Arch, but few people have used it on a real desktop yet, so reports help. There’s no macOS version.",
        ],
        link: { href: links.linux, text: "Linux: the steps and the known limits" },
      },
      {
        q: "What does it sound like?",
        a: [
          "featherplay plays YouTube Music’s Opus stream, about 130 to 140 kbps, where a song has one. It sounds better than AAC at about the same data use. *Settings > Streaming quality* switches to Standard, AAC at 128 kbps.",
        ],
      },
    ],
  },
  {
    id: "q-account",
    title: "Your account",
    items: [
      {
        q: "Do I need an account?",
        a: [
          "No. Without signing in, featherplay searches, opens artists, albums and public playlists, shows Home and Explore, and plays songs with their radio and lyrics.",
          "Sign in for your library, liked songs, history, likes and playlists.",
        ],
      },
      {
        id: "sign-in",
        q: "How does signing in work?",
        a: [
          "**On Windows,** featherplay opens a small sign-in window, *Microsoft Edge WebView2*, with Google’s own sign-in page. You sign in there as you would in a browser. featherplay doesn’t read your password; the code is open. Once you’re in, it keeps the YouTube Music session from that window and closes it. The main window never contains a browser. Windows 11 includes WebView2, and most Windows 10 PCs have it with Edge.",
          "**If you’d rather sign in in a browser you know,** choose *Use Microsoft Edge*, *Use Google Chrome*, *Use Brave* or *Use Vivaldi*. The browser opens as you’d start it yourself, in a temporary profile, with its own address bar, your passkeys and Windows Hello. Sign in, then close the browser. Once it has closed, featherplay opens that profile again without a window, keeps only the YouTube Music session and deletes the profile. If you close the browser before you’ve signed in, nothing is kept; if you leave it open, featherplay closes it after 15 minutes and still keeps a sign-in you finished. Your own profile is never touched.",
          "**Linux** has no sign-in window, so it always signs in this way, with Chrome, Chromium, Firefox, Edge, Brave or Vivaldi, Snap and Flatpak ones too. featherplay reads the session from the closed browser’s temporary profile itself.",
          "**With any other browser,** choose *Paste from any browser*: open music.youtube.com there signed in, press F12, open the Network tab and reload, then right-click the first request, copy it as cURL and paste it into featherplay.",
          "**The session is kept** in *Windows Credential Manager*, or on Linux in your desktop’s keyring (the Secret Service, such as GNOME Keyring or KWallet). While yt-dlp looks up a song’s audio it reads the session from a temporary file, which featherplay deletes as soon as yt-dlp is done. The session gives the same access to your account as a signed-in browser, so treat it like a password; *Sign out* in Settings removes it.",
        ],
      },
      {
        q: "Is my account safe with it?",
        a: [
          "Your YouTube Music session, your Last.fm session and your ListenBrainz token stay on your PC, in Windows Credential Manager, or on Linux in your desktop’s keyring. With no keyring running, Linux keeps a sign-in only until featherplay closes, never in a file. The YouTube Music session also goes into a temporary file while yt-dlp looks up a song, deleted as soon as it’s done. The code is public for anyone to read.",
          "It is still an unofficial client, using YouTube Music in a way Google doesn’t support. If that worries you, sign in with a second account, or listen without signing in.",
        ],
      },
      {
        id: "connects",
        q: "What does it connect to?",
        a: [
          "featherplay sends no telemetry and has no server of its own. It connects to YouTube Music and Google for your music, covers, streams and sign-in, and to GitHub to fetch yt-dlp and Deno in the background on the first run and to update yt-dlp. Only when GitHub can’t be reached, it fetches them from this site and from Deno’s download server instead (on Linux, yt-dlp comes from GitHub only for now).",
          "LRCLIB gets the song’s title, artist, album and length, only while lyrics are on screen, and in Choose other lyrics the words you search for. Google Translate gets the lyric lines on screen only while Translate or Romanize is on (for an English song, only its first eight lines). SponsorBlock, on by default, is asked only about music videos and uploads (songs have nothing to skip), by the first four characters of a hash of the video’s ID, so it doesn’t learn which video you play.",
          "Last.fm and ListenBrainz hear from it only if you connect them, and Discord only if you turn on the status, through the Discord app on your own PC. Google’s or Cloudflare’s DNS over HTTPS is asked only with Look up YouTube directly on, and only for YouTube’s server names.",
        ],
      },
    ],
  },
  {
    id: "q-use",
    title: "Using it",
    items: [
      {
        id: "from-pear",
        q: "I use Pear Desktop. What carries over?",
        a: [
          "Most of Pear’s plugins are settings in featherplay: the scrobbler (Last.fm and ListenBrainz), taskbar media controls, shortcuts and media keys, SponsorBlock, Discord status, synced lyrics and notifications are built in. Its local API uses the routes and WebSocket of Pear’s API Server, so overlays and Stream Deck plugins made for Pear work with it too. featherplay never shows the video, so if you watch the music videos, use YouTube Music itself.",
        ],
        link: { href: links.comingFromPear, text: "Plugin by plugin, in the README" },
      },
      {
        q: "Does it work with stream overlays and Stream Deck?",
        a: [
          "Yes, once you turn on the local API in *Settings > Local API*. It uses the port, routes and WebSocket of Pear Desktop’s API Server, with a token from Settings, so overlays, Stream Deck plugins and scripts on your PC can see what’s playing and control it.",
          "Amuse support, a switch in the same place, answers the Amuse now-playing widget on port 9863. It only reads, and it has no token, so while it’s on, any app on your PC can see what’s playing. Of websites, only the Amuse widget’s page can.",
        ],
      },
      {
        q: "Does it download anything else?",
        a: [
          "Yes, two helpers, in the background on the first run: yt-dlp, which finds the audio stream of the songs featherplay can’t find by itself, and Deno, which yt-dlp needs to do it. Most songs don’t need them: featherplay answers YouTube’s player challenges with the JavaScript engine built into it, so the first song you click doesn’t wait for the download. featherplay fetches both helpers from their GitHub releases, or from this site and Deno’s download server when GitHub can’t be reached (on Linux, yt-dlp from GitHub only), and checks each file against a SHA256 written into its own code; a file that doesn’t match is deleted, not run.",
          "After that it updates yt-dlp at most once a day, and once more the first time a song fails after it starts, because YouTube changes often. Nothing else is downloaded, and never the music itself.",
        ],
      },
      {
        id: "cant-play",
        q: "Some songs won’t play. Why?",
        a: [
          "YouTube refuses some songs where it places you, when they aren’t licensed there. featherplay dims them, skips them in the queue and says so once per queue.",
          "If many songs fail but play in a browser, the usual cause is a DNS service or filter that sends YouTube to its own servers in another country, as NextDNS’s YouTube option and some smart-DNS services do. featherplay may then say “Songs can’t play: DNS reroutes YouTube.” Turn that option off in your DNS service, or turn on *Look up YouTube directly* in *Settings > Playback*: featherplay then asks Google’s or Cloudflare’s DNS over HTTPS for YouTube’s servers only.",
        ],
        link: { href: links.cantPlay, text: "How to check, in the README" },
      },
      {
        id: "uninstall",
        q: "How do I uninstall it?",
        a: [
          "**Installed:** uninstall it from *Settings > Apps*. That removes everything featherplay keeps: settings, cache, the helpers, the sign-in window’s data, its entries in Credential Manager and its entry for starting when you sign in.",
          "**Portable:** sign out and disconnect Last.fm in Settings first, which clears both from Credential Manager, and turn off Start when you sign in to Windows if it’s on. Then delete the exe and its two folders, %APPDATA%\\featherplay and %LOCALAPPDATA%\\featherplay.",
          "**Linux:** turn off Start when you log in if it’s on, then delete the AppImage, or the unpacked .tar.gz and what you installed from it in ~/.local, and the folders ~/.config/featherplay, ~/.cache/featherplay and ~/.local/state/featherplay. Then remove the items whose names end in @featherplay from your keyring.",
        ],
      },
      {
        q: "How do I report a bug?",
        a: ["Open an issue on GitHub. Say what you did, what you expected and what happened, and add the version from *Settings > Account > About featherplay*."],
        link: { href: `${repo}/issues`, text: "Open an issue on GitHub" },
      },
    ],
  },
];
