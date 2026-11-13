//! featherplay's on-disk and OS-store state: paths, settings, the saved session, secrets,
//! the Library's cache, the songs' silences.

mod atomic;
pub mod desktop_lyrics;
pub mod library_cache;
pub mod lyrics_choices;
pub mod paths;
pub mod secrets;
pub mod session;
pub mod settings;
pub mod silence_cache;

pub use atomic::write as write_atomic;
pub use desktop_lyrics::{DesktopLyrics, StripPlace, StripSize};
pub use library_cache::{LibraryCache, LibraryList};
pub use lyrics_choices::{LyricsChoice, LyricsChoices, LyricsPick};
pub use paths::Paths;
pub use secrets::{SecretKey, StoreError, delete_secret, get_secret, set_secret};
pub use session::{load_client_version, load_session, save_client_version, save_session};
pub use settings::{
    Accent, AccentColour, CloseAction, DiscordOptions, DiscordStatusText, DiscordTime, Equalizer,
    Hotkeys, Loudness, LyricsSize, Settings, SkipSilence, StreamingQuality, ThemeMode,
};
pub use silence_cache::{SilenceCache, SongSilences};
