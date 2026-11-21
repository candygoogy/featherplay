use super::*;
use aes::cipher::BlockModeEncrypt;
use hmac::{Hmac, KeyInit, Mac};
use proptest::prelude::*;
use std::path::PathBuf;

/// What Chromium stores for `value` of a cookie for `host`, with the store's fixed key.
fn encrypt(value: &str, host: &str, hashed: bool) -> Vec<u8> {
    let mut plain = Vec::new();
    if hashed {
        plain.extend_from_slice(&Sha256::digest(host.as_bytes()));
    }
    plain.extend_from_slice(value.as_bytes());
    let mut out = b"v10".to_vec();
    out.extend(
        cbc::Encryptor::<aes::Aes128>::new_from_slices(&V10_KEY, &V10_IV)
            .unwrap()
            .encrypt_padded_vec::<Pkcs7>(&plain),
    );
    out
}

#[test]
fn the_v10_key_is_pbkdf2_of_peanuts() {
    // One round of PBKDF2 is one HMAC of the salt and the block number 1.
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(b"peanuts").unwrap();
    mac.update(b"saltysalt\x00\x00\x00\x01");
    assert_eq!(mac.finalize().into_bytes()[..16], V10_KEY);
}

#[test]
fn a_value_chrome_157_wrote_decrypts() {
    // `SAPISID=test-sapisid` for 127.0.0.1, from a profile Chrome 157 wrote in WSL.
    let hex = "7631302BB01EEBE40CCFF8B9D3B700273158B01EEE6BEDEECAB4DE6DBA0816A56B003868B43AFCC10FD65D5411AFB4C42BE837";
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    assert_eq!(decrypt(&bytes, "127.0.0.1", true).unwrap(), "test-sapisid");
    // Another host's hash doesn't match.
    assert!(matches!(
        decrypt(&bytes, ".youtube.com", true),
        Err(Unreadable::Store(_))
    ));
}

#[test]
fn a_keyring_value_is_refused_without_asking_the_keyring() {
    for prefix in [&b"v11"[..], b"v12"] {
        let mut v = prefix.to_vec();
        v.extend_from_slice(&[0; 32]);
        assert_eq!(decrypt(&v, "x", true), Err(Unreadable::Keyring));
    }
    assert!(matches!(
        decrypt(b"plain", "x", true),
        Err(Unreadable::Store(_))
    ));
}

proptest! {
    /// Whatever a store holds, decrypting it ends in a value or an error, never a panic.
    #[test]
    fn any_encrypted_value_is_read_or_refused(bytes in proptest::collection::vec(any::<u8>(), 0..80), v10 in any::<bool>()) {
        let mut v = if v10 { b"v10".to_vec() } else { Vec::new() };
        v.extend(bytes);
        let _ = decrypt(&v, ".youtube.com", true);
        let _ = decrypt(&v, ".youtube.com", false);
    }

    /// A value Chromium encrypted comes back as it was, with or without the host's hash.
    #[test]
    fn an_encrypted_value_comes_back(value in "\\PC{0,64}", host in "[a-z.]{1,24}", hashed in any::<bool>()) {
        prop_assert_eq!(decrypt(&encrypt(&value, &host, hashed), &host, hashed), Ok(value));
    }
}

/// A new, empty folder for one test's profile.
fn profile(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "featherplay-test-cookies-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Default")).unwrap();
    dir
}

/// A Chromium store at schema `version` with Chromium's columns (those the read leaves out
/// too), holding `rows`: (host, name, value, has_expires, top_frame_site_key).
fn chromium_store(dir: &Path, version: i64, rows: &[(&str, &str, &str, bool, &str)]) {
    let db = Connection::open(dir.join("Default/Cookies")).unwrap();
    db.execute_batch(
        "CREATE TABLE meta(key LONGVARCHAR NOT NULL UNIQUE PRIMARY KEY, value LONGVARCHAR);
         CREATE TABLE cookies(creation_utc INTEGER NOT NULL,host_key TEXT NOT NULL,
           top_frame_site_key TEXT NOT NULL,name TEXT NOT NULL,value TEXT NOT NULL,
           encrypted_value BLOB NOT NULL,path TEXT NOT NULL,expires_utc INTEGER NOT NULL,
           is_secure INTEGER NOT NULL,is_httponly INTEGER NOT NULL,
           last_access_utc INTEGER NOT NULL,has_expires INTEGER NOT NULL,
           is_persistent INTEGER NOT NULL,priority INTEGER NOT NULL,samesite INTEGER NOT NULL,
           source_scheme INTEGER NOT NULL,source_port INTEGER NOT NULL,
           last_update_utc INTEGER NOT NULL,source_type INTEGER NOT NULL,
           has_cross_site_ancestor INTEGER NOT NULL);",
    )
    .unwrap();
    db.execute(
        "INSERT INTO meta VALUES ('version', ?1), ('last_compatible_version', ?1)",
        [version.to_string()],
    )
    .unwrap();
    for (host, name, value, expires, top) in rows {
        // 1.8e9 s after 1970 (2027-01-15), in microseconds since 1601.
        let expires_utc: i64 = if *expires { 13_444_473_600_000_000 } else { 0 };
        db.execute(
            "INSERT INTO cookies VALUES (0, ?1, ?2, ?3, '', ?4, '/', ?5, 1, 0, 0, ?6, ?6, 1, 0, 2, 443, 0, 0, 0)",
            rusqlite::params![
                host,
                top,
                name,
                encrypt(value, host, version >= 24),
                expires_utc,
                expires
            ],
        )
        .unwrap();
    }
}

#[test]
fn a_chromium_store_reads_as_its_cookies() {
    let dir = profile("chromium");
    chromium_store(
        &dir,
        24,
        &[
            (".youtube.com", "SAPISID", "abc/def", true, ""),
            ("music.youtube.com", "S", "sess", false, ""),
            (".youtube.com", "P", "part", true, "https://example.com"),
        ],
    );
    let c = read(&dir, Kind::Chromium).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(c.len(), 3);
    assert_eq!(
        (
            c[0].name.as_str(),
            c[0].value.as_str(),
            c[0].domain.as_str()
        ),
        ("SAPISID", "abc/def", ".youtube.com")
    );
    assert_eq!(c[0].expires, Some(1_800_000_000.0));
    assert!(c[0].secure && !c[0].partitioned);
    assert_eq!(c[1].expires, None);
    assert!(c[2].partitioned);
}

#[test]
fn an_older_chromium_store_has_no_host_hash_and_a_missing_one_no_cookies() {
    let dir = profile("chromium-23");
    chromium_store(&dir, 23, &[(".youtube.com", "SID", "x", true, "")]);
    let c = read(&dir, Kind::Chromium).unwrap();
    assert_eq!(c[0].value, "x");
    std::fs::remove_dir_all(dir.join("Default")).unwrap();
    assert_eq!(read(&dir, Kind::Chromium), Ok(Vec::new()));
    assert_eq!(read(&dir, Kind::Firefox), Ok(Vec::new()));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A Firefox store at `schema` with Firefox's columns, one cookie expiring at 1.8e9 s.
fn firefox_store(dir: &Path, schema: i64, origin: &str) {
    let db = Connection::open(dir.join("cookies.sqlite")).unwrap();
    db.execute_batch(&format!(
        "PRAGMA journal_mode = WAL; PRAGMA user_version = {schema};
         CREATE TABLE moz_cookies (id INTEGER PRIMARY KEY,
           originAttributes TEXT NOT NULL DEFAULT '', name TEXT, value TEXT, host TEXT,
           path TEXT, expiry INTEGER, lastAccessed INTEGER, creationTime INTEGER,
           isSecure INTEGER, isHttpOnly INTEGER, inBrowserElement INTEGER DEFAULT 0,
           sameSite INTEGER DEFAULT 0, schemeMap INTEGER DEFAULT 0,
           isPartitionedAttributeSet INTEGER DEFAULT 0, updateTime INTEGER,
           CONSTRAINT moz_uniqueid UNIQUE (name, host, path, originAttributes));"
    ))
    .unwrap();
    let expiry: i64 = if schema >= 16 {
        1_800_000_000_000
    } else {
        1_800_000_000
    };
    db.execute(
        "INSERT INTO moz_cookies (originAttributes, name, value, host, path, expiry, isSecure)
         VALUES (?1, 'SAPISID', 'abc', '.youtube.com', '/', ?2, 1)",
        rusqlite::params![origin, expiry],
    )
    .unwrap();
}

#[test]
fn a_firefox_store_reads_its_expiry_in_the_schemas_unit() {
    for (schema, origin, partitioned) in [
        (17, "", false),
        (15, "", false),
        (17, "^partitionKey=%28https%2Cexample.com%29", true),
    ] {
        let dir = profile(&format!("firefox-{schema}-{partitioned}"));
        firefox_store(&dir, schema, origin);
        let c = read(&dir, Kind::Firefox).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(c.len(), 1);
        let c = &c[0];
        assert_eq!(
            (
                c.name.as_str(),
                c.value.as_str(),
                c.domain.as_str(),
                c.path.as_str()
            ),
            ("SAPISID", "abc", ".youtube.com", "/")
        );
        assert_eq!(c.expires, Some(1_800_000_000.0), "schema {schema}");
        assert!(c.secure);
        assert_eq!(c.partitioned, partitioned);
    }
}

#[test]
fn a_store_that_isnt_one_is_unreadable() {
    let dir = profile("broken");
    std::fs::write(dir.join("cookies.sqlite"), b"not a database").unwrap();
    std::fs::write(dir.join("Default/Cookies"), b"").unwrap();
    let firefox = read(&dir, Kind::Firefox);
    let chromium = read(&dir, Kind::Chromium);
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(matches!(firefox, Err(Unreadable::Store(_))), "{firefox:?}");
    assert!(
        matches!(chromium, Err(Unreadable::Store(_))),
        "{chromium:?}"
    );
}
