//! A long playlist shows its first songs as soon as its first page is read, and the songs of
//! each later page follow them on the page and, after a play started meanwhile, in its
//! queue. On the offscreen platform with fixture data: the recorded mix is three pages.

use std::time::Duration;

use featherplay::controller::{Action, PlayingFrom, QueueAction};
use featherplay::pages::content::{Arriving, TrackList};
use featherplay::router::Page;
use featherplay::snapshot::Scene;
use featherplay::ui::PlaylistData;
use featherplay_core::model::{LikeStatus, PlaylistId, Track, TrackId, VideoType};
use slint::{ComponentHandle, Model};

/// The recorded mix, by the id any mix ("RD…") opens it with.
fn mix() -> Page {
    Page::Playlist {
        id: PlaylistId("RDTMAK5uy_FIXTURE_MIX_100000000000000000000".into()),
    }
}

/// The ids and numbers of the rows on the playlist page.
fn rows(scene: &Scene) -> Vec<(String, String)> {
    let tracks = scene.ui.global::<PlaylistData>().get_tracks();
    tracks
        .iter()
        .map(|r| (r.id.to_string(), r.index.to_string()))
        .collect()
}

#[test]
fn a_mixs_later_pages_follow_its_first_songs_numbered_on() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.ctx.navigate(mix());
    scene.settle();

    let data = scene.ui.global::<PlaylistData>();
    assert_eq!(data.get_title().as_str(), "Archive Mix");
    let shown = rows(&scene);
    let numbers: Vec<&str> = shown.iter().map(|(_, n)| n.as_str()).collect();
    // Five on the first page, three on the second, none on the empty last one.
    assert_eq!(numbers, ["1", "2", "3", "4", "5", "6", "7", "8"]);
    assert_eq!(shown[5].0, "-V817A5o_NM", "the second page's first song");
    assert!(
        data.get_meta().starts_with("8 songs"),
        "the header counts the first page only: {}",
        data.get_meta()
    );
}

#[test]
fn a_playlist_left_while_it_loads_takes_none_of_its_pages() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    // The mix's pages all arrive after the page has moved on.
    scene.ctx.navigate(mix());
    scene.open("home").expect("home");
    scene.settle();

    let data = scene.ui.global::<PlaylistData>();
    assert_eq!(rows(&scene), []);
    assert_eq!(data.get_title().as_str(), "");
}

fn track(id: &str) -> Track {
    Track {
        id: TrackId(id.into()),
        title: format!("Title {id}"),
        artists: Vec::new(),
        album: None,
        duration: Some(Duration::from_secs(200)),
        video_type: VideoType::Song,
        thumbnail: None,
        like: LikeStatus::Indifferent,
        counterpart: None,
    }
}

/// A list of `ids` still arriving, labelled "Mix".
fn arriving_list(scene: &Scene, ids: &[&str]) -> (TrackList, Arriving) {
    let (mut list, _model) = TrackList::new(ids.iter().map(|id| track(id)).collect(), &scene.ctx);
    list.from = PlayingFrom::on_page("Mix", mix());
    let arriving = Arriving::default();
    list.arriving = Some(arriving.clone());
    (list, arriving)
}

/// What the songs that arrive after a play ask of the queue: (their ids, mixed in).
fn extends(actions: &[Action]) -> Vec<(Vec<String>, bool)> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::Queue(QueueAction::Extend { tracks, mix, from }) => {
                assert_eq!(from.label, "Mix");
                Some((tracks.iter().map(|t| t.id.0.clone()).collect(), *mix))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn songs_arriving_after_a_play_join_its_queue_until_another_play() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    let (list, arriving) = arriving_list(&scene, &["a", "b"]);
    scene.take_actions();

    // Nothing played yet: the songs to come are the page's alone.
    assert!(!arriving.join(&scene.ctx, &[track("x")]));

    list.play(&scene.ctx, 1, false);
    assert!(arriving.join(&scene.ctx, &[track("c"), track("d")]));
    let actions = scene.take_actions();
    assert!(matches!(
        &actions[0],
        Action::Queue(QueueAction::PlayFrom { tracks, start: 1, .. }) if tracks.len() == 2
    ));
    assert_eq!(extends(&actions), [(vec!["c".into(), "d".into()], false)]);

    // Shuffle: the songs to come are mixed in.
    list.shuffle(&scene.ctx);
    assert!(arriving.join(&scene.ctx, &[track("e")]));
    assert_eq!(extends(&scene.take_actions()), [(vec!["e".into()], true)]);

    // Another list played since: its queue isn't this one's any more.
    let (other, _) = arriving_list(&scene, &["p", "q"]);
    other.play(&scene.ctx, 0, false);
    assert!(!arriving.join(&scene.ctx, &[track("f")]));
    assert!(extends(&scene.take_actions()).is_empty());
}
