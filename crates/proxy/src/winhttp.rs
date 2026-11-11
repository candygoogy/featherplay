//! Windows' side of the lookups: WinHTTP, the library Windows' own programs ask
//! (`featherplay_platform::proxy`).

use featherplay_platform::proxy;

use crate::lookups::{Sources, System};

/// The system's WinHTTP.
pub(crate) struct WinHttp;

impl System for WinHttp {
    fn sources(&self) -> Sources {
        proxy::settings()
            .map(|s| Sources {
                auto_detect: s.auto_detect,
                script: s.script,
            })
            .unwrap_or_default()
    }

    fn ask(&self, url: &str, script: Option<&str>) -> Result<Option<String>, u32> {
        proxy::ask(url, script)
    }
}
