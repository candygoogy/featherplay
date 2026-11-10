//! The window draws the Tri-vane with `Path`s (`ui/logo.slint`), copied from the SVG
//! drawings the build renders the icons from (`ui/logo/`). This keeps the two in step.

fn read(file: &str) -> String {
    let path = format!("{}/ui/{file}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {path}: {e}"))
        .replace('\r', "")
}

/// Each `<path>` of the drawing, in order: its id, fill and `d`.
fn paths(svg: &str) -> Vec<(String, String, String)> {
    svg.split("<path ")
        .skip(1)
        .map(|p| {
            let attr = |name: &str| {
                let key = format!(" {name}=\"");
                let start = format!(" {p}").find(&key).expect(name) + key.len() - 1;
                p[start..start + p[start..].find('"').expect("quote")].to_string()
            };
            (attr("id"), attr("fill"), attr("d"))
        })
        .collect()
}

/// `component`'s paths, drawn in the order of `light`'s (the dark drawing on a ring that only
/// light grounds fill).
fn check(slint: &str, component: &str, light: &str) {
    let start = slint
        .find(&format!("export component {component} {{"))
        .unwrap_or_else(|| panic!("{component} in logo.slint"));
    let body = &slint[start..];
    let body = &body[..body[1..]
        .find("export component")
        .map_or(body.len(), |i| i + 1)];
    let mut at = 0;
    for (id, fill, d) in paths(&read(&format!("logo/{light}"))) {
        let fill = if id == "ring" {
            format!("Theme.dark ? transparent : {fill}")
        } else {
            fill
        };
        let drawn = format!("fill: {fill};\n        commands: \"{d}\";");
        let found = body[at..]
            .find(&drawn)
            .unwrap_or_else(|| panic!("{light}'s {id} isn't {component}'s next path"));
        at += found + drawn.len();
    }
}

#[test]
fn logo_paths_match_the_svgs() {
    let slint = read("logo.slint");
    check(&slint, "Logo", "tri-vane-light.svg");
    check(&slint, "SmallLogo", "tri-vane-light-16.svg");
    // The dark drawings are the light ones without the ring.
    for (dark, light) in [
        ("tri-vane.svg", "tri-vane-light.svg"),
        ("tri-vane-16.svg", "tri-vane-light-16.svg"),
    ] {
        let mut ringed = paths(&read(&format!("logo/{light}")));
        ringed.retain(|(id, _, _)| id != "ring");
        assert_eq!(paths(&read(&format!("logo/{dark}"))), ringed, "{dark}");
    }
}
