//! Last.fm request signatures.

use std::fmt::Write;

use md5::{Digest, Md5};

/// The `api_sig` for a request: the md5 of every `name` + `value` pair sorted by name,
/// followed by the shared secret, as lowercase hex.
///
/// Pass every parameter except `format` and `callback`, which Last.fm leaves out of the signature.
pub fn api_sig(params: &[(&str, &str)], secret: &str) -> String {
    let mut sorted = params.to_vec();
    sorted.sort_unstable_by(|a, b| a.0.cmp(b.0));

    let mut hasher = Md5::new();
    for (name, value) in sorted {
        hasher.update(name.as_bytes());
        hasher.update(value.as_bytes());
    }
    hasher.update(secret.as_bytes());

    hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(32), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}
