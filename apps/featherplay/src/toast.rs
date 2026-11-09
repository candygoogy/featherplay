//! Toasts (ui/toast.slint): one short line at a time above the bar, a confirmation
//! (neutral) or something gone wrong (error). A new toast replaces the one showing and
//! starts its time again; hovering the toast, or its button holding the keyboard focus,
//! holds it open. The timer is single-shot, so nothing runs once the toast has gone.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, Timer, TimerMode};

use crate::app::Ctx;
use crate::ui::{Toast, ToastTone};

/// How long a toast stays (unless `show_for` says): a plain confirmation, and one with an
/// action button or saying something went wrong, which takes longer to read and act on.
const PLAIN: Duration = Duration::from_secs(3);
const LONGER: Duration = Duration::from_secs(5);

type Action = Box<dyn Fn(&Rc<Ctx>)>;

thread_local! {
    static TIMER: Timer = Timer::default();
    static SHOWN_FOR: Cell<Duration> = const { Cell::new(PLAIN) };
    static ACTION: RefCell<Option<Action>> = RefCell::new(None);
}

pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let toast = ui.global::<Toast>();
    toast.on_hover_changed(|over| {
        if over {
            TIMER.with(|t| t.stop());
        } else if Ctx::get()
            .and_then(|c| c.window())
            .is_some_and(|ui| ui.global::<Toast>().get_open())
        {
            arm(SHOWN_FOR.with(|d| d.get()));
        }
    });
    let c = ctx.clone();
    toast.on_action_clicked(move || {
        // Taken before `close`, which drops it.
        let action = ACTION.with(|a| a.borrow_mut().take());
        close();
        if let Some(action) = action {
            action(&c);
        }
    });
}

/// Shows `text` ("Added to queue: Da Funk").
pub fn show(ctx: &Ctx, text: &str) {
    put(ctx, ToastTone::Neutral, text, "", None, PLAIN);
}

/// Shows `text` with a text button (`label`, "View") that runs `action`.
pub fn show_with(ctx: &Ctx, text: &str, label: &str, action: impl Fn(&Rc<Ctx>) + 'static) {
    put(
        ctx,
        ToastTone::Neutral,
        text,
        label,
        Some(Box::new(action)),
        LONGER,
    );
}

/// Shows that something went wrong (`text`, "Couldn't copy the token. Try again.").
pub fn show_error(ctx: &Ctx, text: &str) {
    put(ctx, ToastTone::Error, text, "", None, LONGER);
}

/// `show_error` with a text button (`label`, "Get it") that runs `action`.
pub fn show_error_with(ctx: &Ctx, text: &str, label: &str, action: impl Fn(&Rc<Ctx>) + 'static) {
    put(
        ctx,
        ToastTone::Error,
        text,
        label,
        Some(Box::new(action)),
        LONGER,
    );
}

/// A toast with a button, for `shown_for` (news that comes once and unasked, as an
/// update's).
pub fn show_for(
    ctx: &Ctx,
    tone: ToastTone,
    text: &str,
    label: &str,
    action: impl Fn(&Rc<Ctx>) + 'static,
    shown_for: Duration,
) {
    put(ctx, tone, text, label, Some(Box::new(action)), shown_for);
}

fn put(
    ctx: &Ctx,
    tone: ToastTone,
    text: &str,
    label: &str,
    action: Option<Action>,
    shown_for: Duration,
) {
    let Some(ui) = ctx.window() else { return };
    let toast = ui.global::<Toast>();
    toast.set_tone(tone);
    toast.set_text(text.into());
    toast.set_action(label.into());
    // Counted, so a new toast while one shows is seen as new (its line comes up again).
    toast.set_shown(toast.get_shown().wrapping_add(1));
    toast.set_open(true);
    ACTION.with(|a| *a.borrow_mut() = action);
    SHOWN_FOR.with(|d| d.set(shown_for));
    arm(shown_for);
}

fn arm(after: Duration) {
    TIMER.with(|t| t.start(TimerMode::SingleShot, after, close));
}

fn close() {
    TIMER.with(|t| t.stop());
    ACTION.with(|a| a.borrow_mut().take());
    if let Some(ui) = Ctx::get().and_then(|c| c.window()) {
        ui.global::<Toast>().set_open(false);
    }
}
