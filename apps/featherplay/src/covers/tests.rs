use super::*;

/// A thread that panicked while updating the index may have left its total off: the
/// next use that needs it (a write; a read needs none) scans the folder again rather than
/// giving up on the disk cache.
#[test]
fn a_poisoned_index_is_scanned_again() {
    let Some(dir) = crate::testing::scratch("covers-poison") else {
        return;
    };
    std::fs::write(dir.join("a.img"), b"abc").unwrap();
    let cache = DiskCache::new(dir.clone());
    std::thread::scope(|s| {
        let poisoned = s
            .spawn(|| {
                let mut index = cache.index.lock().unwrap();
                *index = Some(Index {
                    total: 999,
                    ..Index::default()
                });
                panic!("poisoning the index on purpose");
            })
            .join();
        assert!(poisoned.is_err());
    });

    let read = cache.read("a.img");
    cache.write("b.img", b"de");
    let total = cache.index.lock().map(|i| i.as_ref().map(|i| i.total));
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(read.as_deref(), Some(&b"abc"[..]));
    assert_eq!(total.ok().flatten(), Some(5));
}

/// A hit refreshes its file's time, the eviction order, and leaves the folder unscanned until
/// a write needs its total; the scan then reads the refreshed time.
#[test]
fn a_hit_refreshes_its_time_without_scanning_the_folder() {
    let Some(dir) = crate::testing::scratch("covers-hit") else {
        return;
    };
    let path = dir.join("a.img");
    std::fs::write(&path, b"abc").unwrap();
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(old)
        .unwrap();
    let cache = DiskCache::new(dir.clone());

    let read = cache.read("a.img");
    let scanned = cache.index.lock().unwrap().is_some();
    let touched = std::fs::metadata(&path).unwrap().modified().unwrap();
    cache.write("b.img", b"de");
    let indexed = cache
        .index
        .lock()
        .unwrap()
        .as_ref()
        .map(|i| i.files["a.img"]);
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(read.as_deref(), Some(&b"abc"[..]));
    assert!(!scanned, "a read scans nothing");
    assert!(touched > old, "the hit's time is refreshed");
    assert_eq!(indexed, Some((3, touched)));
}

#[test]
fn sized_url_rewrites_cdn_sizes() {
    assert_eq!(
        sized_url("https://lh3.googleusercontent.com/abc=w60-h60-l90-rj", 544),
        "https://lh3.googleusercontent.com/abc=w544-h544-l90-rj"
    );
    assert_eq!(
        sized_url("https://yt3.ggpht.com/xyz=s88-c-k-c0x00ffffff-no-rj", 226),
        "https://yt3.ggpht.com/xyz=s226-c-k-c0x00ffffff-no-rj"
    );
    let video = "https://i.ytimg.com/vi/abc/sddefault.jpg?sqp=x=y";
    assert_eq!(sized_url(video, 544), video);
}

/// `w`×`h` red, with black bands of `band` rows top and bottom, as a PNG.
fn letterboxed(w: u32, h: u32, band: u32) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |_, y| {
        if y < band || y >= h - band {
            image::Rgb([0, 0, 0])
        } else {
            image::Rgb([220, 20, 20])
        }
    });
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)
        .expect("png");
    out.into_inner()
}

#[test]
fn decode_wide_drops_a_4_3_thumbnails_letterbox() {
    // ytimg's sddefault: 640×480, the 16:9 picture between 60-row bands.
    let out = decode(&letterboxed(640, 480, 60), 320, 180).expect("decode");
    assert_eq!((out.width(), out.height()), (320, 180));
    assert!(
        out.as_slice().iter().all(|p| p.r > 150 && p.g < 80),
        "a band of the letterbox shows"
    );
}

#[test]
fn a_square_cover_of_a_letterboxed_thumbnail_shows_no_band() {
    // ytimg's hqdefault: 480×360, the 16:9 picture between 45-row bands.
    let out = decode(&letterboxed(480, 360, 45), 64, 64).expect("decode");
    assert!(
        out.as_slice().iter().all(|p| p.r > 150 && p.g < 80),
        "a band of the letterbox shows"
    );
    // A 4:3 picture without bands keeps its whole height.
    let plain = decode(&letterboxed(480, 360, 0), 64, 64).expect("decode");
    assert_eq!((plain.width(), plain.height()), (64, 64));
}

/// Resizing from the crop's view gives the pixels a resize of a copy of the crop gave.
#[test]
fn decode_resizes_the_crop_as_a_copy_of_it_would() {
    let img = image::RgbaImage::from_fn(97, 61, |x, y| {
        image::Rgba([
            (x * 7 % 256) as u8,
            (y * 11 % 256) as u8,
            ((x * y) % 256) as u8,
            255,
        ])
    });
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png)
        .expect("png");
    let png = png.into_inner();
    let copy = |x, y, w, h, to_w, to_h| {
        let crop = image::imageops::crop_imm(&img, x, y, w, h).to_image();
        image::imageops::resize(&crop, to_w, to_h, image::imageops::FilterType::CatmullRom)
            .into_raw()
    };
    let bytes = |b: SharedPixelBuffer<Rgba8Pixel>| b.as_bytes().to_vec();
    // A square from the middle, smaller; and a 16:9 band, larger.
    assert_eq!(
        bytes(decode(&png, 40, 40).expect("decode")),
        copy(18, 0, 61, 61, 40, 40)
    );
    assert_eq!(
        bytes(decode(&png, 160, 90).expect("decode")),
        copy(0, 3, 97, 54, 160, 90)
    );
    // The whole image, resized or as it is.
    assert_eq!(
        bytes(decode(&png, 194, 122).expect("decode")),
        copy(0, 0, 97, 61, 194, 122)
    );
    // The same picture as an RGB PNG (as a JPEG decodes) gives the same bytes at every size.
    let mut rgb = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img.clone())
        .into_rgb8()
        .write_to(&mut rgb, image::ImageFormat::Png)
        .expect("png");
    let rgb = rgb.into_inner();
    for (w, h) in [(40, 40), (160, 90), (194, 122), (97, 61), (61, 61)] {
        assert_eq!(
            bytes(decode(&rgb, w, h).expect("decode")),
            bytes(decode(&png, w, h).expect("decode")),
            "{w}x{h}"
        );
    }
    assert_eq!(bytes(decode(&png, 97, 61).expect("decode")), img.into_raw());
}

/// What the window keeps while it shows nothing: the named covers, and their bytes.
#[test]
fn retain_keeps_only_the_named_covers() {
    let mut memory = Memory::default();
    let img = || Image::from_rgba8(SharedPixelBuffer::new(10, 10));
    let key = |u: &str| (u.to_string(), 10, 10, Shape::Centre);
    memory.put(key("a"), img());
    memory.put(key("b"), img());
    memory.retain(&HashSet::from([key("a")]));
    assert!(memory.get(&key("a")).is_some());
    assert!(memory.get(&key("b")).is_none());
    assert_eq!(memory.bytes, 400);
}

#[test]
fn wide_is_16_9() {
    assert_eq!(wide_height(320), 180);
    assert_eq!(wide_height(481), 271);
}

#[test]
fn bucket_covers_the_display() {
    assert_eq!(bucket(48), 120);
    assert_eq!(bucket(264), 544);
    assert_eq!(bucket(5000), 1200);
}

/// The artist banner: the photo cropped from the top across its whole width, the ground faded
/// in down it as the page's gradient was (none at the top, all of it at the foot), opaque.
#[test]
fn a_banner_is_cut_from_the_top_with_the_ground_faded_in() {
    // A square photo: red at the top half, blue below.
    let img = image::RgbaImage::from_fn(64, 64, |_, y| {
        image::Rgba(if y < 32 {
            [200, 0, 0, 255]
        } else {
            [0, 0, 200, 255]
        })
    });
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png)
        .expect("png");
    let ground = 0x0F_0F_0F;
    let banner = decode_banner(&png.into_inner(), 32, 10, ground).expect("decode");
    let px = |x: usize, y: usize| {
        let at = (y * 32 + x) * 4;
        banner.as_bytes()[at..at + 4].to_vec()
    };
    // 32x10 from a 64 px square takes its top 20 rows, all red (none of the blue below),
    // the ground faded in at each row's middle: the stops are none at the top, 10 % at 30 %
    // of the height, 70 % at 65 %, all of it at the foot.
    let red_at = |y: usize| {
        let a = banner_fade((y as f32 + 0.5) / 10.0);
        vec![
            (200.0 * (1.0 - a) + 15.0 * a).round() as u8,
            (15.0 * a).round() as u8,
        ]
    };
    for y in 0..10 {
        assert_eq!(px(0, y)[..2], red_at(y)[..], "row {y}");
        assert_eq!(px(31, y)[..2], red_at(y)[..], "row {y}");
    }
    assert!((banner_fade(0.25) - 0.25 / 0.3 * 0.1).abs() < 1e-6);
    assert!(
        banner
            .as_bytes()
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[3] == 255)
    );
    assert_eq!(banner_fade(0.0), 0.0);
    assert_eq!(banner_fade(1.0), 1.0);
    assert!((banner_fade(0.65) - 0.7).abs() < 1e-6);
}
