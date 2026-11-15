use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder,
};

use crate::engine::model::ProviderSnapshot;
use crate::MAIN_WINDOW;

pub const TRAY_ID: &str = "main-tray";
pub const POPUP_WINDOW: &str = "popup";
pub const EVENT_REFRESH_REQUESTED: &str = "refresh-requested";

const MENU_OPEN: &str = "open";
const MENU_REFRESH: &str = "refresh";
const MENU_QUIT: &str = "quit";

const POPUP_WIDTH: f64 = 340.0;
const POPUP_HEIGHT: f64 = 252.0;
const POPUP_MARGIN: f64 = 8.0;
/// Windows truncates tray tooltips at 127 characters.
const TOOLTIP_MAX_CHARS: usize = 120;

/// When the popup was last hidden. A click on the tray icon first blurs the
/// popup (hiding it) and then arrives as a click; without this it would reopen.
static LAST_POPUP_HIDE: Mutex<Option<Instant>> = Mutex::new(None);

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, MENU_OPEN, "Open Dashboard", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, MENU_REFRESH, "Refresh", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &refresh, &separator, &quit])?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("AI Usage Monitor")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            MENU_OPEN => show_dashboard(app),
            MENU_REFRESH => {
                let _ = app.emit(EVENT_REFRESH_REQUESTED, ());
            }
            MENU_QUIT => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                position,
                ..
            } => toggle_popup(tray.app_handle(), position),
            TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } => {
                hide_popup(tray.app_handle());
                show_dashboard(tray.app_handle());
            }
            _ => {}
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder.build(app)?;
    Ok(())
}

/// Creates the (hidden) popup window shown next to the tray icon.
pub fn create_popup(app: &AppHandle) -> tauri::Result<()> {
    WebviewWindowBuilder::new(app, POPUP_WINDOW, WebviewUrl::App("index.html".into()))
        .title("AI Usage Monitor")
        .inner_size(POPUP_WIDTH, POPUP_HEIGHT)
        .decorations(false)
        .resizable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .visible(false)
        .build()?;
    Ok(())
}

pub fn show_dashboard(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub fn hide_popup(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(POPUP_WINDOW) {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
            *LAST_POPUP_HIDE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
        }
    }
}

/// Top-left corner for the popup: centred on the click, just above it, and
/// kept inside the monitor's work area (so it hugs the taskbar on any edge).
/// `work` is `(x, y, width, height)`; everything is in physical pixels.
pub fn popup_origin(
    click: (f64, f64),
    popup: (f64, f64),
    work: (f64, f64, f64, f64),
) -> (i32, i32) {
    let (work_x, work_y, work_w, work_h) = work;
    let min_x = work_x + POPUP_MARGIN;
    let min_y = work_y + POPUP_MARGIN;
    let max_x = (work_x + work_w - popup.0 - POPUP_MARGIN).max(min_x);
    let max_y = (work_y + work_h - popup.1 - POPUP_MARGIN).max(min_y);
    let x = (click.0 - popup.0 / 2.0).clamp(min_x, max_x);
    let y = (click.1 - popup.1 - POPUP_MARGIN).clamp(min_y, max_y);
    (x.round() as i32, y.round() as i32)
}

fn toggle_popup(app: &AppHandle, click: PhysicalPosition<f64>) {
    let Some(window) = app.get_webview_window(POPUP_WINDOW) else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        hide_popup(app);
        return;
    }
    let just_hidden = LAST_POPUP_HIDE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some_and(|at| at.elapsed() < Duration::from_millis(300));
    if just_hidden {
        return;
    }

    let monitor = window
        .monitor_from_point(click.x, click.y)
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten());
    if let (Some(monitor), Ok(size)) = (monitor, window.outer_size()) {
        let area = monitor.work_area();
        let (x, y) = popup_origin(
            (click.x, click.y),
            (f64::from(size.width), f64::from(size.height)),
            (
                f64::from(area.position.x),
                f64::from(area.position.y),
                f64::from(area.size.width),
                f64::from(area.size.height),
            ),
        );
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }
    let _ = window.show();
    let _ = window.set_focus();
}

/// One-line summary for the tray tooltip: each provider with its tightest window.
pub fn tooltip_text(snapshots: &[ProviderSnapshot]) -> String {
    let parts: Vec<String> = snapshots
        .iter()
        .map(|snapshot| {
            let tightest = snapshot
                .windows
                .iter()
                .filter_map(|window| window.used_percent)
                .fold(None, |best: Option<f32>, used| {
                    Some(best.map_or(used, |best| best.max(used)))
                });
            match tightest {
                Some(used) => format!("{} {used:.0}%", snapshot.provider.short_name()),
                None => format!("{} –", snapshot.provider.short_name()),
            }
        })
        .collect();
    let text = if parts.is_empty() {
        "AI Usage Monitor".to_string()
    } else {
        format!("AI Usage Monitor\n{}", parts.join(" · "))
    };
    text.chars().take(TOOLTIP_MAX_CHARS).collect()
}

pub fn update_tooltip(app: &AppHandle, snapshots: &[ProviderSnapshot]) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(tooltip_text(snapshots)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::model::{ProviderId, QuotaWindow};
    use chrono::Utc;

    const WORK: (f64, f64, f64, f64) = (0.0, 0.0, 1920.0, 1040.0);
    const POPUP: (f64, f64) = (340.0, 252.0);

    #[test]
    fn popup_sits_above_a_bottom_taskbar() {
        // Click inside the taskbar, below the work area.
        assert_eq!(popup_origin((1800.0, 1060.0), POPUP, WORK), (1572, 780));
        // Centred on the click when there is room.
        assert_eq!(popup_origin((900.0, 1060.0), POPUP, WORK), (730, 780));
    }

    #[test]
    fn popup_stays_inside_the_work_area() {
        // Taskbar at the top: work area starts at y = 40.
        let top = (0.0, 40.0, 1920.0, 1040.0);
        assert_eq!(popup_origin((1800.0, 20.0), POPUP, top), (1572, 48));
        // Taskbar on the left edge of a second monitor at x = 1920.
        let second = (1968.0, 0.0, 1872.0, 1080.0);
        assert_eq!(popup_origin((1940.0, 1000.0), POPUP, second), (1976, 740));
        // Work area smaller than the popup never panics.
        assert_eq!(
            popup_origin((5.0, 5.0), POPUP, (0.0, 0.0, 100.0, 100.0)),
            (8, 8)
        );
    }

    #[test]
    fn tooltip_lists_the_tightest_window_per_provider() {
        let now = Utc::now();
        let mut claude = ProviderSnapshot::empty(ProviderId::Claude, now);
        claude.windows = vec![
            QuotaWindow::percent("five_hour", "5 hours", 37.4),
            QuotaWindow::percent("weekly", "Weekly", 62.6),
        ];
        let copilot = ProviderSnapshot::empty(ProviderId::Copilot, now);
        assert_eq!(
            tooltip_text(&[claude.clone(), copilot]),
            "AI Usage Monitor\nClaude 63% · Copilot –"
        );
        assert_eq!(tooltip_text(&[]), "AI Usage Monitor");

        let many: Vec<ProviderSnapshot> = std::iter::repeat_n(claude, 40).collect();
        assert_eq!(tooltip_text(&many).chars().count(), TOOLTIP_MAX_CHARS);
    }
}
