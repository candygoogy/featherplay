//! What a signed-in YouTube Music session looks like, and the `cookie:` header made from a
//! browser's cookies: `featherplay_core::session`, which the platform crate's WebView2 page
//! checks use too. Both sign-ins (`webview`, `browser`) reach it here.

pub use featherplay_core::session::*;
