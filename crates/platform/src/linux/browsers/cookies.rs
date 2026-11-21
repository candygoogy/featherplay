//! The cookies a closed browser kept in its temporary profile, read from its cookie database
//! (SQLite, through rusqlite with SQLite built in: rusqlite 0.43 needs SQLite 3.45.3, newer
//! than Ubuntu 22.04's and Debian 12's). Only a sign-in's own profile is ever opened, never a
//! profile of the user's. Cookie values are never logged, printed or written here.
//!
//! # Where each fact comes from (Chromium and Firefox `main`, checked 2026-11-21)
//! - **Chromium**: `Default/Cookies` (`chrome/common/chrome_constants.h`; it moves to
//!   `Default/Network/Cookies` only where `kTriggerNetworkDataMigration` is on, Windows by
//!   default, so both are looked for). Its `cookies` table
//!   (`net/extras/sqlite/sqlite_persistent_cookie_store.cc`, schema 24): `host_key`, `name`,
//!   `encrypted_value` (`value` stays empty), `path`, `expires_utc` (microseconds since
//!   1601-01-01, `sql/statement.cc`), `has_expires`, `is_secure`, `top_frame_site_key` (set
//!   for a partitioned cookie); from schema 24 (the `meta` table's `version`) the decrypted
//!   value starts with the SHA-256 of `host_key`, which Chromium checks and strips.
//! - **Its encryption** with `--password-store=basic`: os_crypt_async's `PosixKeyProvider`
//!   (`components/os_crypt/async/browser/posix_key_provider.cc`): the prefix `v10`, then
//!   AES-128-CBC with PKCS#7 padding, the fixed key below (PBKDF2-HMAC-SHA1 of "peanuts" with
//!   the salt "saltysalt", one round) and an IV of 16 spaces
//!   (`components/os_crypt/async/common/encryptor.cc`). `v11` is a key in the desktop's
//!   keyring (`freedesktop_secret_key_provider.cc`) and `v12` one from the secret portal: the
//!   switch didn't take, and the keyring is never asked (`Unreadable::Keyring`).
//! - **Firefox**: `cookies.sqlite`, table `moz_cookies`
//!   (`netwerk/cookie/CookiePersistentStorage.cpp`, schema 17): `name`, `value`, `host`
//!   (`.youtube.com` for a domain cookie, as Chromium's `host_key`), `path`, `expiry`,
//!   `isSecure`, `originAttributes` (with `partitionKey=` for a partitioned cookie). `expiry`
//!   is in milliseconds from schema 16 (Firefox 142, which multiplied the old seconds by
//!   1000), so the schema (`PRAGMA user_version`) says which. Only cookies with an expiry are
//!   written; the database is in WAL mode, so it's opened for writing, which lets SQLite take
//!   in the `-wal` file a killed Firefox leaves.
//!
//! All seen in WSL with Chrome 157, Chromium 158 and Firefox 160 and 153 ESR (2026-11-21).

use std::path::Path;

use aes::cipher::block_padding::Pkcs7;
use aes::cipher::{BlockModeDecrypt, KeyIvInit};
use featherplay_core::session::Cookie;
use rusqlite::{Connection, OpenFlags};
use sha2::{Digest, Sha256};

use super::Kind;

/// Why a profile's cookies couldn't be read.
#[derive(Debug, PartialEq, Eq)]
pub enum Unreadable {
    /// Chromium kept them under a key in the desktop's keyring (`v11`, `v12`):
    /// `--password-store=basic` didn't take (a wrapper or a policy may override it).
    Keyring,
    /// The database couldn't be read: what went wrong, never a value.
    Store(String),
}

/// The cookies the browser of `kind` kept in `profile`. None when it kept no database (it
/// was closed before a page set one).
pub fn read(profile: &Path, kind: Kind) -> Result<Vec<Cookie>, Unreadable> {
    match kind {
        Kind::Chromium => chromium(profile),
        Kind::Firefox => firefox(profile),
    }
}

/// The Chromium store's `v10` key: PBKDF2-HMAC-SHA1("peanuts", "saltysalt", 1 round, 16
/// bytes), as `posix_key_provider.cc` has it.
const V10_KEY: [u8; 16] = [
    0xfd, 0x62, 0x1f, 0xe5, 0xa2, 0xb4, 0x02, 0x53, 0x9d, 0xfa, 0x14, 0x7c, 0xa9, 0x27, 0x27, 0x78,
];
const V10_IV: [u8; 16] = [b' '; 16];

/// Seconds between 1601-01-01 (Chromium's epoch) and 1970-01-01.
const WINDOWS_TO_UNIX: f64 = 11_644_473_600.0;

fn store(what: &str) -> impl FnOnce(rusqlite::Error) -> Unreadable + '_ {
    // rusqlite's message names the column or the SQL, never a value.
    move |e| Unreadable::Store(format!("{what}: {e}"))
}

/// Opens an existing database for writing (`-wal` files get taken in), creating nothing.
fn open(db: &Path) -> Result<Connection, Unreadable> {
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    Connection::open_with_flags(db, flags).map_err(store("opening the cookie store"))
}

fn chromium(profile: &Path) -> Result<Vec<Cookie>, Unreadable> {
    let Some(db) = ["Default/Cookies", "Default/Network/Cookies"]
        .into_iter()
        .map(|p| profile.join(p))
        .find(|p| p.is_file())
    else {
        return Ok(Vec::new());
    };
    let db = open(&db)?;
    let version: i64 = db
        .query_row(
            "SELECT CAST(value AS INTEGER) FROM meta WHERE key = 'version'",
            [],
            |r| r.get(0),
        )
        .map_err(store("the store's version"))?;
    let mut rows = db
        .prepare(
            "SELECT host_key, name, value, encrypted_value, path, expires_utc, has_expires, \
             is_secure, top_frame_site_key FROM cookies",
        )
        .map_err(store("the cookies table"))?;
    let rows = rows
        .query_map([], |r| {
            Ok(ChromiumRow {
                host: r.get(0)?,
                name: r.get(1)?,
                value: r.get(2)?,
                encrypted: r.get(3)?,
                path: r.get(4)?,
                expires_utc: r.get(5)?,
                has_expires: r.get(6)?,
                secure: r.get(7)?,
                top_frame_site: r.get(8)?,
            })
        })
        .map_err(store("the cookies table"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(store("a cookie"))?.cookie(version >= 24)?);
    }
    Ok(out)
}

/// One row of Chromium's `cookies` table, as read.
struct ChromiumRow {
    host: String,
    name: String,
    value: String,
    encrypted: Vec<u8>,
    path: String,
    expires_utc: i64,
    has_expires: bool,
    secure: bool,
    top_frame_site: String,
}

impl ChromiumRow {
    /// The cookie, its value decrypted; `hashed` for a store whose values start with their
    /// host's hash.
    fn cookie(self, hashed: bool) -> Result<Cookie, Unreadable> {
        let value = if self.encrypted.is_empty() {
            self.value
        } else {
            decrypt(&self.encrypted, &self.host, hashed)?
        };
        Ok(Cookie {
            name: self.name,
            value,
            domain: self.host,
            path: self.path,
            secure: self.secure,
            expires: self
                .has_expires
                .then(|| self.expires_utc as f64 / 1e6 - WINDOWS_TO_UNIX),
            partitioned: !self.top_frame_site.is_empty(),
        })
    }
}

/// A Chromium cookie's value from its `encrypted_value` (`v10`, see the module's notes).
fn decrypt(encrypted: &[u8], host: &str, hashed: bool) -> Result<String, Unreadable> {
    let Some(data) = encrypted.strip_prefix(b"v10") else {
        return Err(if encrypted.starts_with(b"v1") {
            Unreadable::Keyring
        } else {
            Unreadable::Store("a cookie with an unknown encryption".into())
        });
    };
    let undecryptable = || Unreadable::Store("a cookie didn't decrypt".into());
    let plain = cbc::Decryptor::<aes::Aes128>::new_from_slices(&V10_KEY, &V10_IV)
        .map_err(|_| undecryptable())?
        .decrypt_padded_vec::<Pkcs7>(data)
        .map_err(|_| undecryptable())?;
    let value = if hashed {
        let hash = Sha256::digest(host.as_bytes());
        plain
            .strip_prefix(hash.as_slice())
            .ok_or_else(|| Unreadable::Store("a cookie's host hash didn't match".into()))?
    } else {
        &plain[..]
    };
    String::from_utf8(value.to_vec()).map_err(|_| Unreadable::Store("a cookie isn't text".into()))
}

fn firefox(profile: &Path) -> Result<Vec<Cookie>, Unreadable> {
    let db = profile.join("cookies.sqlite");
    if !db.is_file() {
        return Ok(Vec::new());
    }
    let db = open(&db)?;
    let schema: i64 = db
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(store("the store's version"))?;
    let per_second = if schema >= 16 { 1000.0 } else { 1.0 };
    let mut rows = db
        .prepare(
            "SELECT name, value, host, path, expiry, isSecure, originAttributes FROM moz_cookies",
        )
        .map_err(store("the moz_cookies table"))?;
    let rows = rows
        .query_map([], |r| {
            let text = |i| r.get::<_, Option<String>>(i).map(Option::unwrap_or_default);
            Ok(Cookie {
                name: text(0)?,
                value: text(1)?,
                domain: text(2)?,
                path: text(3)?,
                expires: Some(r.get::<_, i64>(4)? as f64 / per_second),
                secure: r.get(5)?,
                partitioned: text(6)?.contains("partitionKey="),
            })
        })
        .map_err(store("the moz_cookies table"))?;
    rows.collect::<Result<_, _>>().map_err(store("a cookie"))
}

#[cfg(test)]
mod tests;
