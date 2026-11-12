//! What the mini player's placement asks the X server, over a connection of its own made for
//! each question (a click on the tray icon is rare): the pointer, the monitors (RandR), the
//! work area the window manager leaves beside its panels (`_NET_WORKAREA`), and the scale
//! winit gives windows (`WINIT_X11_SCALE_FACTOR`, else `Xft.dpi`).

use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, Window};
use x11rb::rust_connection::RustConnection;

use crate::common::window::ScreenRect;

fn connect() -> Result<(RustConnection, Window), String> {
    let (conn, screen) = x11rb::connect(None).map_err(|e| format!("X display: {e}"))?;
    let root = conn
        .setup()
        .roots
        .get(screen)
        .ok_or("X display: no such screen")?
        .root;
    Ok((conn, root))
}

/// Where the pointer is on the screen.
pub fn pointer() -> Result<(i32, i32), String> {
    let (conn, root) = connect()?;
    let p = conn
        .query_pointer(root)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| e.to_string())?;
    Ok((p.root_x.into(), p.root_y.into()))
}

/// The work area of the monitor at `(x, y)` (the nearest one if none holds it), and the
/// scale.
pub fn work_area_at(x: i32, y: i32) -> Result<(ScreenRect, f32), String> {
    let (conn, root) = connect()?;
    let monitors: Vec<ScreenRect> = conn
        .randr_get_monitors(root, true)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| e.to_string())?
        .monitors
        .iter()
        .map(|m| {
            let (left, top) = (i32::from(m.x), i32::from(m.y));
            (
                left,
                top,
                left + i32::from(m.width),
                top + i32::from(m.height),
            )
        })
        .collect();
    let monitor = nearest(&monitors, (x, y)).ok_or("no monitor")?;
    let work = net_workarea(&conn, root).unwrap_or(monitor);
    Ok((within(monitor, work), scale(&conn)))
}

/// The monitor holding `point`, else the one nearest it.
fn nearest(monitors: &[ScreenRect], (x, y): (i32, i32)) -> Option<ScreenRect> {
    let distance = |&(l, t, r, b): &ScreenRect| {
        let dx = (l - x).max(x - (r - 1)).max(0);
        let dy = (t - y).max(y - (b - 1)).max(0);
        i64::from(dx).pow(2) + i64::from(dy).pow(2)
    };
    monitors.iter().copied().min_by_key(distance)
}

/// The window manager's work area for the current desktop, which spans every monitor.
fn net_workarea(conn: &RustConnection, root: Window) -> Option<ScreenRect> {
    let atom = |name: &[u8]| Some(conn.intern_atom(true, name).ok()?.reply().ok()?.atom);
    let cardinals = |name: &[u8]| -> Option<Vec<u32>> {
        let reply = conn
            .get_property(false, root, atom(name)?, AtomEnum::CARDINAL, 0, 4 * 64)
            .ok()?
            .reply()
            .ok()?;
        Some(reply.value32()?.collect())
    };
    let desktop = cardinals(b"_NET_CURRENT_DESKTOP")
        .and_then(|d| d.first().copied())
        .unwrap_or(0) as usize;
    let area = cardinals(b"_NET_WORKAREA")?;
    let [x, y, w, h] = area.get(desktop * 4..desktop * 4 + 4)? else {
        return None;
    };
    let (x, y) = (*x as i32, *y as i32);
    Some((x, y, x + *w as i32, y + *h as i32))
}

/// The part of `monitor` inside the work area, or the whole monitor when the work area
/// misses it (it is the bounding box of every monitor's, so it may).
fn within(monitor: ScreenRect, work: ScreenRect) -> ScreenRect {
    let (l, t, r, b) = (
        monitor.0.max(work.0),
        monitor.1.max(work.1),
        monitor.2.min(work.2),
        monitor.3.min(work.3),
    );
    if l < r && t < b {
        (l, t, r, b)
    } else {
        monitor
    }
}

/// The scale winit gives windows on X11: `WINIT_X11_SCALE_FACTOR` when it's a number, else
/// the resources' `Xft.dpi` over 96, else 1.
fn scale(conn: &RustConnection) -> f32 {
    let positive = |s: &str| s.trim().parse::<f32>().ok().filter(|v| *v > 0.0);
    if let Some(s) = std::env::var("WINIT_X11_SCALE_FACTOR")
        .ok()
        .and_then(|v| positive(&v))
    {
        return s;
    }
    x11rb::resource_manager::new_from_default(conn)
        .ok()
        .and_then(|db| db.get_string("Xft.dpi", "").and_then(positive))
        .map_or(1.0, |dpi| dpi / 96.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_monitor_at_the_point_or_the_nearest_one() {
        let two = [(0, 0, 1920, 1080), (1920, 0, 4480, 1440)];
        assert_eq!(nearest(&two, (2000, 100)), Some(two[1]));
        // A panel's icon just below the left monitor's edge.
        assert_eq!(nearest(&two, (100, 1100)), Some(two[0]));
        assert_eq!(nearest(&[], (0, 0)), None);
    }

    #[test]
    fn the_work_area_is_cut_to_the_monitor() {
        // A 32 px panel at the bottom of the left monitor; the work area spans both.
        let work = (0, 0, 4480, 1048);
        assert_eq!(within((0, 0, 1920, 1080), work), (0, 0, 1920, 1048));
        assert_eq!(within((0, 2000, 100, 2100), work), (0, 2000, 100, 2100));
    }
}
