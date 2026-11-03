//! The routes, under `/api/v1/` as Pear Desktop's API Server has them. Reads answer JSON
//! from the latest snapshot; controls go on the command channel the media keys and the tray
//! use, and answer 204. Errors are `{"error": {"code", "message"}}`.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use http_body_util::{BodyExt, Full, LengthLimitError, Limited};
use hyper::body::Incoming;
use hyper::header::{CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, HeaderValue, WWW_AUTHENTICATE};
use hyper::{Method, Request, Response, StatusCode};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::gate::{self, Refusal};
use super::{Shared, events, ws};
use crate::UiCommand;

/// The largest body a control takes: `{"seconds": 12.5}` and the like.
const MAX_BODY: usize = 1024;
/// How long a client has to send a body.
const BODY_TIMEOUT: Duration = Duration::from_secs(5);
/// The longest seek or skip a control takes, a day: no track is longer.
const MAX_SECONDS: f64 = 86_400.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Route {
    Song,
    Volume,
    SetVolume,
    Shuffle,
    ToggleShuffle,
    RepeatMode,
    SwitchRepeat,
    LikeState,
    Play,
    Pause,
    TogglePlay,
    Next,
    Previous,
    SeekTo,
    GoBack,
    GoForward,
    Ws,
}

#[derive(Debug, PartialEq, Eq)]
enum Miss {
    NotFound,
    WrongMethod,
}

fn route(method: &Method, path: &str) -> Result<Route, Miss> {
    use Route::*;
    let name = path.strip_prefix("/api/v1/").ok_or(Miss::NotFound)?;
    let (get, post) = match name {
        "song" | "song-info" => (Some(Song), None),
        "volume" => (Some(Volume), Some(SetVolume)),
        "shuffle" => (Some(Shuffle), Some(ToggleShuffle)),
        "repeat-mode" => (Some(RepeatMode), None),
        "switch-repeat" => (None, Some(SwitchRepeat)),
        "like-state" => (Some(LikeState), None),
        "play" => (None, Some(Play)),
        "pause" => (None, Some(Pause)),
        "toggle-play" => (None, Some(TogglePlay)),
        "next" => (None, Some(Next)),
        "previous" => (None, Some(Previous)),
        "seek-to" => (None, Some(SeekTo)),
        "go-back" => (None, Some(GoBack)),
        "go-forward" => (None, Some(GoForward)),
        "ws" => (Some(Ws), None),
        _ => return Err(Miss::NotFound),
    };
    let found = match *method {
        Method::GET => get,
        Method::POST => post,
        _ => None,
    };
    found.ok_or(Miss::WrongMethod)
}

pub(super) async fn handle(
    req: Request<Incoming>,
    shared: Arc<Shared>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let ws_path = req.uri().path() == "/api/v1/ws";
    let gate = gate::check(
        req.headers(),
        req.uri().query(),
        shared.port,
        &shared.token,
        ws_path,
    );
    if let Err(why) = gate {
        return Ok(refused(why));
    }
    let reply = match route(req.method(), req.uri().path()) {
        Err(Miss::NotFound) => error(StatusCode::NOT_FOUND, "not_found", "No such route."),
        Err(Miss::WrongMethod) => error(
            StatusCode::METHOD_NOT_ALLOWED,
            "method_not_allowed",
            "This route takes another method.",
        ),
        Ok(r) => answer(r, req, &shared).await,
    };
    Ok(reply)
}

async fn answer(route: Route, req: Request<Incoming>, shared: &Shared) -> Response<Full<Bytes>> {
    let now = shared.state.borrow().clone();
    let at = SystemTime::now();
    let command = match route {
        Route::Ws => return ws::upgrade(req, shared),
        Route::Song => {
            return match &now.track {
                Some(t) => ok(events::song(t, &now, at)),
                None => no_content(),
            };
        }
        Route::Volume => return ok(json!({ "state": now.volume, "isMuted": false })),
        Route::Shuffle => return ok(json!({ "state": now.shuffle })),
        Route::RepeatMode => return ok(json!({ "mode": events::repeat(now.repeat) })),
        Route::LikeState => {
            let like = now.track.as_ref().map(|t| events::like(t.like));
            return ok(json!({ "state": like }));
        }
        Route::Play => UiCommand::Play,
        Route::Pause => UiCommand::Pause,
        Route::TogglePlay => UiCommand::PlayPause,
        Route::Next => UiCommand::Next,
        Route::Previous => UiCommand::Previous,
        Route::ToggleShuffle => UiCommand::ToggleShuffle,
        Route::SeekTo | Route::GoBack | Route::GoForward => {
            let Seconds { seconds } = match body(req).await {
                Ok(b) => b,
                Err(bad) => return bad.reply(),
            };
            if !(0.0..=MAX_SECONDS).contains(&seconds) {
                return invalid("`seconds` is a number from 0 to 86400.");
            }
            let by = Duration::from_secs_f64(seconds);
            match route {
                Route::SeekTo => UiCommand::SeekTo(by),
                Route::GoBack => UiCommand::SeekBack(by),
                _ => UiCommand::SeekForward(by),
            }
        }
        Route::SetVolume => {
            let VolumeBody { volume } = match body(req).await {
                Ok(b) => b,
                Err(bad) => return bad.reply(),
            };
            if !(0.0..=100.0).contains(&volume) {
                return invalid("`volume` is a number from 0 to 100.");
            }
            UiCommand::SetVolume(volume.round() as u8)
        }
        Route::SwitchRepeat => {
            let Iteration { iteration } = match body(req).await {
                Ok(b) => b,
                Err(bad) => return bad.reply(),
            };
            if !(0.0..=1000.0).contains(&iteration) || iteration.fract() != 0.0 {
                return invalid("`iteration` is a whole number from 0 to 1000.");
            }
            // Three presses come back round to the same mode.
            UiCommand::CycleRepeat((iteration as u32 % 3) as u8)
        }
    };
    match shared.commands.send(command) {
        Ok(()) => no_content(),
        Err(_) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "closing",
            "featherplay is closing.",
        ),
    }
}

#[derive(Deserialize)]
struct Seconds {
    seconds: f64,
}

#[derive(Deserialize)]
struct VolumeBody {
    volume: f64,
}

#[derive(Deserialize)]
struct Iteration {
    iteration: f64,
}

/// Why a body was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BadBody {
    TooLarge,
    Late,
    Unreadable,
    NotJson,
}

impl BadBody {
    fn reply(self) -> Response<Full<Bytes>> {
        match self {
            BadBody::TooLarge => error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "body_too_large",
                "Bodies are 1 KB at most.",
            ),
            BadBody::Late => error(
                StatusCode::REQUEST_TIMEOUT,
                "timeout",
                "The body didn't arrive in time.",
            ),
            BadBody::Unreadable => error(
                StatusCode::BAD_REQUEST,
                "bad_body",
                "The body couldn't be read.",
            ),
            BadBody::NotJson => error(
                StatusCode::BAD_REQUEST,
                "bad_json",
                "The body isn't the JSON this route takes.",
            ),
        }
    }
}

/// The request's JSON body, small and in time.
async fn body<T: DeserializeOwned>(req: Request<Incoming>) -> Result<T, BadBody> {
    let declared = req
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok()?.parse::<u64>().ok());
    if declared.is_some_and(|n| n > MAX_BODY as u64) {
        return Err(BadBody::TooLarge);
    }
    let read = Limited::new(req.into_body(), MAX_BODY).collect();
    let bytes = match tokio::time::timeout(BODY_TIMEOUT, read).await {
        Err(_) => return Err(BadBody::Late),
        Ok(Err(e)) if e.downcast_ref::<LengthLimitError>().is_some() => {
            return Err(BadBody::TooLarge);
        }
        Ok(Err(_)) => return Err(BadBody::Unreadable),
        Ok(Ok(collected)) => collected.to_bytes(),
    };
    serde_json::from_slice(&bytes).map_err(|_| BadBody::NotJson)
}

fn refused(why: Refusal) -> Response<Full<Bytes>> {
    match why {
        Refusal::Host => error(
            StatusCode::FORBIDDEN,
            "host_not_allowed",
            "Use 127.0.0.1 or localhost with the port.",
        ),
        Refusal::Origin => error(
            StatusCode::FORBIDDEN,
            "origin_not_allowed",
            "Web pages can't use the local API.",
        ),
        Refusal::Token => {
            let mut reply = error(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Send the token from featherplay's Settings as `Authorization: Bearer <token>`.",
            );
            reply
                .headers_mut()
                .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
            reply
        }
    }
}

fn invalid(message: &str) -> Response<Full<Bytes>> {
    error(StatusCode::UNPROCESSABLE_ENTITY, "invalid_body", message)
}

pub(super) fn error(status: StatusCode, code: &str, message: &str) -> Response<Full<Bytes>> {
    let mut reply = json_reply(&json!({ "error": { "code": code, "message": message } }));
    *reply.status_mut() = status;
    reply
}

fn ok(value: Value) -> Response<Full<Bytes>> {
    json_reply(&value)
}

fn json_reply(value: &Value) -> Response<Full<Bytes>> {
    let mut reply = Response::new(Full::new(Bytes::from(value.to_string())));
    let h = reply.headers_mut();
    h.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    h.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(
        hyper::header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    reply
}

fn no_content() -> Response<Full<Bytes>> {
    let mut reply = Response::new(Full::new(Bytes::new()));
    *reply.status_mut() = StatusCode::NO_CONTENT;
    reply
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_take_their_method() {
        assert_eq!(route(&Method::GET, "/api/v1/song"), Ok(Route::Song));
        assert_eq!(route(&Method::GET, "/api/v1/song-info"), Ok(Route::Song));
        assert_eq!(route(&Method::POST, "/api/v1/volume"), Ok(Route::SetVolume));
        assert_eq!(route(&Method::GET, "/api/v1/volume"), Ok(Route::Volume));
        assert_eq!(route(&Method::POST, "/api/v1/song"), Err(Miss::WrongMethod));
        assert_eq!(
            route(&Method::DELETE, "/api/v1/next"),
            Err(Miss::WrongMethod)
        );
    }

    #[test]
    fn other_paths_are_not_found() {
        for path in [
            "/",
            "/api/v1/",
            "/api/v1/song/",
            "/api/v2/song",
            "/swagger",
            "/auth/x",
        ] {
            assert_eq!(route(&Method::GET, path), Err(Miss::NotFound), "{path}");
        }
    }
}
