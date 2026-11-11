//! Every integration test of `featherplay` in one test program, one module per
//! former test file, so the crate links one test executable instead of 60.
//! Run them with `cargo nextest run` (one process per test); see `.config/nextest.toml`.
//! Most of them install Slint's offscreen platform, which a process can do only once, so
//! `cargo test` runs them one at a time only: `cargo test -p featherplay --test app -- <name>`.
//! `support` holds the helpers several modules share, and starts every test in a fresh home
//! of its own (`FEATHERPLAY_HOME`).
#![allow(
    clippy::print_stderr,
    clippy::print_stdout,
    reason = "tests report skips and measurements"
)]

mod support;

mod account_menu;
mod account_picker;
mod album_actions;
mod animations;
mod bar_silences;
mod bar_speed;
mod bar_tips;
mod bar_total;
mod card_hint;
mod card_veil;
mod close_choice;
mod content_fade;
mod diagnostics;
mod discord_card;
mod first_frames;
mod first_run_toast;
mod frame;
mod glyph_fallback;
mod grid_cover;
mod hero_menu;
mod home_covers;
mod home_more;
mod home_scroll_bump;
mod home_shapes;
mod home_shelves_dropped;
mod home_video_covers;
mod home_wash;
mod hover_play;
// The Windows installer's script (Inno Setup) and the Windows names it repeats.
#[cfg(windows)]
mod installer;
mod instance;
mod library_refresh;
mod live_resize;
mod logo;
mod lyrics_arrive;
mod lyrics_button;
mod lyrics_cover;
mod lyrics_debounce;
mod lyrics_follow;
mod lyrics_frames;
mod lyrics_gate;
mod lyrics_lines;
mod lyrics_no_duplicate;
mod lyrics_none;
mod lyrics_off;
mod lyrics_outage;
mod lyrics_toggle;
mod lyrics_top;
mod lyrics_unavailable;
mod lyrics_view;
mod memory;
mod mini_player;
mod motion_idle;
mod narrow_panel;
mod neutral_accent;
mod no_device_toast;
mod not_for_me;
mod not_for_me_tint;
mod not_for_me_toast;
mod now_playing_info;
mod nowplaying_side;
mod page_ahead;
mod paused_idle;
mod playing_frames;
mod playing_from;
mod playing_mark;
mod playlist_menu;
mod playlist_pages;
mod prepare_rows;
mod present_damage;
mod press_feedback;
mod queue_panel;
mod related_scroll;
mod render_surface;
mod render_timing;
mod resize_frames;
mod resume_card;
mod row_menus;
mod scroll_frames;
mod search_link;
mod search_paste;
mod search_suggestions;
mod settings_index;
mod shortcut_list;
mod side_panel;
mod sidebar_chips;
mod sign_in_page;
mod sign_in_unsaved;
mod signed_out;
mod skeleton_frames;
mod skeleton_sweep;
mod smoothness;
mod song_menu_place;
mod song_menu_queue;
mod space_play;
mod startup_session;
mod tab_pill;
mod title_row;
mod top_search;
mod translations;
mod transport_keys;
mod tray_click;
mod tray_hide;
mod ui_bridge;
mod update_notice;
mod updates;
mod upnext_cover;
mod upnext_hover;
mod upnext_slide;
mod volume_hover;
mod volume_keys;
mod volume_wheel;
mod wheel_scroll;
