# The local API and Amuse

featherplay can answer two kinds of local requests, both off by default and both on this PC
only: the local API, which follows Pear Desktop's API Server, and the endpoint Pear's Amuse
plugin offers for the Amuse widget. Turn them on in Settings > Local API.

## Local API

For stream overlays, Stream Deck plugins and scripts on your PC: what's playing, and the
controls. Turn on **API server** in Settings > Local API. featherplay then listens on
`http://127.0.0.1:26538`, the port of Pear Desktop's API Server (change it in Settings).

Every request needs the token shown there. **Copy** puts it on the clipboard, kept out of the
clipboard history. **Regenerate** replaces it and disconnects the apps using the old one.

```sh
curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:26538/api/v1/song
curl -X POST -H "Authorization: Bearer $TOKEN" http://127.0.0.1:26538/api/v1/toggle-play
curl -X POST -H "Authorization: Bearer $TOKEN" -d '{"seconds": 30}' http://127.0.0.1:26538/api/v1/seek-to
```

### Routes

The routes are under `/api/v1/`, as in Pear's API Server:

| Route | What it does |
|---|---|
| `GET song` (or `song-info`) | The current song: `title`, `artist`, `album`, `imageSrc`, `videoId`, `songDuration`, `elapsedSeconds`, `isPaused`. 204 when nothing is loaded. |
| `GET volume`, `shuffle`, `repeat-mode`, `like-state` | The volume (0 to 100), shuffle, repeat (`NONE`, `ALL`, `ONE`) and the song's like. |
| `POST play`, `pause`, `toggle-play`, `next`, `previous` | The transport. |
| `POST seek-to`, `go-back`, `go-forward` with `{"seconds": 30}` | Seek to a time, or skip back or forward. |
| `POST volume` with `{"volume": 50}` | Set the volume. |
| `POST shuffle`, `POST switch-repeat` with `{"iteration": 1}` | Turn shuffle on or off; press Repeat that many times. |
| `GET ws?token=…` | A WebSocket: `PLAYER_INFO` first, then `VIDEO_CHANGED`, `PLAYER_STATE_CHANGED`, `POSITION_CHANGED`, `VOLUME_CHANGED`, `REPEAT_CHANGED` and `SHUFFLE_CHANGED` as they happen. |

Messages go out only when something changes. A position comes in whole seconds with a
`timestamp`, the Unix time in milliseconds when the song was at that second, so a client
moves the time on by itself while `isPlaying`.

### How it differs from Pear's

- One token from Settings instead of Pear's `/auth/{id}` prompt. In a tool made for Pear,
  paste the token where it asks for one; a tool that only knows the prompt won't connect.
- No queue, search, like, mute, fullscreen or Swagger routes.
- `muted` is always `false`, and `views` is 0.

### Who can reach it

Only this PC. It listens on 127.0.0.1 (and ::1), and refuses requests whose `Host` isn't that
address and port, and requests from web pages (an `Origin` other than `null` or `file://`).
It sends no CORS headers, so a page in a browser can't read it. A browser overlay needs a
helper on the PC that talks to the API for it.

## Amuse

For 6K Labs' Amuse now-playing widget, turn on **Amuse** in Settings > Local API. featherplay
then answers on `http://127.0.0.1:9863` as Pear's Amuse plugin does: `GET /query` (or `/api`)
returns the song.

```json
{ "player": { "hasSong": true, "isPaused": false, "seekbarCurrentPosition": 42 },
  "track": { "title": "…", "author": "…", "duration": 215, "cover": "https://…",
             "url": "https://music.youtube.com/watch?v=…", "id": "…", "isAdvertisement": false } }
```

Times are whole seconds. It only reads: there are no controls.

Like Pear's plugin, it needs no token, so while it's on, any app on this PC can see what's
playing. Unlike Pear's, it answers only one web page: the Amuse widget, which runs on
`https://6klabs.com` and reads `http://localhost:9863/query` from your browser source. Other
websites get a 403 with no CORS headers, so they can't read it. Requests whose `Host` isn't
127.0.0.1, localhost or [::1] with the port are refused too, so a page can't reach it through
DNS rebinding. That also rules out a home-made overlay in a local file or on another site,
which Pear's plugin answers; such an overlay can use the [local API](#local-api) through a
helper instead.
