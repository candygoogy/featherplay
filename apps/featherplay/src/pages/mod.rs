//! One module per page, each with the same four entry points: `init` (once, connects the
//! page's global), `open` (each time the router shows it), `player_changed` and
//! `account_changed`. This file only dispatches; page owners work in their own module.

use std::rc::Rc;

use crate::app::Ctx;
use crate::controller::PlayerState;
use crate::router::Page;

pub mod about;
pub mod ahead;
pub mod album;
pub mod artist;
pub mod content;
pub mod explore;
pub mod grid;
pub mod history;
pub mod home;
pub mod library;
pub mod now_playing;
pub mod playlist;
pub mod search;
pub mod settings;
pub mod sign_in;

pub fn init(ctx: &Rc<Ctx>) {
    ahead::init(ctx);
    home::init(ctx);
    explore::init(ctx);
    grid::init(ctx);
    history::init(ctx);
    library::init(ctx);
    album::init(ctx);
    playlist::init(ctx);
    artist::init(ctx);
    search::init(ctx);
    now_playing::init(ctx);
    settings::init(ctx);
    about::init(ctx);
    sign_in::init(ctx);
}

pub fn open(ctx: &Rc<Ctx>, page: &Page) {
    // Its card is gone: what's read ahead for another page needn't come.
    ahead::shown(page);
    match page {
        Page::Home => home::open(ctx),
        Page::Explore { tab } => explore::open(ctx, *tab),
        Page::Mood { params, title } => explore::open_mood(ctx, params, title),
        Page::History => history::open(ctx),
        Page::Library { tab } => library::open(ctx, *tab),
        Page::Album { id } => album::open(ctx, id),
        Page::Playlist { id } => playlist::open(ctx, id),
        Page::Artist { id } => artist::open(ctx, id),
        Page::Search { query } => search::open(ctx, query),
        Page::NowPlaying => now_playing::open(ctx),
        Page::Settings => settings::open(ctx),
        Page::About => about::open(ctx),
        Page::SignIn => sign_in::open(ctx),
    }
}

pub fn player_changed(ctx: &Rc<Ctx>, state: &PlayerState) {
    home::player_changed(ctx, state);
    explore::player_changed(ctx, state);
    history::player_changed(ctx, state);
    library::player_changed(ctx, state);
    album::player_changed(ctx, state);
    playlist::player_changed(ctx, state);
    artist::player_changed(ctx, state);
    search::player_changed(ctx, state);
    now_playing::player_changed(ctx, state);
    settings::player_changed(ctx, state);
    about::player_changed(ctx, state);
    sign_in::player_changed(ctx, state);
}

pub fn account_changed(ctx: &Rc<Ctx>, signed_in: bool) {
    ahead::clear();
    home::account_changed(ctx, signed_in);
    explore::account_changed(ctx, signed_in);
    history::account_changed(ctx, signed_in);
    library::account_changed(ctx, signed_in);
    album::account_changed(ctx, signed_in);
    playlist::account_changed(ctx, signed_in);
    artist::account_changed(ctx, signed_in);
    search::account_changed(ctx, signed_in);
    now_playing::account_changed(ctx, signed_in);
    settings::account_changed(ctx, signed_in);
    about::account_changed(ctx, signed_in);
    sign_in::account_changed(ctx, signed_in);
}

/// The app acts as another identity of the session: Home's feed is fetched again, and the
/// app goes to Home, as YouTube Music does; the page that was showing may belong to the
/// previous identity (a private playlist).
pub fn identity_changed(ctx: &Rc<Ctx>) {
    ahead::clear();
    // Drops the feed; reopens Home when it's showing.
    home::account_changed(ctx, true);
    library::forget(ctx);
    explore::account_changed(ctx, true);
    history::account_changed(ctx, true);
    if ctx.current_page() != Page::Home {
        ctx.navigate(Page::Home);
    }
}
