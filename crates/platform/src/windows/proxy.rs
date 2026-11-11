//! The WinHTTP proxy lookup: proxy scripts and auto-detection, through the library Windows'
//! own programs ask.
//!
//! winhttp.dll is loaded by the first lookup, on the lookup thread, and not linked as an
//! import: an import would be loaded before `main` in every start, for a lookup most PCs
//! never need an answer from.

use std::ffi::c_void;
use std::sync::OnceLock;

use windows::Win32::Foundation::{ERROR_MOD_NOT_FOUND, GetLastError, GlobalFree, HGLOBAL};
use windows::Win32::Networking::WinHttp::{
    WINHTTP_ACCESS_TYPE, WINHTTP_ACCESS_TYPE_NAMED_PROXY, WINHTTP_ACCESS_TYPE_NO_PROXY,
    WINHTTP_AUTO_DETECT_TYPE_DHCP, WINHTTP_AUTO_DETECT_TYPE_DNS_A, WINHTTP_AUTOPROXY_AUTO_DETECT,
    WINHTTP_AUTOPROXY_CONFIG_URL, WINHTTP_AUTOPROXY_OPTIONS, WINHTTP_CURRENT_USER_IE_PROXY_CONFIG,
    WINHTTP_PROXY_INFO,
};
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows::core::{BOOL, PCWSTR, PWSTR, s, w};

/// What Windows' proxy settings say to ask. No `Debug`: the script's address may name a
/// user or carry a key.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Settings {
    /// "Automatically detect settings" is on.
    pub auto_detect: bool,
    /// "Use setup script" is on: the script's address.
    pub script: Option<String>,
}

/// What `GetProcAddress` hands out, and the types `winhttp.h` declares for the five
/// functions used.
type Untyped = unsafe extern "system" fn() -> isize;
type IeProxyConfig = unsafe extern "system" fn(*mut WINHTTP_CURRENT_USER_IE_PROXY_CONFIG) -> BOOL;
type Open =
    unsafe extern "system" fn(PCWSTR, WINHTTP_ACCESS_TYPE, PCWSTR, PCWSTR, u32) -> *mut c_void;
type ProxyForUrl = unsafe extern "system" fn(
    *mut c_void,
    PCWSTR,
    *mut WINHTTP_AUTOPROXY_OPTIONS,
    *mut WINHTTP_PROXY_INFO,
) -> BOOL;
type CloseHandle = unsafe extern "system" fn(*mut c_void) -> BOOL;
type SetTimeouts = unsafe extern "system" fn(*mut c_void, i32, i32, i32, i32) -> BOOL;

struct Api {
    ie_proxy_config: IeProxyConfig,
    open: Open,
    proxy_for_url: ProxyForUrl,
    close_handle: CloseHandle,
    set_timeouts: SetTimeouts,
}

/// Milliseconds for each step of fetching a script (resolving its server's name,
/// connecting, sending, receiving): a server that never answers mustn't hold up the
/// lookups queued behind it. Name resolution has no limit by default.
const STEP_TIMEOUT_MS: i32 = 5_000;

/// winhttp.dll's functions, loaded at the first call and kept for the process. `None`
/// when the library or one of them can't be had: then nothing is looked up.
fn api() -> Option<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(|| {
        // SAFETY: the name is a NUL-terminated constant, and the flag makes Windows look
        // in System32 only, so no winhttp.dll beside the exe or on the path is loaded.
        let library =
            unsafe { LoadLibraryExW(w!("winhttp.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32).ok()? };
        // SAFETY: `library` is winhttp.dll, loaded above and never freed, and the names
        // are NUL-terminated constants.
        let (ie_proxy_config, open, proxy_for_url, close_handle, set_timeouts) = unsafe {
            (
                GetProcAddress(library, s!("WinHttpGetIEProxyConfigForCurrentUser"))?,
                GetProcAddress(library, s!("WinHttpOpen"))?,
                GetProcAddress(library, s!("WinHttpGetProxyForUrl"))?,
                GetProcAddress(library, s!("WinHttpCloseHandle"))?,
                GetProcAddress(library, s!("WinHttpSetTimeouts"))?,
            )
        };
        // SAFETY: each pointer is winhttp.dll's function of that name, cast to the type
        // `winhttp.h` declares for it. Function pointers all have one size, and both
        // sides are `extern "system"`.
        unsafe {
            Some(Api {
                ie_proxy_config: std::mem::transmute::<Untyped, IeProxyConfig>(ie_proxy_config),
                open: std::mem::transmute::<Untyped, Open>(open),
                proxy_for_url: std::mem::transmute::<Untyped, ProxyForUrl>(proxy_for_url),
                close_handle: std::mem::transmute::<Untyped, CloseHandle>(close_handle),
                set_timeouts: std::mem::transmute::<Untyped, SetTimeouts>(set_timeouts),
            })
        }
    })
    .as_ref()
}

/// A WinHTTP session, closed when dropped.
struct Session<'a> {
    handle: *mut c_void,
    api: &'a Api,
}

impl Drop for Session<'_> {
    fn drop(&mut self) {
        // SAFETY: `handle` came from `WinHttpOpen`, isn't null (`ask` checks) and is
        // closed only here.
        let _ = unsafe { (self.api.close_handle)(self.handle) };
    }
}

/// A string WinHTTP allocated for its caller: read, then freed.
fn take(text: PWSTR) -> Option<String> {
    if text.is_null() {
        return None;
    }
    // SAFETY: WinHTTP returned `text`: a NUL-terminated UTF-16 string from `GlobalAlloc`
    // that the caller owns. It's read up to its NUL and freed once, here.
    unsafe {
        let read = String::from_utf16_lossy(text.as_wide());
        let _ = GlobalFree(Some(HGLOBAL(text.0.cast())));
        Some(read)
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// Windows' proxy settings for this user ("Automatically detect settings", "Use setup
/// script"). Reading them takes a few milliseconds, which is why a lookup does it and
/// not a request. The manual proxy and its exceptions come with them and are let go:
/// reqwest's matcher has read those from the registry before the first request.
pub fn settings() -> Option<Settings> {
    let api = api()?;
    let mut config = WINHTTP_CURRENT_USER_IE_PROXY_CONFIG::default();
    // SAFETY: `config` is a writable struct of the type the function fills.
    let read = unsafe { (api.ie_proxy_config)(&mut config) }.as_bool();
    let script = take(config.lpszAutoConfigUrl);
    let _ = take(config.lpszProxy);
    let _ = take(config.lpszProxyBypass);
    read.then(|| Settings {
        auto_detect: config.fAutoDetect.as_bool(),
        script: script.filter(|s| !s.trim().is_empty()),
    })
}

/// `WinHttpGetProxyForUrl` for `url`, with the script at `script`, else with the one
/// auto-detection finds (DHCP, then DNS). Windows' proxy service fetches and runs the
/// script when it's there (it is, on Windows 10 and 11); WinHTTP falls back to this
/// process without it. Windows' sign-in is never offered to a server that asks for one
/// before handing out its script (`fAutoLogonIfChallenged` stays off): with
/// auto-detection, the network picks that server.
///
/// The proxies as WinHTTP lists them (`a:8080;b:8080`), `None` for a direct connection.
/// `Err`: Windows' error number. This may take seconds.
pub fn ask(url: &str, script: Option<&str>) -> Result<Option<String>, u32> {
    let api = api().ok_or(ERROR_MOD_NOT_FOUND.0)?;
    let url = wide(url);
    let script = script.map(wide);
    // SAFETY: the agent name is a NUL-terminated constant; the proxy names may be null
    // with `WINHTTP_ACCESS_TYPE_NO_PROXY`.
    let handle = unsafe {
        (api.open)(
            w!("featherplay"),
            WINHTTP_ACCESS_TYPE_NO_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        )
    };
    if handle.is_null() {
        // SAFETY: reads the calling thread's last error, set by the failed call above.
        return Err(unsafe { GetLastError() }.0);
    }
    let session = Session { handle, api };
    let t = STEP_TIMEOUT_MS;
    // SAFETY: the session is open. A failure leaves WinHTTP's own timeouts, which is fine.
    let _ = unsafe { (api.set_timeouts)(session.handle, t, t, t, t) };
    let mut options = WINHTTP_AUTOPROXY_OPTIONS::default();
    match &script {
        Some(script) => {
            options.dwFlags = WINHTTP_AUTOPROXY_CONFIG_URL;
            options.lpszAutoConfigUrl = PCWSTR(script.as_ptr());
        }
        None => {
            options.dwFlags = WINHTTP_AUTOPROXY_AUTO_DETECT;
            options.dwAutoDetectFlags =
                WINHTTP_AUTO_DETECT_TYPE_DHCP | WINHTTP_AUTO_DETECT_TYPE_DNS_A;
        }
    }
    let mut info = WINHTTP_PROXY_INFO::default();
    // SAFETY: the session is open; `url` and `script` are NUL-terminated and outlive the
    // call; `options` and `info` are valid structs of the types the function takes.
    let answered = unsafe {
        (api.proxy_for_url)(
            session.handle,
            PCWSTR(url.as_ptr()),
            &mut options,
            &mut info,
        )
    }
    .as_bool();
    // SAFETY: reads the calling thread's last error, right after the call that set it.
    let error = unsafe { GetLastError() }.0;
    let proxies = take(info.lpszProxy);
    let _ = take(info.lpszProxyBypass);
    if !answered {
        return Err(error);
    }
    Ok(proxies.filter(|_| info.dwAccessType == WINHTTP_ACCESS_TYPE_NAMED_PROXY))
}
