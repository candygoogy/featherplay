//! The report: each step's JSON line, and the times and frames written into it.

use std::io::Write;
use std::time::{Duration, Instant};

use featherplay_platform::perf::cpu_ms;
use serde_json::{Map, Value, json};

use super::process::{Sample, sample};
use super::wait::ticking;
use super::{DECODES, Mark, QUIET, RUN_GAP, SEEN, STEP_TIMEOUT, idle_at};

pub(super) struct Out {
    pub(super) file: Option<std::fs::File>,
    /// The process's creation, as an `Instant`.
    pub(super) epoch: Instant,
    /// The display's frame interval, ms.
    pub(super) period: f64,
}

impl Out {
    pub(super) fn ms(&self, t: Instant) -> f64 {
        round((t - self.epoch).as_secs_f64() * 1000.0)
    }

    pub(super) fn write(&mut self, mut record: Map<String, Value>) {
        record.insert("t".into(), json!(self.ms(Instant::now())));
        let line = Value::Object(record).to_string();
        log!("perf: {line}");
        if let Some(f) = &mut self.file {
            let _ = writeln!(f, "{line}");
            let _ = f.flush();
        }
    }
}

pub(super) fn round(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn quantile(v: &mut [f64], q: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * q).round() as usize]
}

/// The frames of a step: how many, their paint times, and the gaps inside runs of them
/// (a gap longer than 1.5 display frames is a frame missed).
pub(super) fn frames(record: &mut Map<String, Value>, period: f64) {
    let paints = SEEN.with_borrow(|s| s.paints.clone());
    let mut cost: Vec<f64> = paints
        .iter()
        .map(|(a, b)| (*b - *a).as_secs_f64() * 1000.0)
        .collect();
    let mut gaps: Vec<f64> = paints
        .windows(2)
        .map(|w| w[1].0 - w[0].0)
        .filter(|g| *g < RUN_GAP)
        .map(|g| g.as_secs_f64() * 1000.0)
        .collect();
    let missed = gaps.iter().filter(|g| **g > period * 1.5).count();
    record.insert("frames".into(), json!(paints.len()));
    record.insert("paint_ms_total".into(), json!(round(cost.iter().sum())));
    record.insert(
        "paint_ms_p50".into(),
        json!(round(quantile(&mut cost, 0.5))),
    );
    record.insert(
        "paint_ms_p95".into(),
        json!(round(quantile(&mut cost, 0.95))),
    );
    record.insert(
        "paint_ms_max".into(),
        json!(round(quantile(&mut cost, 1.0))),
    );
    record.insert("gap_ms_max".into(), json!(round(quantile(&mut gaps, 1.0))));
    record.insert("frames_missed".into(), json!(missed));
}

/// Starts a step: forgets what earlier steps saw.
pub(super) fn begin() -> (Instant, f64) {
    SEEN.with_borrow_mut(|s| {
        s.paints.clear();
        s.marks.clear();
    });
    if let Ok(mut d) = DECODES.lock() {
        *d = (0, 0.0, 0.0, 0.0);
    }
    (Instant::now(), cpu_ms())
}

/// The threads that used the most CPU since `before`, taken as the step began at `t0` (as
/// `idle` lists them; outside the step's `cpu_ms`).
pub(super) fn threads(record: &mut Map<String, Value>, before: &Sample, t0: Instant) {
    let wall = t0.elapsed().as_secs_f64();
    record.insert(
        "busiest".into(),
        Value::Array(before.busiest_until(&sample(), wall)),
    );
}

pub(super) fn decodes(record: &mut Map<String, Value>) {
    if let Ok(d) = DECODES.lock() {
        record.insert("covers_decoded".into(), json!(d.0));
        record.insert("decode_ms_total".into(), json!(round(d.1)));
        record.insert("decode_ms_max".into(), json!(round(d.2)));
        record.insert("decode_alone_ms_total".into(), json!(round(d.3)));
    }
}

pub(super) fn first_mark(kind: Mark, after: Instant) -> Option<Instant> {
    SEEN.with_borrow(|s| {
        s.marks
            .iter()
            .find(|(m, t)| *m == kind && *t >= after)
            .map(|(_, t)| *t)
    })
}

pub(super) fn first_paint_after(after: Instant) -> Option<Instant> {
    SEEN.with_borrow(|s| s.paints.iter().find(|(a, _)| *a >= after).map(|(_, b)| *b))
}

pub(super) fn ms_since(t0: Instant, t: Option<Instant>) -> Value {
    t.map_or(Value::Null, |t| {
        json!(round((t - t0).as_secs_f64() * 1000.0))
    })
}

/// Waits for a step's work to be in and drawn: no background work pending, and a paint since
/// it dropped to none (or nothing to paint for `QUIET`); then for its animations to end.
/// Writes `first_paint`, `loaded` (first page load applied), `first_cover`, `settled` and
/// `quiet`, in ms from `t0`.
pub(super) async fn settle(
    record: &mut Map<String, Value>,
    t0: Instant,
    covers: &crate::covers::Covers,
) {
    // When the work was in, and the end of the first paint after that, if any.
    let mut done: (Instant, Option<Instant>) = (t0, None);
    let settled = ticking(STEP_TIMEOUT, || {
        if crate::busy::pending() > 0 || covers.pacing() {
            return false;
        }
        SEEN.with_borrow(|s| {
            // The last of: the work pending dropped to none, a card got its cover.
            let shown = s
                .marks
                .iter()
                .rev()
                .find(|(m, _)| *m == Mark::CoverShown)
                .map(|(_, t)| *t);
            let zero = idle_at().max(shown).filter(|z| *z >= t0).unwrap_or(t0);
            let painted = s.paints.iter().find(|(a, _)| *a >= zero).map(|(_, b)| *b);
            done = (zero, painted);
            painted.is_some() || Instant::now() - zero >= QUIET
        })
    })
    .await;
    let settled_at = if settled {
        done.1.unwrap_or(done.0)
    } else {
        record.insert("timeout".into(), json!(true));
        Instant::now()
    };
    // Then the frames stop.
    ticking(Duration::from_secs(10), || {
        SEEN.with_borrow(|s| s.last_paint.is_none_or(|p| Instant::now() - p >= QUIET))
    })
    .await;
    let quiet_at = SEEN.with_borrow(|s| s.last_paint).filter(|p| *p >= t0);
    record.insert("first_paint".into(), ms_since(t0, first_paint_after(t0)));
    record.insert("loaded".into(), ms_since(t0, first_mark(Mark::Load, t0)));
    record.insert(
        "first_cover".into(),
        ms_since(t0, first_mark(Mark::Cover, t0)),
    );
    record.insert("settled".into(), ms_since(t0, Some(settled_at)));
    record.insert(
        "quiet".into(),
        ms_since(t0, Some(quiet_at.unwrap_or(settled_at).max(settled_at))),
    );
}
