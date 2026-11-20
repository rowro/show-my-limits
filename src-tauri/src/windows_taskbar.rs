use std::{cell::{Cell, OnceCell, RefCell}, mem::size_of, ptr::{null, null_mut}, slice};

use windows::Win32::{
    System::{
        Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize},
        Variant::VARIANT,
    },
    UI::Accessibility::{
        AutomationElementMode_None, CUIAutomation8, IUIAutomation2, TreeScope_Descendants,
        UIA_BoundingRectanglePropertyId, UIA_ButtonControlTypeId, UIA_CheckBoxControlTypeId,
        UIA_ControlTypePropertyId, UIA_IsKeyboardFocusablePropertyId, UIA_IsOffscreenPropertyId,
        UIA_ProcessIdPropertyId,
    },
};
use windows_sys::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM},
        Graphics::Gdi::{
            AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
            CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject,
            GetDC, MapWindowPoints, ReleaseDC, SelectObject,
        },
        System::{
            LibraryLoader::GetModuleHandleW,
            Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW},
        },
        UI::{
            HiDpi::{GetDpiForWindow, GetWindowDpiAwarenessContext, SetThreadDpiAwarenessContext},
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, FindWindowExW, FindWindowW, GetParent,
                GetWindowRect, GWL_EXSTYLE, GWL_STYLE, HWND_TOP, IDC_HAND, IsWindow,
                IsWindowVisible, LoadCursorW, MA_NOACTIVATE, SWP_FRAMECHANGED, SWP_NOACTIVATE,
                SWP_SHOWWINDOW, RegisterClassW, SetParent, SetWindowLongW,
                SetWindowPos, SetWindowTextW, ULW_ALPHA,
                UpdateLayeredWindow, WNDCLASSW, WS_CHILD, WS_CLIPSIBLINGS, WS_EX_LAYERED,
                WM_LBUTTONUP, WM_MOUSEACTIVATE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
            },
        },
    },
    w,
};

use crate::windows_tray_text;

const WINDOW_CLASS: windows_sys::core::PCWSTR = w!("ShowMyLimits.TaskbarIndicator");

// Only the Tauri UI thread creates, updates and destroys the Explorer child window.
thread_local! {
    static INDICATOR: RefCell<Indicator> = RefCell::new(Indicator::default());
    static APP: OnceCell<tauri::AppHandle> = const { OnceCell::new() };
    static TRAY_VISIBLE: Cell<bool> = const { Cell::new(true) };
}

pub fn init(app: tauri::AppHandle) {
    let _ = APP.with(|handle| handle.set(app));
}

#[derive(Default)]
struct Indicator {
    window: HWND,
    frame: Option<Frame>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Placement {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

struct Frame {
    placement: Placement,
    values: [String; 2],
    light: bool,
}

pub struct TaskbarControls {
    taskbar: isize,
    bounds: Vec<RECT>,
}

// Explorer's weather and app buttons can share a XAML window. Read their individual
// bounds through UI Automation on a worker thread, away from our window procedure.
pub fn occupied_controls() -> Option<TaskbarControls> {
    let taskbar = unsafe { FindWindowW(w!("Shell_TrayWnd"), null()) };
    if taskbar.is_null() {
        return None;
    }
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok() }.ok()?;
    let bounds = control_bounds(taskbar);
    unsafe { CoUninitialize() };
    match bounds {
        Ok(bounds) => Some(TaskbarControls {
            taskbar: taskbar as isize,
            bounds,
        }),
        Err(_error) => {
            #[cfg(debug_assertions)]
            debug_once("reading taskbar controls", _error);
            None
        }
    }
}

fn control_bounds(taskbar: HWND) -> windows::core::Result<Vec<RECT>> {
    unsafe {
        let automation: IUIAutomation2 = CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)?;
        automation.SetConnectionTimeout(500)?;
        automation.SetTransactionTimeout(500)?;
        let conditions = [
            Some(automation.CreatePropertyCondition(UIA_IsKeyboardFocusablePropertyId, &VARIANT::from(true))?),
            Some(automation.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_ButtonControlTypeId.0))?),
            Some(automation.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_CheckBoxControlTypeId.0))?),
        ];
        let interactive = automation.CreateOrConditionFromNativeArray(&conditions)?;
        let cache = automation.CreateCacheRequest()?;
        cache.SetAutomationElementMode(AutomationElementMode_None)?;
        cache.AddProperty(UIA_BoundingRectanglePropertyId)?;
        cache.AddProperty(UIA_IsOffscreenPropertyId)?;
        cache.AddProperty(UIA_ProcessIdPropertyId)?;
        let root = automation.ElementFromHandle(windows::Win32::Foundation::HWND(taskbar))?;
        let controls = root.FindAllBuildCache(TreeScope_Descendants, &interactive, &cache)?;
        let own_pid = std::process::id() as i32;
        let mut bounds = Vec::new();
        for index in 0..controls.Length()? {
            let control = controls.GetElement(index)?;
            if control.CachedProcessId()? != own_pid && !control.CachedIsOffscreen()?.as_bool() {
                let rect = control.CachedBoundingRectangle()?;
                if rect.right > rect.left && rect.bottom > rect.top {
                    bounds.push(RECT {
                        left: rect.left,
                        top: rect.top,
                        right: rect.right,
                        bottom: rect.bottom,
                    });
                }
            }
        }
        Ok(bounds)
    }
}

pub fn update(values: &[String; 2], controls: Option<&TaskbarControls>) {
    let shown = INDICATOR.with_borrow_mut(|indicator| {
        if let Some(controls) = controls {
            indicator.update(values, controls);
        } else {
            indicator.close();
        }
        indicator.frame.is_some() && unsafe { IsWindowVisible(indicator.window) != 0 }
    });
    let fallback = !shown;
    TRAY_VISIBLE.with(|visible| {
        if visible.get() == fallback {
            return;
        }
        APP.with(|app| {
            if let Some(tray) = app.get().and_then(|app| app.tray_by_id("show-my-limits")) {
                if tray.set_visible(fallback).is_ok() {
                    visible.set(fallback);
                }
            }
        });
    });
}

pub fn close() {
    INDICATOR.with_borrow_mut(Indicator::close);
}

impl Indicator {
    fn update(&mut self, values: &[String; 2], controls: &TaskbarControls) {
        let (taskbar, placement) = match taskbar_placement(controls) {
            Ok(target) => target,
            Err(_reason) => {
                #[cfg(debug_assertions)]
                debug_once("shell layout", _reason);
                self.close();
                return;
            }
        };
        // Explorer replaces its taskbar handles after a restart.
        if unsafe { GetParent(self.window) } != taskbar {
            self.close();
            match create_indicator(taskbar) {
                Ok(window) => self.window = window,
                Err((_stage, _reason)) => {
                    #[cfg(debug_assertions)]
                    debug_once(_stage, _reason);
                    return;
                }
            }
        }

        let light = taskbar_uses_light_theme();
        let repaint = self.frame.as_ref().is_none_or(|frame| {
            &frame.values != values || frame.light != light
                || frame.placement.width != placement.width
                || frame.placement.height != placement.height
        });
        let moved = self.frame.as_ref().is_none_or(|frame| frame.placement != placement);
        if !moved && !repaint {
            return;
        }
        if moved {
            // The child coordinates are relative to Explorer, so it follows auto-hide.
            let flags = SWP_NOACTIVATE | SWP_SHOWWINDOW
                | if self.frame.is_none() { SWP_FRAMECHANGED } else { 0 };
            if unsafe {
                SetWindowPos(self.window, HWND_TOP, placement.x, placement.y,
                    placement.width, placement.height, flags)
            } == 0 {
                #[cfg(debug_assertions)]
                debug_once("positioning the child", std::io::Error::last_os_error());
                self.close();
                return;
            }
        }
        if repaint {
            if !present(self.window, placement, values, light) {
                self.close();
                return;
            }
            let label = format!("Show My Limits, 5h - {}, 7d - {}", values[0], values[1]);
            let text: Vec<u16> = label.encode_utf16().chain([0]).collect();
            unsafe { SetWindowTextW(self.window, text.as_ptr()) };
        }
        #[cfg(debug_assertions)]
        if moved {
            let mut actual = RECT::default();
            unsafe { GetWindowRect(self.window, &mut actual) };
            eprintln!(
                "Show My Limits taskbar indicator: 5h - {} / 7d - {}, {}×{} at ({}, {}), visible={}",
                values[0], values[1],
                actual.right - actual.left, actual.bottom - actual.top, actual.left, actual.top,
                unsafe { IsWindowVisible(self.window) } != 0,
            );
        }
        self.frame = Some(Frame { placement, values: values.clone(), light });
    }

    fn close(&mut self) {
        if unsafe { IsWindow(self.window) } != 0 {
            unsafe { DestroyWindow(self.window) };
        }
        *self = Self::default();
    }
}

fn taskbar_placement(controls: &TaskbarControls) -> Result<(HWND, Placement), &'static str> {
    // These Explorer classes are implementation details, so keep the tray icon
    // available if the shell does not expose the expected layout.
    unsafe {
        let taskbar = FindWindowW(w!("Shell_TrayWnd"), null());
        if taskbar.is_null() || taskbar as isize != controls.taskbar || IsWindowVisible(taskbar) == 0 {
            return Err("Shell_TrayWnd unavailable");
        }
        let tray = FindWindowExW(taskbar, null_mut(), w!("TrayNotifyWnd"), null());
        let mut bar_rect = RECT::default();
        let mut tray_rect = RECT::default();
        if tray.is_null() {
            return Err("TrayNotifyWnd unavailable");
        }
        if GetWindowRect(taskbar, &mut bar_rect) == 0 || GetWindowRect(tray, &mut tray_rect) == 0 {
            return Err("window rectangles unavailable");
        }
        let mut placement = layout(bar_rect, tray_rect, GetDpiForWindow(taskbar), &controls.bounds)
            .ok_or("the taskbar has no space to the left of the notification area")?;
        let mut origin = POINT { x: placement.x, y: placement.y };
        MapWindowPoints(null_mut(), taskbar, &mut origin, 1);
        placement.x = origin.x;
        placement.y = origin.y;
        Ok((taskbar, placement))
    }
}

#[cfg(debug_assertions)]
fn debug_once(stage: &'static str, reason: impl std::fmt::Display) {
    static LAST_STAGE: std::sync::Mutex<Option<&'static str>> = std::sync::Mutex::new(None);
    if let Ok(mut last) = LAST_STAGE.lock() {
        if *last != Some(stage) {
            eprintln!("Show My Limits taskbar indicator, {stage}: {reason}");
            *last = Some(stage);
        }
    }
}

fn layout(bar: RECT, tray: RECT, dpi: u32, occupied: &[RECT]) -> Option<Placement> {
    let bar_height = bar.bottom - bar.top;
    if bar_height < 3 || bar.right - bar.left <= bar_height || dpi == 0
        || tray.left <= bar.left || tray.left >= bar.right || tray.bottom <= bar.top
        || tray.top >= bar.bottom {
        return None;
    }
    let scaled = |logical: i32| (logical * dpi as i32 + 48) / 96;
    if bar_height < scaled(30) {
        return None;
    }
    let width = scaled(76);
    let height = scaled(40).min(bar_height);
    let gap = scaled(4);
    let y = bar.top + (bar_height - height) / 2;
    let mut right = tray.left - gap;
    while right - width >= bar.left {
        let x = right - width;
        let collision = occupied
            .iter()
            .filter(|rect| {
                rect.left < right + gap
                    && rect.right + gap > x
                    && rect.top < y + height
                    && rect.bottom > y
            })
            .map(|rect| rect.left - gap)
            .min();
        match collision {
            Some(next_right) => right = next_right,
            None => return Some(Placement { x, y, width, height }),
        }
    }
    None
}

unsafe extern "system" fn indicator_window_proc(
    window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_LBUTTONUP => {
            let mut bounds = RECT::default();
            if unsafe { GetWindowRect(window, &mut bounds) } != 0 {
                let anchor = tauri::Rect {
                    position: tauri::PhysicalPosition::new(bounds.left, bounds.top).into(),
                    size: tauri::PhysicalSize::new(
                        (bounds.right - bounds.left) as u32,
                        (bounds.bottom - bounds.top) as u32,
                    ).into(),
                };
                APP.with(|app| {
                    if let Some(app) = app.get() {
                        crate::show_popover(app, Some(anchor));
                    }
                });
            }
            0
        }
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

fn create_indicator(taskbar: HWND) -> Result<HWND, (&'static str, std::io::Error)> {
    static REGISTERED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    // Match the parent's DPI context before creating a cross-process child.
    unsafe {
        let instance = GetModuleHandleW(null());
        if instance.is_null() {
            return Err(("loading the application instance", std::io::Error::last_os_error()));
        }
        if !*REGISTERED.get_or_init(|| {
            RegisterClassW(&WNDCLASSW {
                hInstance: instance,
                lpszClassName: WINDOW_CLASS,
                lpfnWndProc: Some(indicator_window_proc),
                hCursor: LoadCursorW(null_mut(), IDC_HAND),
                ..Default::default()
            }) != 0
        }) {
            return Err(("registering the window class", std::io::Error::last_os_error()));
        }
        let context = GetWindowDpiAwarenessContext(taskbar);
        let previous = SetThreadDpiAwarenessContext(context);
        if previous.is_null() {
            return Err(("matching Explorer DPI awareness", std::io::Error::last_os_error()));
        }
        let style = WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW;
        let window = CreateWindowExW(
            style | WS_EX_LAYERED, WINDOW_CLASS, w!("Show My Limits"), WS_POPUP, 0, 0, 1, 1,
            null_mut(), null_mut(), instance, null(),
        );
        let result = if window.is_null() {
            Err(("creating the indicator window", std::io::Error::last_os_error()))
        } else {
            SetWindowLongW(window, GWL_STYLE, (WS_CHILD | WS_CLIPSIBLINGS) as i32);
            SetParent(window, taskbar);
            let error = std::io::Error::last_os_error();
            if GetParent(window) == taskbar {
                // Bind the layered surface after reparenting into Explorer.
                SetWindowLongW(window, GWL_EXSTYLE, style as i32);
                SetWindowLongW(window, GWL_EXSTYLE, (style | WS_EX_LAYERED) as i32);
                Ok(window)
            } else {
                DestroyWindow(window);
                Err(("attaching the indicator to Explorer", error))
            }
        };
        SetThreadDpiAwarenessContext(previous);
        result
    }
}

fn taskbar_uses_light_theme() -> bool {
    let mut value = 0u32;
    let mut bytes = size_of::<u32>() as u32;
    unsafe {
        RegGetValueW(HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("SystemUsesLightTheme"), RRF_RT_REG_DWORD, null_mut(),
            (&mut value as *mut u32).cast(), &mut bytes) == 0 && value != 0
    }
}

fn present(window: HWND, placement: Placement, values: &[String; 2], light: bool) -> bool {
    let (width, height) = (placement.width, placement.height);
    let Some(pixels) = windows_tray_text::render(values, light, width as usize, height as usize) else {
        #[cfg(debug_assertions)]
        debug_once("drawing the text", "no glyphs were rendered");
        return false;
    };
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: width, biHeight: -height,
            biPlanes: 1, biBitCount: 32, biCompression: BI_RGB, ..Default::default()
        },
        ..Default::default()
    };
    let size = SIZE { cx: width, cy: height };
    let source = POINT { x: 0, y: 0 };
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8,
    };
    // The private bitmap contains exactly width × height premultiplied BGRA pixels.
    unsafe {
        let screen = GetDC(null_mut());
        if screen.is_null() {
            #[cfg(debug_assertions)]
            debug_once("opening the screen DC", std::io::Error::last_os_error());
            return false;
        }
        let memory = CreateCompatibleDC(screen);
        let mut bits = null_mut();
        let bitmap = if memory.is_null() { null_mut() } else {
            CreateDIBSection(memory, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0)
        };
        let presented = if bitmap.is_null() || bits.is_null() {
            #[cfg(debug_assertions)]
            debug_once("allocating the surface bitmap", std::io::Error::last_os_error());
            false
        } else {
            slice::from_raw_parts_mut(bits.cast::<u8>(), pixels.len()).copy_from_slice(&pixels);
            let previous = SelectObject(memory, bitmap);
            let success = UpdateLayeredWindow(window, screen, null(), &size, memory, &source,
                0, &blend, ULW_ALPHA) != 0;
            #[cfg(debug_assertions)]
            if !success {
                let error = std::io::Error::last_os_error();
                let style = windows_sys::Win32::UI::WindowsAndMessaging::GetWindowLongW(window, GWL_EXSTYLE);
                debug_once("updating the layered surface", format_args!("{error}, style={style:#x}"));
            }
            SelectObject(memory, previous);
            success
        };
        if !bitmap.is_null() { DeleteObject(bitmap); }
        if !memory.is_null() { DeleteDC(memory); }
        ReleaseDC(null_mut(), screen);
        presented
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligns_to_tray_boundary_and_scales_at_150_percent() {
        let bar = RECT { left: -1920, top: 1008, right: 0, bottom: 1080 };
        let tray = RECT { left: -210, top: 1008, right: 0, bottom: 1080 };
        assert_eq!(layout(bar, tray, 144, &[]), Some(Placement { x: -330, y: 1014, width: 114, height: 60 }));
        let crowded = RECT { left: -1970, ..bar };
        let tray = RECT { left: -1950, ..tray };
        assert_eq!(layout(crowded, tray, 144, &[]), None);
    }

    #[test]
    fn moves_left_of_weather_and_other_occupied_buttons() {
        let bar = RECT { left: 0, top: 1008, right: 1920, bottom: 1080 };
        let tray = RECT { left: 1740, right: 1920, ..bar };
        let weather = RECT { left: 1580, right: 1740, ..bar };
        let button = RECT { left: 1460, right: 1510, ..bar };
        let placement = layout(bar, tray, 144, &[button, weather]).unwrap();
        assert_eq!(placement.x + placement.width + 6, button.left);

        let crowded = RECT { left: 1520, ..bar };
        assert_eq!(layout(crowded, tray, 144, &[weather]), None);
    }
}
