//! GTK owns the move gesture and may consume WebView mouse-up events.
use crate::PopoverState;
use gtk::{gdk, glib, prelude::*};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tauri::Manager;

pub fn start_drag(window: tauri::WebviewWindow) -> Result<(), String> {
    let state = window.state::<PopoverState>();
    if state.dragging.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let monitor = window.clone();
    if let Err(error) = window.run_on_main_thread(move || {
        let Ok(native) = monitor.gtk_window() else {
            monitor
                .state::<PopoverState>()
                .dragging
                .store(false, Ordering::SeqCst);
            return;
        };
        let started = Instant::now();
        glib::timeout_add_local(Duration::from_millis(50), move || {
            let pointer = native
                .display()
                .default_seat()
                .and_then(|seat| seat.pointer());
            let held = native.window().zip(pointer).map(|(surface, pointer)| {
                surface
                    .device_position(&pointer)
                    .3
                    .contains(gdk::ModifierType::BUTTON1_MASK)
            });
            // No arbitrary short grace period: keep even a long drag visible.
            // The deadline also releases the guard if the compositor loses input.
            if held == Some(true) && started.elapsed() < Duration::from_secs(120) {
                return glib::ControlFlow::Continue;
            }
            monitor
                .state::<PopoverState>()
                .dragging
                .store(false, Ordering::SeqCst);
            if monitor.is_visible().unwrap_or(false) && !native.is_active() {
                // A window-manager move can temporarily take focus away. Restore
                // it after release so the next outside click hides the panel.
                let _ = monitor.set_focus();
            }
            glib::ControlFlow::Break
        });
    }) {
        state.dragging.store(false, Ordering::SeqCst);
        return Err(error.to_string());
    }
    if let Err(error) = window.start_dragging() {
        state.dragging.store(false, Ordering::SeqCst);
        return Err(error.to_string());
    }
    Ok(())
}
