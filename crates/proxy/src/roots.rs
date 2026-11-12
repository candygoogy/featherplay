//! The certificates a client trusts, and the client for when none can be built.
//!
//! Windows: SChannel's, which is Windows' store. Elsewhere: the system's store, as
//! rustls-platform-verifier reads it (`SSL_CERT_FILE`/`SSL_CERT_DIR`, else the
//! distribution's bundle), so a company's or the user's own CA keeps working. A system
//! without one (a container with no `ca-certificates`) gets the roots built into the
//! binary instead, Mozilla's, rather than a client that can't be built.

use std::io;
use std::sync::Arc;

use reqwest::dns::{Name, Resolve, Resolving};

/// `reqwest::Client::builder()` with the roots this system can give. Every client starts
/// from it, `crate::client`'s and `doh`'s.
pub(crate) fn builder() -> reqwest::ClientBuilder {
    let builder = reqwest::Client::builder();
    #[cfg(not(windows))]
    if let Some(roots) = &*built_in::FALLBACK {
        return builder.tls_certs_only(roots.iter().cloned());
    }
    builder
}

#[cfg(not(windows))]
mod built_in {
    use std::sync::LazyLock;

    use reqwest::Certificate;

    /// The built-in roots when the system has none; `None` when it has some. Read once,
    /// at the first client.
    pub(super) static FALLBACK: LazyLock<Option<Vec<Certificate>>> = LazyLock::new(|| {
        let system = rustls_native_certs::load_native_certs();
        let roots = fallback(&system.certs);
        if roots.is_some() {
            log::warn!("tls: no system certificates found; using built-in roots");
        }
        roots
    });

    /// `None` for a system store with certificates in it, else Mozilla's roots.
    pub(super) fn fallback<C>(system: &[C]) -> Option<Vec<Certificate>> {
        if !system.is_empty() {
            return None;
        }
        Some(
            webpki_root_certs::TLS_SERVER_ROOT_CERTS
                .iter()
                .filter_map(|der| Certificate::from_der(der).ok())
                .collect(),
        )
    }
}

/// A client that reaches nothing: every request fails at once, as with no network. For
/// where a client of `client()` can't be built and the app goes on as if offline. It
/// trusts no certificate, takes no proxy and looks up no name, so nothing it needs can be
/// missing; it fails to build only where no client of any kind can be built.
pub fn offline() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .dns_resolver(Arc::new(NoNames))
        .tls_certs_only([])
        .build()
        .expect("a client with no proxy, no names and no certificates")
}

/// `offline`'s resolver: every name fails.
struct NoNames;

impl Resolve for NoNames {
    fn resolve(&self, _: Name) -> Resolving {
        Box::pin(async {
            Err(Box::new(io::Error::new(
                io::ErrorKind::NotConnected,
                "offline: no HTTP client could be built",
            )) as Box<dyn std::error::Error + Send + Sync>)
        })
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn offline_fails_every_request() {
        let err = super::offline()
            .get("https://music.youtube.com/")
            .send()
            .await
            .unwrap_err();
        assert!(err.is_connect(), "{err:?}");
    }

    #[cfg(not(windows))]
    #[test]
    fn no_system_store_builds_on_the_built_in_roots() {
        let roots = super::built_in::fallback::<()>(&[]).expect("the built-in roots");
        assert!(roots.len() > 100, "{} roots", roots.len());
        reqwest::Client::builder()
            .tls_certs_only(roots)
            .build()
            .expect("a client on the built-in roots");
    }

    #[cfg(not(windows))]
    #[test]
    fn a_system_store_is_kept() {
        let system = [webpki_root_certs::TLS_SERVER_ROOT_CERTS[0].clone()];
        assert!(super::built_in::fallback(&system).is_none());
    }
}
