//! Skip silence's marks reach the seek bar's data (`Player.silences`), on the offscreen
//! platform (no window is shown): the controller's marks as they come, and none after a
//! track change.

use std::path::PathBuf;

use featherplay::app::Ctx;
use featherplay::bar;
use featherplay::catalog::Catalog;
use featherplay::covers::Covers;
use featherplay::offscreen;
use featherplay::ui::{AppWindow, Player};
use slint::{ComponentHandle, Model};
use tokio::sync::mpsc;

/// One test: the platform can be installed once per process.
#[test]
fn the_bar_takes_the_marks_and_drops_them_with_the_track() {
    let _pump = offscreen::install().expect("platform");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("runtime");
    let ui = AppWindow::new().expect("window");
    let catalog = Catalog::new(rt.handle().clone()).expect("catalog");
    let cache = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("bar_silences");
    let covers = Covers::new(rt.handle().clone(), cache, None);
    let (actions, _) = mpsc::unbounded_channel();
    let ctx = Ctx::new(&ui, catalog, covers, actions);

    let spans = || {
        let m = ui.global::<Player>().get_silences();
        (0..m.row_count())
            .filter_map(|i| m.row_data(i))
            .map(|s| (s.start, s.end))
            .collect::<Vec<_>>()
    };
    assert!(spans().is_empty());
    bar::show_silences(&ctx, &[(0.0, 0.1), (0.85, 1.0)]);
    assert_eq!(spans(), [(0.0, 0.1), (0.85, 1.0)]);
    // A track change: the controller sends none.
    bar::show_silences(&ctx, &[]);
    assert!(spans().is_empty());
}
