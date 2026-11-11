//! Secrets in the OS credential store (Windows Credential Manager, through
//! `featherplay_platform::credentials`), service `featherplay`.
//!
//! A Windows credential holds at most 2560 bytes, and a YouTube Music cookie header is
//! often longer. So a value is stored as UTF-8 bytes split into chunks: the first chunk
//! under the entry `<key>`, later ones under `<key>#1`, `<key>#2`, and so on. A value that
//! fits in one chunk is a single entry named `<key>`.

use std::collections::HashMap;
use std::fmt;
use std::sync::LazyLock;

use featherplay_platform::credentials;
use keyring_core::{Entry, Error};

/// Makes the OS's store the default, once, on first use: what the `keyring` crate's
/// `Entry::store_status` does on Windows.
static STORE: LazyLock<Result<(), Error>> = LazyLock::new(|| {
    keyring_core::set_default_store(credentials::store()?);
    Ok(())
});

const SERVICE: &str = "featherplay";
const CHUNK: usize = credentials::MAX_SECRET_BYTES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecretKey {
    YtCookies,
    LastfmSession,
    /// The user's own Last.fm API account, which replaces the build's.
    LastfmApiKey,
    LastfmApiSecret,
    ListenbrainzToken,
    /// The local API's bearer token.
    LocalApiToken,
}

impl SecretKey {
    fn name(self) -> &'static str {
        match self {
            SecretKey::YtCookies => "yt-cookies",
            SecretKey::LastfmSession => "lastfm-session",
            SecretKey::LastfmApiKey => "lastfm-api-key",
            SecretKey::LastfmApiSecret => "lastfm-api-secret",
            SecretKey::ListenbrainzToken => "listenbrainz-token",
            SecretKey::LocalApiToken => "local-api-token",
        }
    }
}

/// A credential store failure. Neither `Display` nor `Debug` shows secret values.
pub struct StoreError(Error);

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            // These variants hold the raw secret bytes; don't format them.
            Error::BadEncoding(_) | Error::BadDataFormat(..) => {
                f.write_str("credential store: badly encoded secret")
            }
            e => write!(f, "credential store: {e}"),
        }
    }
}

/// The derived `Debug` would print `keyring_core::Error`'s, which shows the secret bytes of
/// `BadEncoding` and `BadDataFormat`. This one shows the `Display` text.
impl fmt::Debug for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StoreError({self})")
    }
}

impl std::error::Error for StoreError {}

impl From<Error> for StoreError {
    fn from(e: Error) -> Self {
        StoreError(e)
    }
}

/// The secret, or `None` if it isn't set or the store can't be read.
pub fn get_secret(k: SecretKey) -> Option<String> {
    read(k.name())
}

pub fn set_secret(k: SecretKey, v: &str) -> Result<(), StoreError> {
    write(k.name(), v)
}

/// Deletes the secret. Deleting a secret that isn't set is not an error.
pub fn delete_secret(k: SecretKey) -> Result<(), StoreError> {
    remove_from(k.name(), 0)
}

fn entry(name: &str, chunk: usize) -> Result<Entry, Error> {
    if STORE.is_err() {
        return Err(Error::NoDefaultStore);
    }
    let modifiers: HashMap<&str, &str> = credentials::MODIFIERS.iter().copied().collect();
    if chunk == 0 {
        Entry::new_with_modifiers(SERVICE, name, &modifiers)
    } else {
        Entry::new_with_modifiers(SERVICE, &format!("{name}#{chunk}"), &modifiers)
    }
}

fn read(name: &str) -> Option<String> {
    let mut bytes = Vec::new();
    for chunk in 0.. {
        match entry(name, chunk).ok()?.get_secret() {
            Ok(part) => bytes.extend_from_slice(&part),
            Err(Error::NoEntry) if chunk > 0 => break,
            Err(_) => return None,
        }
    }
    String::from_utf8(bytes).ok()
}

fn write(name: &str, v: &str) -> Result<(), StoreError> {
    let bytes = v.as_bytes();
    let chunks: Vec<&[u8]> = if bytes.is_empty() {
        vec![bytes]
    } else {
        bytes.chunks(CHUNK).collect()
    };
    for (i, part) in chunks.iter().enumerate() {
        entry(name, i)?.set_secret(part)?;
    }
    // Drop chunks left over from a longer previous value.
    remove_from(name, chunks.len())
}

/// Deletes chunk `first` and every chunk after it, stopping at the first missing one.
fn remove_from(name: &str, first: usize) -> Result<(), StoreError> {
    for chunk in first.. {
        match entry(name, chunk)?.delete_credential() {
            Ok(()) => {}
            Err(Error::NoEntry) => break,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_errors_never_show_the_secret() {
        let secret = b"SAPISID=secret-value".to_vec();
        let errors = [
            StoreError(Error::BadEncoding(secret.clone())),
            StoreError(Error::BadDataFormat(secret, "bad".into())),
        ];
        for e in &errors {
            assert!(!format!("{e}").contains("secret-value"));
            assert!(!format!("{e:?}").contains("secret-value"));
            assert!(
                !format!("{e:?}").contains("83, 65, 80"),
                "raw bytes in {e:?}"
            );
        }
    }

    /// The uninstaller deletes the secrets by exact name, so it lists them all, under the
    /// same service (packaging/windows/featherplay.iss).
    #[test]
    fn the_uninstaller_deletes_every_secret() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../packaging/windows/featherplay.iss"
        );
        let iss = std::fs::read_to_string(path).expect("read the installer script");
        let names: Vec<&str> = iss
            .lines()
            .find_map(|l| l.trim().strip_prefix("SecretNames = '"))
            .and_then(|l| l.strip_suffix("';"))
            .expect("SecretNames in the installer script")
            .split(',')
            .collect();
        // A key added to the enum must be added here (the match won't compile without
        // it) and to the installer's list.
        let all = [
            SecretKey::YtCookies,
            SecretKey::LastfmSession,
            SecretKey::LastfmApiKey,
            SecretKey::LastfmApiSecret,
            SecretKey::ListenbrainzToken,
            SecretKey::LocalApiToken,
        ];
        for k in all {
            match k {
                SecretKey::YtCookies
                | SecretKey::LastfmSession
                | SecretKey::LastfmApiKey
                | SecretKey::LastfmApiSecret
                | SecretKey::ListenbrainzToken
                | SecretKey::LocalApiToken => {}
            }
            assert!(
                names.contains(&k.name()),
                "{} isn't in SecretNames",
                k.name()
            );
        }
        assert!(
            iss.contains(&format!("#define CredService \"{SERVICE}\"")),
            "the installer's CredService isn't {SERVICE}"
        );
    }

    /// Touches the real Credential Manager, under a test-only name. Run with `--ignored`.
    #[test]
    #[ignore = "writes to the OS credential store"]
    fn live_set_get_delete() {
        const NAME: &str = "store-live-test";
        let long: String = "k=v; ".repeat(1100) + "é";
        assert!(long.len() > 2 * CHUNK);

        write(NAME, &long).unwrap();
        assert_eq!(read(NAME).as_deref(), Some(long.as_str()));
        for chunk in 0..3 {
            let attrs = entry(NAME, chunk).unwrap().get_attributes().unwrap();
            for &(name, value) in credentials::MODIFIERS {
                assert_eq!(attrs[name], value, "chunk {chunk}");
            }
        }

        write(NAME, "short").unwrap();
        assert_eq!(read(NAME).as_deref(), Some("short"));
        assert!(matches!(
            entry(NAME, 1).unwrap().get_secret(),
            Err(Error::NoEntry)
        ));

        remove_from(NAME, 0).unwrap();
        assert_eq!(read(NAME), None);
        remove_from(NAME, 0).unwrap();
    }
}
