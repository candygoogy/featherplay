//! Home's video tiles are 16:9, and so is their art: decoded 16:9 (a 4:3 thumbnail loses its
//! letterbox), not a square whose middle band the tile would show. Square shelves keep square
//! art. On the offscreen platform with the fixtures and generated covers: nothing is fetched.

use std::path::Path;

use featherplay::snapshot::Scene;
use featherplay::ui::{HomeData, ShelfShape};
use slint::{ComponentHandle, Model};

/// A 640×480 thumbnail for every fixture URL, its picture between letterbox bands.
fn fixture_covers() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("home-video-covers");
    std::fs::create_dir_all(&dir).expect("covers dir");
    let img = image::RgbImage::from_fn(640, 480, |_, y| {
        if !(60..420).contains(&y) {
            image::Rgb([0, 0, 0])
        } else {
            image::Rgb([220, 20, 20])
        }
    });
    img.save(dir.join("thumb.png")).expect("cover");
    // SAFETY: set before the scene starts any other thread.
    unsafe { std::env::set_var("FEATHERPLAY_FIXTURE_COVERS", &dir) };
}

#[test]
fn video_tiles_get_16_9_art_and_card_shelves_square_art() {
    fixture_covers();
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.play(true).expect("player");
    scene.open("home").expect("home");
    scene.settle();

    let mut sizes = Vec::new();
    for shelf in scene.ui.global::<HomeData>().get_shelves().iter() {
        for item in shelf.items.iter() {
            let size = item.cover.size();
            if size.width > 0 {
                sizes.push((shelf.shape, size.width, size.height));
            }
        }
    }
    let videos: Vec<_> = sizes
        .iter()
        .filter(|(shape, ..)| *shape == ShelfShape::Videos)
        .collect();
    assert!(!videos.is_empty(), "no video tile got art: {sizes:?}");
    for (_, w, h) in &videos {
        assert_eq!(
            (*h as f32 / *w as f32 * 16.0).round(),
            9.0,
            "video art isn't 16:9: {w}×{h}"
        );
    }
    let squares = sizes
        .iter()
        .filter(|(shape, ..)| !matches!(shape, ShelfShape::Videos | ShelfShape::Songs))
        .collect::<Vec<_>>();
    assert!(!squares.is_empty(), "no square tile got art: {sizes:?}");
    for (shape, w, h) in squares {
        assert_eq!(w, h, "{shape:?} art isn't square");
    }
}
