//! featherplay's on-disk and OS-store state: paths, settings, the saved session, secrets,
//! the Library's cache, the songs' silences.

mod atomic;
pub mod library_cache;
pub mod paths;
pub mod secrets;
pub mod session;
pub mod settings;
pub mod silence_cache;

pub use library_cache::{LibraryCache, LibraryList};
pub use paths::Paths;
pub use secrets::{SecretKey, StoreError, delete_secret, get_secret, set_secret};
pub use session::{load_client_version, load_session, save_client_version, save_session};
pub use settings::{
    Accent, AccentColour, CloseAction, DiscordOptions, DiscordStatusText, DiscordTime, Equalizer,
    Hotkeys, Loudness, Settings, SkipSilence, StreamingQuality, ThemeMode,
};
pub use silence_cache::{SilenceCache, SongSilences};
