use featherplay_lastfm::api_sig;

#[test]
fn api_sig_vector() {
    assert_eq!(
        api_sig(
            &[
                ("method", "auth.getSession"),
                ("token", "yyy"),
                ("api_key", "xxx")
            ],
            "zzz"
        ),
        "75df1fdb6b738160924a52b1732fdde7"
    );
}
