//! Toasts (ui/toast.slint): one short confirmation at a time above the bar. A new toast
//! replaces the one showing and starts its time again; hovering the toast holds it open.
//! The timer is single-shot, so nothing runs once the toast has gone.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, Timer, TimerMode};

use crate::app::Ctx;
use crate::ui::Toast;

/// How long a toast stays: plain, and with an action button.
const PLAIN: Duration = Duration::from_secs(3);
const WITH_ACTION: Duration = Duration::from_secs(5);

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
    put(ctx, text, "", None);
}

/// Shows `text` with a text button (`label`, "View") that runs `action`.
pub fn show_with(ctx: &Ctx, text: &str, label: &str, action: impl Fn(&Rc<Ctx>) + 'static) {
    put(ctx, text, label, Some(Box::new(action)));
}

fn put(ctx: &Ctx, text: &str, label: &str, action: Option<Action>) {
    let Some(ui) = ctx.window() else { return };
    let toast = ui.global::<Toast>();
    toast.set_text(text.into());
    toast.set_action(label.into());
    toast.set_open(true);
    let shown_for = if action.is_some() { WITH_ACTION } else { PLAIN };
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
