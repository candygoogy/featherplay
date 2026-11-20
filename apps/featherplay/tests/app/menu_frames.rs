//! Menus and Settings' expanders unfold and fold without long frames, at 4K 150 %, playing
//! with sound: Settings' choice menu (a dropdown's), the account menu (through `MenuPanel`, as
//! every in-window menu) and the Sleep timer's fold-out (`ExpanderBody`, whose unfolding
//! moves the rows under it). Each frame is one turn of the event loop on a frozen clock at
//! 240 Hz through the app's own present path; the first frame holds the input that opens
//! or closes. The ceilings hold in optimised builds.

use featherplay::snapshot::Scene;
use featherplay::ui::{AccountMenu, ChoiceMenu, SettingId, SettingsData};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::support::{Frame1, frames, label, quantile, scene_at, three_tries};

/// The worst frame of a menu's unfolding or folding, ms of CPU (optimised).
const MENU_MS: f64 = 8.0;
/// The worst frame of the expander's, which lays the page out and repaints what moves.
const EXPANDER_MS: f64 = 12.0;
/// Frames for each motion: 360 ms, past `t-expand`.
const RUN: u32 = 90;

fn worst(run: &[Frame1]) -> f64 {
    quantile(&run.iter().map(|f| f.cpu).collect::<Vec<_>>(), 1.0)
}

/// The motion `act` starts, run out; its worst frame against `ceiling`.
fn check(scene: &Scene, name: &str, ceiling: f64, act: &dyn Fn()) -> Result<(), String> {
    let run = frames(scene.ui.window(), RUN, |i| {
        if i == 0 {
            act();
        }
    });
    let w = worst(&run);
    println!("{name} at 4K 150 %: worst frame {w:.2} ms");
    if w <= ceiling {
        Ok(())
    } else {
        Err(format!("{name}: a frame cost {w:.2} ms, over {ceiling}"))
    }
}

fn strings(v: &[&str]) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        v.iter().map(|s| SharedString::from(*s)).collect::<Vec<_>>(),
    ))
}

#[test]
fn menus_and_expanders_unfold_without_long_frames() {
    let scene = scene_at(3840, 2160, 1.5, "settings", true);
    let menu = scene.ui.global::<ChoiceMenu>();
    let data = scene.ui.global::<SettingsData>();
    let open_choices = || {
        menu.invoke_show(
            SettingId::OutputDevice,
            "Output device".into(),
            strings(&["System default", "Speakers", "Headphones"]),
            strings(&["", "Realtek(R) Audio", "WH-1000XM4"]),
            0,
            1600.0,
            400.0,
            160.0,
            32.0,
            false,
        );
    };
    let account = || label::click(&scene, "Account, Test Listener");
    data.set_sleep_timer(false);
    // Each once before measuring: a menu's first open makes its rows.
    open_choices();
    frames(scene.ui.window(), RUN, |_| {});
    menu.invoke_close(false);
    account();
    frames(scene.ui.window(), RUN, |_| {});
    scene.ui.global::<AccountMenu>().invoke_close();
    frames(scene.ui.window(), RUN, |_| {});
    three_tries(|_| {
        check(&scene, "the choice menu unfolding", MENU_MS, &open_choices)?;
        check(&scene, "the choice menu folding", MENU_MS, &|| {
            menu.invoke_close(false);
        })?;
        check(&scene, "the account menu unfolding", MENU_MS, &account)?;
        check(&scene, "the account menu folding", MENU_MS, &|| {
            scene.ui.global::<AccountMenu>().invoke_close();
        })?;
        check(
            &scene,
            "the Sleep timer's rows unfolding",
            EXPANDER_MS,
            &|| {
                data.set_sleep_timer(true);
            },
        )?;
        check(
            &scene,
            "the Sleep timer's rows folding",
            EXPANDER_MS,
            &|| {
                data.set_sleep_timer(false);
            },
        )
    });
    assert!(!menu.get_open());
}
