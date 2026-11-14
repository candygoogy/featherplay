use super::*;

/// A thread that panicked while updating the index may have left its total off: the
/// next use scans the folder again rather than giving up on the disk cache.
#[test]
fn a_poisoned_index_is_scanned_again() {
    // Under `FEATHERPLAY_HOME`, else the temp folder.
    let base = std::env::var("FEATHERPLAY_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("featherplay-tests"));
    let dir = base.join(format!("test-covers-poison-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
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
    let total = cache.index.lock().map(|i| i.as_ref().map(|i| i.total));
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(read.as_deref(), Some(&b"abc"[..]));
    assert_eq!(total.ok().flatten(), Some(3));
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
fn decode_square_keeps_the_centred_square() {
    let out = decode(&letterboxed(640, 480, 60), 300, 300).expect("decode");
    assert_eq!((out.width(), out.height()), (300, 300));
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
        image::imageops::resize(&crop, to_w, to_h, FilterType::CatmullRom).into_raw()
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
    assert_eq!(bytes(decode(&png, 97, 61).expect("decode")), img.into_raw());
}

/// What the window keeps while it shows nothing: the named covers, and their bytes.
#[test]
fn retain_keeps_only_the_named_covers() {
    let mut memory = Memory::default();
    let img = || Image::from_rgba8(SharedPixelBuffer::new(10, 10));
    let key = |u: &str| (u.to_string(), 10, 10);
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
