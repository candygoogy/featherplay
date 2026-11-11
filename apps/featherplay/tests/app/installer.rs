//! The installer script (`packaging/windows/featherplay.iss`) repeats a few of the app's
//! names, since Inno Setup can't read them from the code: the Start menu shortcut and its
//! AppUserModelID, which the app checks at every start, and the end-session window the
//! uninstaller closes the app through. These keep the two in step.

use featherplay::endsession;
use featherplay_platform::identity;

fn script() -> String {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../packaging/windows/featherplay.iss"
    );
    std::fs::read_to_string(path).expect("read the installer script")
}

#[test]
fn the_shortcut_carries_the_apps_identity() {
    let iss = script();
    assert!(
        iss.contains(&format!("#define AppUserModelId \"{}\"", identity::AUMID)),
        "the installer's AppUserModelId isn't {}",
        identity::AUMID
    );
    // `{userprograms}\{#AppName}` makes `featherplay.lnk` in the Start menu's Programs
    // folder, the file the app looks for.
    assert_eq!(identity::SHORTCUT_FILE, "featherplay.lnk");
    assert!(iss.contains("#define AppName \"featherplay\""));
    assert!(
        iss.contains(r#"Name: "{userprograms}\{#AppName}"; Filename: "{app}\featherplay.exe""#)
    );
}

#[test]
fn the_uninstaller_finds_the_end_session_window() {
    let iss = script();
    assert!(
        iss.contains(&format!(
            "#define EndSessionClass \"{}\"",
            endsession::CLASS
        )),
        "the installer's EndSessionClass isn't {}",
        endsession::CLASS
    );
    assert!(
        iss.contains(&format!(
            "#define EndSessionTitle \"{}\"",
            endsession::TITLE
        )),
        "the installer's EndSessionTitle isn't {}",
        endsession::TITLE
    );
}

#[test]
fn an_update_the_app_starts_relaunches_it() {
    use featherplay::updates::install::SETUP_ARGS;

    let iss = script();
    // The app starts the setup silently with /RELAUNCH; the script reads that flag to close
    // the app first and to start the new version after.
    assert!(SETUP_ARGS.contains(&"/VERYSILENT") && SETUP_ARGS.contains(&"/RELAUNCH"));
    assert!(iss.contains("CompareText(ParamStr(I), '/RELAUNCH') = 0"));
    assert!(iss.contains(r#"Filename: "{app}\featherplay.exe"; Flags: nowait; Check: Relaunch"#));
    assert!(iss.contains("if Relaunch() then begin"));
}
