//! At startup the window opens Home before the controller has restored the stored session.
//! Home (and any page opened meanwhile) waits for it instead of loading signed out and
//! saying the session has ended; once the session is back, Home fills in. On the offscreen
//! platform with the fixtures: nothing is fetched.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use featherplay::app::{self, Ctx};
use featherplay::controller::UiEvent;
use featherplay::covers::Covers;
use featherplay::offscreen;
use featherplay::router::{LibraryTab, Page};
use featherplay::ui::{AppWindow, HomeData, LibraryData, Theme};
use featherplay_controller::catalog::{Catalog, Source};
use featherplay_controller::fixtures::Fixtures;
use slint::{ComponentHandle, Model};
use tokio::sync::mpsc;

const WAIT: Duration = Duration::from_secs(10);
/// Long enough for a load against the signed-out source to come back.
const SETTLE: Duration = Duration::from_millis(300);

/// One test: the platform can be installed once per process.
#[test]
fn pages_wait_for_the_stored_session_at_startup() {
    let home = Path::new(env!("CARGO_TARGET_TMPDIR")).join("startup-session-home");
    std::fs::create_dir_all(&home).expect("home");
    // SAFETY: set before any other thread of this test process starts.
    unsafe { std::env::set_var("FEATHERPLAY_HOME", &home) };

    let pump = offscreen::install().expect("platform");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime");
    let fixtures = Arc::new(Fixtures::load().expect("fixtures"));
    // As in the app: the controller sets the source once it has signed in.
    let catalog = Catalog::new(runtime.handle().clone()).expect("catalog");
    let covers = Covers::new(
        runtime.handle().clone(),
        home.join("cache"),
        Some(fixtures.clone()),
    );
    let (actions, _sent) = mpsc::unbounded_channel();
    let ui = AppWindow::new().expect("window");
    ui.global::<Theme>().set_still(true);
    let ctx = Ctx::new(&ui, catalog.clone(), covers, actions);
    app::wire(&ctx, &ui);
    let home_data = ui.global::<HomeData>();
    let library = ui.global::<LibraryData>();

    // The window opens on Home while the session is still being restored.
    ctx.show_current();
    pump.until(|| false, SETTLE);
    assert_eq!(
        home_data.get_error(),
        "",
        "Home loaded before the session was restored"
    );
    assert!(home_data.get_loading(), "Home shows it's loading meanwhile");

    // A page opened meanwhile waits too.
    ctx.navigate(Page::Library {
        tab: LibraryTab::Liked,
    });
    pump.until(|| false, SETTLE);
    assert_eq!(
        library.get_error(),
        "",
        "Library loaded before the session was restored"
    );
    assert!(library.get_loading());

    // The controller signs in with the stored session: the page that waited loads.
    catalog.set(Source::Fixtures(fixtures));
    app::on_event(UiEvent::Account { signed_in: true });
    assert!(
        pump.until(
            || !library.get_loading() && library.get_liked().row_count() > 0,
            WAIT
        ),
        "Library fills in once the session is back"
    );
    assert_eq!(library.get_error(), "");

    // And so does Home.
    ctx.back();
    assert!(
        pump.until(
            || !home_data.get_loading() && home_data.get_shelves().row_count() > 0,
            WAIT
        ),
        "Home fills in once the session is back"
    );
    assert_eq!(home_data.get_error(), "");
}
