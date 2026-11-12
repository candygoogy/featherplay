use std::sync::mpsc;

use zbus::zvariant::OwnedValue;

use super::*;

fn song(position: u64, playing: bool, speed: f32) -> MediaState {
    MediaState {
        title: "Song & title".into(),
        artist: "Artist".into(),
        album: Some("Album".into()),
        art_url: Some("https://example.com/cover.jpg".into()),
        duration: Some(Duration::from_secs(200)),
        playing,
        position: Duration::from_secs(position),
        speed,
    }
}

#[test]
fn the_position_moves_on_at_the_rate_while_playing_and_stops_at_the_end() {
    let start = Instant::now();
    let mut state = State::default();
    apply(&mut state, song(42, true, 1.5), start);
    let later = start + Duration::from_secs(10);
    assert_eq!(state.position(later), Duration::from_secs(57));
    assert_eq!(state.rate(), 1.5);
    assert_eq!(
        state.position(start + Duration::from_secs(1000)),
        Duration::from_secs(200)
    );
    apply(&mut state, song(57, false, 1.5), later);
    assert_eq!(
        state.position(later + Duration::from_secs(10)),
        Duration::from_secs(57)
    );
}

#[test]
fn an_update_signals_what_changed_and_a_jump_is_seeked() {
    let start = Instant::now();
    let mut state = State::default();
    let first = apply(&mut state, song(0, true, 1.0), start);
    assert!(first.properties.contains(&"Metadata"));
    assert!(first.properties.contains(&"PlaybackStatus"));
    assert_eq!(first.seeked, None);
    // Where it was heading: no jump, nothing changed.
    let t = start + Duration::from_secs(5);
    assert_eq!(apply(&mut state, song(5, true, 1.0), t), Changes::default());
    // Moved to 1:00.
    let jumped = apply(&mut state, song(60, true, 1.0), t);
    assert_eq!(jumped.seeked, Some(Duration::from_secs(60)));
    assert!(jumped.properties.is_empty());
    // A new track is new metadata, not a jump.
    let next = apply(
        &mut state,
        MediaState {
            title: "Next".into(),
            ..song(0, true, 1.0)
        },
        t,
    );
    assert_eq!(next.seeked, None);
    assert!(next.properties.contains(&"Metadata"));
}

#[test]
fn seek_goes_ahead_or_back() {
    let ten = Duration::from_secs(10);
    assert_eq!(seek_by(10_000_000), Some(MediaEvent::SeekForward(ten)));
    assert_eq!(seek_by(-10_000_000), Some(MediaEvent::SeekBack(ten)));
    assert_eq!(seek_by(0), None);
}

/// An uncached proxy on `iface`: each read asks the player, as `playerctl` does.
fn proxy<'a>(bus: &'a Connection, iface: &'a str) -> zbus::blocking::Proxy<'a> {
    let name = format!(
        "org.mpris.MediaPlayer2.featherplay.instance{}",
        std::process::id()
    );
    zbus::blocking::proxy::Builder::<zbus::blocking::Proxy<'_>>::new(bus)
        .destination(name)
        .unwrap()
        .path(PATH)
        .unwrap()
        .interface(iface)
        .unwrap()
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .unwrap()
}

/// The player on the real session bus, read and driven the way the desktop does.
/// `cargo nextest run -p featherplay-platform --lib --run-ignored only -E 'test(live_mpris)'`
#[test]
#[ignore = "needs a session D-Bus"]
fn live_mpris_shows_the_track_and_takes_commands() {
    let (tx, rx) = mpsc::channel();
    let window = Handle::from_raw(raw_window_handle::RawWindowHandle::Xlib(
        raw_window_handle::XlibWindowHandle::new(1),
    ))
    .unwrap();
    let mut controls = MediaControls::attach(window, move |e| {
        let _ = tx.send(e);
    })
    .unwrap();
    controls.update(song(42, true, 1.25));
    let bus = bus::session().unwrap();
    let root = proxy(bus, "org.mpris.MediaPlayer2");
    let player = proxy(bus, PLAYER);
    assert_eq!(
        root.get_property::<String>("DesktopEntry").unwrap(),
        "dev.candygoogy.featherplay"
    );
    assert_eq!(
        player.get_property::<String>("PlaybackStatus").unwrap(),
        "Playing"
    );
    assert_eq!(player.get_property::<f64>("Rate").unwrap(), 1.25);
    let metadata: HashMap<String, OwnedValue> = player.get_property("Metadata").unwrap();
    let text = |key: &str| String::try_from(metadata[key].clone()).unwrap();
    assert_eq!(text("xesam:title"), "Song & title");
    assert_eq!(text("xesam:album"), "Album");
    assert_eq!(text("mpris:artUrl"), "https://example.com/cover.jpg");
    assert_eq!(
        i64::try_from(metadata["mpris:length"].clone()).unwrap(),
        200_000_000
    );
    let position = player.get_property::<i64>("Position").unwrap();
    assert!((42_000_000..45_000_000).contains(&position), "{position}");

    let track =
        zbus::zvariant::OwnedObjectPath::try_from(metadata["mpris:trackid"].clone()).unwrap();
    player.call_method("PlayPause", &()).unwrap();
    player.call_method("Next", &()).unwrap();
    player
        .call_method("SetPosition", &(track.into_inner(), 60_000_000i64))
        .unwrap();
    player.call_method("Seek", &(-5_000_000i64,)).unwrap();
    // Play while playing changes nothing.
    player.call_method("Play", &()).unwrap();
    let wait = Duration::from_secs(2);
    assert_eq!(rx.recv_timeout(wait), Ok(MediaEvent::PlayPause));
    assert_eq!(rx.recv_timeout(wait), Ok(MediaEvent::Next));
    assert_eq!(
        rx.recv_timeout(wait),
        Ok(MediaEvent::SeekTo(Duration::from_secs(60)))
    );
    assert_eq!(
        rx.recv_timeout(wait),
        Ok(MediaEvent::SeekBack(Duration::from_secs(5)))
    );
    assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
    drop(controls);
}
