//! What every OS's single-instance guard shares.

/// 64-bit FNV-1a: stable across builds and Rust versions, unlike `DefaultHasher`, so two
/// featherplay builds agree on a home's key.
pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}
