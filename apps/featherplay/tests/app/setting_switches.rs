//! Settings' plain switches: each starts at its default and sets what the app reads
//! (`pages::settings`). The defaults themselves are pinned in `featherplay-store`.

use featherplay::pages;
use featherplay::snapshot::Scene;
use featherplay::ui::{SettingKey, SettingsData};
use slint::ComponentHandle;

#[test]
fn each_switch_starts_at_its_default_and_sets_its_setting() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("settings").expect("settings");
    scene.settle();
    let data = scene.ui.global::<SettingsData>();

    // (case, the switch, its default, what the page shows, what the app reads, the turns)
    type Case = (
        &'static str,
        SettingKey,
        bool,
        fn(&SettingsData) -> bool,
        fn() -> bool,
        &'static [bool],
    );
    let cases: [Case; 6] = [
        (
            "Add what I play to my history",
            SettingKey::PlayHistory,
            true,
            |d| d.get_play_history(),
            pages::settings::play_history,
            &[false],
        ),
        (
            "Check for updates",
            SettingKey::CheckForUpdates,
            true,
            |d| d.get_check_for_updates(),
            pages::settings::check_for_updates,
            &[false],
        ),
        (
            "Install updates",
            SettingKey::InstallUpdates,
            true,
            |d| d.get_install_updates(),
            pages::settings::install_updates,
            &[false],
        ),
        (
            "Look up YouTube directly",
            SettingKey::DnsOverHttps,
            false,
            |d| d.get_dns_over_https(),
            pages::settings::dns_over_https,
            &[true, false],
        ),
        (
            "Turn off the mini player (a tray click opens the window)",
            SettingKey::TrayClickOpensWindow,
            false,
            |d| d.get_tray_click_opens_window(),
            pages::settings::tray_click_opens_window,
            &[true, false],
        ),
        (
            "Cover behind the lyrics",
            SettingKey::LyricsBlur,
            false,
            |d| d.get_lyrics_blur(),
            pages::settings::lyrics_blur,
            &[true, false],
        ),
    ];
    for (case, key, default, shown, read, turns) in cases {
        assert_eq!(shown(&data), default, "{case}: the switch's default");
        assert_eq!(read(), default, "{case}: the setting's default");
        for &on in turns {
            data.invoke_toggled(key, on);
            assert_eq!(shown(&data), on, "{case} turned {on}: the switch");
            assert_eq!(read(), on, "{case} turned {on}: the setting");
        }
    }
}
