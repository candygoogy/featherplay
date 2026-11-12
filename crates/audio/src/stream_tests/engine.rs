//! The engine end to end, on a null output: load, play, and the `Started` event.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::media::SHORT_READ_TIMEOUT;
use super::{engine_tmp, fixture, serve, src, wait_event};

#[test]
fn engine_sounds_long_before_the_download_ends() {
    let data = fixture("tone-10s-frag.m4a");
    let server = serve(data, 16 << 10, Duration::ZERO);
    let engine = crate::AudioEngine::new_null(engine_tmp());
    let events = engine.events();
    let t = Instant::now();
    engine.load(src(&server.url), Duration::ZERO);
    engine.play();
    wait_event(&events, Duration::from_secs(3), crate::AudioEvent::Started);
    let started = t.elapsed();
    assert!(
        started < Duration::from_millis(1500),
        "Started after {started:?}"
    );
    std::thread::sleep(Duration::from_millis(500));
    let at = engine.position();
    assert!(at >= Duration::from_millis(300), "position {at:?}");
}

#[test]
fn engine_starts_a_prefetched_stream_without_the_network() {
    let data = fixture("tone-10s-frag.m4a");
    // Every request takes 1.5 s to answer.
    let server = serve(data, 1 << 20, Duration::from_millis(1500));
    let engine = crate::AudioEngine::new_null(engine_tmp());
    let events = engine.events();
    engine.prefetch(&src(&server.url));
    std::thread::sleep(Duration::from_millis(2500));
    let t = Instant::now();
    engine.load(src(&server.url), Duration::ZERO);
    engine.play();
    wait_event(&events, Duration::from_secs(3), crate::AudioEvent::Started);
    let started = t.elapsed();
    assert!(
        started < Duration::from_millis(300),
        "Started after {started:?}"
    );
    assert_eq!(
        server.log.starts.lock().unwrap().len(),
        1,
        "one request, the prefetch"
    );
}

/// A click on a song resolved just before it: its head is on its way. The load waits for
/// that response instead of sending a second request, which would start later and come
/// later (on a second connection, over the network).
#[test]
fn engine_load_takes_the_head_that_is_on_its_way() {
    let data = fixture("tone-10s-frag.m4a");
    // Every request takes 600 ms to answer.
    let server = serve(data, 1 << 20, Duration::from_millis(600));
    let engine = crate::AudioEngine::new_null(engine_tmp());
    let events = engine.events();
    engine.prefetch(&src(&server.url));
    std::thread::sleep(Duration::from_millis(250));
    let t = Instant::now();
    engine.load(src(&server.url), Duration::ZERO);
    engine.play();
    wait_event(&events, Duration::from_secs(3), crate::AudioEvent::Started);
    let started = t.elapsed();
    let starts = server.log.starts.lock().unwrap().clone();
    assert_eq!(starts, [0], "one request for the start, the prefetch's");
    // The head comes ~350 ms after the load; a request of its own would take 600 ms.
    assert!(
        started < Duration::from_millis(550),
        "Started after {started:?}"
    );
}

/// On a slow link the head takes seconds to come in whole: the load takes its request over
/// and plays from its first bytes, as a request of its own would.
#[test]
fn engine_load_on_a_slow_link_takes_the_head_request_over() {
    let data = fixture("tone-10s-frag.m4a");
    // ~5 s for the whole file.
    let server = serve(data, 16 << 10, Duration::ZERO);
    let engine = crate::AudioEngine::new_null(engine_tmp());
    let events = engine.events();
    engine.prefetch(&src(&server.url));
    std::thread::sleep(Duration::from_millis(250));
    let t = Instant::now();
    engine.load(src(&server.url), Duration::ZERO);
    engine.play();
    wait_event(&events, Duration::from_secs(3), crate::AudioEvent::Started);
    let started = t.elapsed();
    let starts = server.log.starts.lock().unwrap().clone();
    assert_eq!(starts, [0], "one request for the start, the prefetch's");
    assert!(
        started < Duration::from_millis(500),
        "Started after {started:?}"
    );
}

/// A head's answer that comes late still wins if it beats the request the load then sends
/// of its own: the load never comes later than it would have without the head.
#[test]
fn engine_load_takes_a_late_head_if_it_answers_first() {
    let data = fixture("tone-10s-frag.m4a");
    // Every request takes 1.3 s to answer: longer than a load waits for the head's.
    let server = serve(data, 1 << 20, Duration::from_millis(1300));
    let engine = crate::AudioEngine::new_null(engine_tmp());
    let events = engine.events();
    engine.prefetch(&src(&server.url));
    std::thread::sleep(Duration::from_millis(100));
    let t = Instant::now();
    engine.load(src(&server.url), Duration::ZERO);
    engine.play();
    wait_event(&events, Duration::from_secs(4), crate::AudioEvent::Started);
    let started = t.elapsed();
    // The head answers ~1.2 s after the load; a request of its own would answer later.
    assert!(
        started < Duration::from_millis(1600),
        "Started after {started:?}"
    );
}

/// A server that keeps its connections open between requests, as the stream servers do,
/// and counts them.
struct KeepAlive {
    url: String,
    connections: Arc<AtomicU32>,
    /// The paths asked for, in the order they were answered.
    answered: Arc<Mutex<Vec<String>>>,
}

impl KeepAlive {
    /// Waits until `n` requests are answered, and a moment more for the client to take
    /// the last answer in.
    fn wait_answered(&self, n: usize) {
        let t = Instant::now();
        while self.answered.lock().unwrap().len() < n {
            assert!(
                t.elapsed() < Duration::from_secs(5),
                "answer {n} never came"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Serves `data` to `Range` requests and an empty 204 to any other, as the stream servers'
/// `/generate_204`, each after `delay`.
fn serve_keep_alive(data: Vec<u8>, delay: Duration) -> KeepAlive {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/track.m4a", listener.local_addr().unwrap());
    let connections = Arc::new(AtomicU32::new(0));
    let answered = Arc::new(Mutex::new(Vec::new()));
    let data = Arc::new(data);
    let (count, log) = (connections.clone(), answered.clone());
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(conn) = conn else { return };
            count.fetch_add(1, Ordering::SeqCst);
            let (data, log) = (data.clone(), log.clone());
            std::thread::spawn(move || keep_answering(conn, &data, &log, delay));
        }
    });
    KeepAlive {
        url,
        connections,
        answered,
    }
}

fn keep_answering(mut conn: TcpStream, data: &[u8], log: &Mutex<Vec<String>>, delay: Duration) {
    let mut reader = BufReader::new(conn.try_clone().unwrap());
    loop {
        let mut path = None;
        let mut range = None;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                // The client closed the connection.
                return;
            }
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if path.is_none() {
                // `GET /track.m4a HTTP/1.1`
                path = line.split(' ').nth(1).map(str::to_owned);
            } else if let Some(v) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                let (a, b) = v.split_once('-').unwrap();
                range = Some((a.parse::<usize>().unwrap(), b.parse::<usize>().unwrap()));
            }
        }
        std::thread::sleep(delay);
        log.lock().unwrap().push(path.unwrap_or_default());
        let sent = match range {
            Some((a, b)) => {
                let b = b.min(data.len() - 1);
                let head = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Type: audio/mp4\r\n\
                     Content-Length: {}\r\nContent-Range: bytes {a}-{b}/{}\r\n\r\n",
                    b - a + 1,
                    data.len()
                );
                conn.write_all(head.as_bytes())
                    .and_then(|()| conn.write_all(&data[a..=b]))
            }
            None => conn.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n"),
        };
        if sent.is_err() {
            return;
        }
    }
}

/// A click on a song that isn't resolved yet: while its resolve runs, the app has the
/// engine open a connection to the server the stream will likely come from. The stream's
/// first request then goes over it instead of opening its own (~100 ms over TLS).
#[test]
fn engine_load_uses_the_connection_opened_ahead() {
    let server = serve_keep_alive(fixture("tone-10s-frag.m4a"), Duration::ZERO);
    let engine = crate::AudioEngine::new_null(engine_tmp());
    let events = engine.events();
    // The server alone, as the app names it.
    engine.connect(server.url.trim_end_matches("/track.m4a"));
    server.wait_answered(1);
    engine.load(src(&server.url), Duration::ZERO);
    engine.play();
    wait_event(&events, Duration::from_secs(3), crate::AudioEvent::Started);
    assert_eq!(
        *server.answered.lock().unwrap(),
        ["/generate_204", "/track.m4a"]
    );
    assert_eq!(
        server.connections.load(Ordering::SeqCst),
        1,
        "the stream's request opened no connection"
    );
}

/// Clicks in a row each ask for a connection: the one on its way or waiting in the pool
/// serves them all, and no second one opens.
#[test]
fn engine_opens_no_second_connection_ahead() {
    // Each answer takes 300 ms, so the second call comes while the first is on its way.
    let server = serve_keep_alive(Vec::new(), Duration::from_millis(300));
    let engine = crate::AudioEngine::new_null(engine_tmp());
    engine.connect(&server.url);
    engine.connect(&server.url);
    server.wait_answered(1);
    // The connection waits in the pool now: the next call's request goes over it.
    engine.connect(&server.url);
    server.wait_answered(2);
    assert_eq!(
        server.answered.lock().unwrap().len(),
        2,
        "the second call sent nothing"
    );
    assert_eq!(server.connections.load(Ordering::SeqCst), 1);
}

#[test]
fn engine_starts_a_preloaded_stream_at_once() {
    let data = fixture("tone-10s-frag.m4a");
    let server = serve(data, 1 << 20, Duration::from_millis(1000));
    let engine = crate::AudioEngine::new_null(engine_tmp());
    let events = engine.events();
    engine.preload(src(&server.url));
    std::thread::sleep(Duration::from_millis(2000));
    let t = Instant::now();
    engine.load(src(&server.url), Duration::ZERO);
    engine.play();
    wait_event(&events, Duration::from_secs(3), crate::AudioEvent::Started);
    let started = t.elapsed();
    assert!(
        started < Duration::from_millis(300),
        "Started after {started:?}"
    );
}

/// A preload whose URL is refused says so, so the app can resolve it again before Play
/// instead of after it (2026-11-05: the restored track's URL, refused once Play was pressed).
#[test]
fn engine_reports_a_refused_preload() {
    let server = serve(fixture("tone-10s-frag.m4a"), 1 << 20, Duration::ZERO);
    server.log.refuse.store(u32::MAX, Ordering::SeqCst);
    let engine = crate::AudioEngine::new_null(engine_tmp());
    let events = engine.events();
    let url = format!("{}?c=WEB_EMBEDDED_PLAYER", server.url);
    engine.preload(src(&url));
    let event = events.recv_timeout(Duration::from_secs(3));
    assert_eq!(event, Ok(crate::AudioEvent::PreloadRefused(url)));
}

/// A track resolved ahead gets its head fetched: that fetch is the probe that finds a
/// refused URL before the track is played.
#[test]
fn engine_reports_a_refused_prefetch() {
    let server = serve(fixture("tone-10s-frag.m4a"), 1 << 20, Duration::ZERO);
    server.log.refuse.store(u32::MAX, Ordering::SeqCst);
    let engine = crate::AudioEngine::new_null(engine_tmp());
    let events = engine.events();
    let url = format!("{}?c=WEB_EMBEDDED_PLAYER", server.url);
    engine.prefetch(&src(&url));
    let event = events.recv_timeout(Duration::from_secs(3));
    assert_eq!(event, Ok(crate::AudioEvent::PreloadRefused(url)));
}

#[test]
fn engine_seek_into_the_undownloaded_part_plays_on() {
    let data = fixture("tone-10s-frag.m4a");
    // ~2.6 s for the whole file. A target this close is waited for (a jump costs more).
    let server = serve(data, 32 << 10, Duration::ZERO);
    let engine = crate::AudioEngine::new_null(engine_tmp());
    let events = engine.events();
    engine.load(src(&server.url), Duration::ZERO);
    engine.play();
    wait_event(&events, Duration::from_secs(3), crate::AudioEvent::Started);
    engine.seek(Duration::from_secs(8));
    assert_eq!(engine.position(), Duration::from_secs(8));
    // The stall shows as buffering, then the last 2 s play to the end.
    wait_event(
        &events,
        Duration::from_secs(3),
        crate::AudioEvent::Buffering(true),
    );
    let ended = wait_event(&events, Duration::from_secs(7), crate::AudioEvent::Ended);
    assert!(
        ended > Duration::from_millis(1500),
        "Ended {ended:?} after the seek"
    );
}

#[test]
fn engine_reports_a_stream_that_stays_stalled() {
    let data = fixture("tone-10s-frag.m4a");
    let half = data.len() as u64 / 2;
    let server = serve(data, 1 << 20, Duration::ZERO);
    server.log.stall.store(u32::MAX, Ordering::SeqCst);
    server.log.stall_at.store(half, Ordering::SeqCst);
    let engine = crate::AudioEngine::new_null_with_read_timeout(engine_tmp(), SHORT_READ_TIMEOUT);
    let events = engine.events();
    engine.load(src(&server.url), Duration::ZERO);
    engine.play();
    let t = Instant::now();
    let limit = Duration::from_secs(20);
    loop {
        match events.recv_timeout(limit.saturating_sub(t.elapsed())) {
            Ok(crate::AudioEvent::Error(_)) => break,
            Ok(crate::AudioEvent::Ended) => panic!("half a file ended as if whole"),
            Ok(_) => {}
            Err(_) => panic!("no error within {limit:?}: the stalled stream hung"),
        }
    }
}
