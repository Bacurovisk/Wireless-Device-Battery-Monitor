//! BatteryHub Taskbar. Runs in the background (the installer can add it to autostart)
//! and draws "🎧 25%  🖱 32%" straight onto the taskbar, left of the notification area, so no
//! hovering is needed. Levels reload every minute. Left click does nothing; right click offers
//! "Refresh devices", "Refresh battery levels" and "Exit".
#![windows_subsystem = "windows"]

use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::ffi::c_void;
use std::time::Instant;

use batteryhub::settings;
use batteryhub::ui::{self, rgb, split};
use batteryhub::DeviceStatus;
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcess, SetProcessWorkingSetSize};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Controls::{
    TOOLTIPS_CLASSW, TTF_ABSOLUTE, TTF_TRACK, TTM_ADDTOOLW, TTM_GETBUBBLESIZE, TTM_SETMAXTIPWIDTH, TTM_TRACKACTIVATE,
    TTM_TRACKPOSITION, TTM_UPDATETIPTEXTW, TTS_ALWAYSTIP, TTS_NOPREFIX, TTTOOLINFOW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows::Win32::UI::WindowsAndMessaging::*;

const WM_POLL_DONE: u32 = WM_APP + 2;
// Not exported by the windows crate's WindowsAndMessaging module.
const WM_MOUSELEAVE: u32 = 0x02A3;
const TIMER_POLL: usize = 1;
const TIMER_PLACE: usize = 2;
const TIMER_DEVCHANGE: usize = 3;
const WM_DEVICECHANGE_MSG: u32 = 0x0219;
const DBT_DEVNODES_CHANGED: usize = 0x0007;
const PBT_APMRESUMEAUTOMATIC: usize = 0x0012;
/// Battery refresh intervals offered in the menu, in minutes.
const INTERVALS: [(u32, &str); 5] = [(1, "1 minute"), (5, "5 minutes"), (10, "10 minutes"), (30, "30 minutes"), (60, "1 hour")];
const ID_INTERVAL_BASE: usize = 10;
const ID_ON_CHANGE: usize = 20;
const MIN_GRACE_SEC: u64 = 150;
const ID_EXIT: usize = 1;
const ID_REFRESH_DEVICES: usize = 2;
const ID_REFRESH_LEVELS: usize = 3;

pub const MAIN_CLASS: windows::core::PCWSTR = w!("BatteryHub.Taskbar.Main");
const STRIP_CLASS: windows::core::PCWSTR = w!("BatteryHub.Taskbar.Strip");
const SETTINGS_KEY: &str = batteryhub::install::TASKBAR.settings_key;

struct Tracked {
    status: DeviceStatus,
    seen: Instant,
}

#[derive(Default)]
struct App {
    main: HWND,
    strip: HWND,
    taskbar: HWND,
    devices: BTreeMap<String, Tracked>,
    order: Vec<String>,
    polling: bool,
    loaded: bool,
    /// A refresh started from the menu: show the ↻ indicator until it finishes.
    manual: bool,
    /// "Refresh devices": forget every known device (no grace period) on the next result.
    forget: bool,
    /// Whether the devices were enumerated at least once (the first read always enumerates).
    scanned: bool,
    /// The next poll was triggered by a device connecting / disconnecting: devices that are gone
    /// and have no sleep grace period disappear at once instead of after MIN_GRACE_SEC.
    devchange: bool,
    size: SIZE,
    last_place: (RECT, RECT, i32),
    /// Strip x ranges of the devices drawn in the last paint: (from, to, device index).
    hits: Vec<(i32, i32, usize)>,
    /// Hover tooltip (tracking tooltip owned by the main window) and what it shows right now.
    tip: HWND,
    tip_for: Option<usize>,
    tracking: bool,
    taskbar_created: u32,
}

thread_local! {
    static APP: RefCell<App> = RefCell::new(App::default());
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().map(|mut a| f(&mut a)))
}

// ---- drawing -----------------------------------------------------------------------------------

/// Premultiplied BGRA canvas.
struct Canvas {
    w: i32,
    h: i32,
    px: Vec<u32>,
}

impl Canvas {
    fn blend(&mut self, x: i32, y: i32, c: COLORREF, a: u32) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h || a == 0 {
            return;
        }
        let (r, g, b) = split(c);
        let d = &mut self.px[(y * self.w + x) as usize];
        let inv = 255 - a;
        let ch = |src: u8, shift: u32| ((src as u32 * a + ((*d >> shift) & 0xFF) * inv) / 255) & 0xFF;
        let na = (a + ((*d >> 24) & 0xFF) * inv / 255).min(255);
        *d = na << 24 | ch(r, 16) << 16 | ch(g, 8) << 8 | ch(b, 0);
    }
}

/// Ink coverage of `text`, cropped to the drawn pixels. Same technique as the original widget:
/// ClearType text, white on black (black on white for a light taskbar), coverage = RGB average.
struct Mask {
    w: i32,
    h: i32,
    a: Vec<u8>,
}

fn mask(text: &str, font: HFONT, dark_on_light: bool) -> Mask {
    unsafe {
        let dc = CreateCompatibleDC(None);
        let old_f = SelectObject(dc, HGDIOBJ(font.0));
        let wtext: Vec<u16> = text.encode_utf16().collect();
        let mut sz = SIZE::default();
        let _ = GetTextExtentPoint32W(dc, &wtext, &mut sz);
        let (w, h) = (sz.cx + 8, sz.cy + 8);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let mut out = Mask { w: 0, h: 0, a: Vec::new() };
        if let Ok(bmp) = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            let old_b = SelectObject(dc, HGDIOBJ(bmp.0));
            let px = std::slice::from_raw_parts_mut(bits as *mut u32, (w * h) as usize);
            px.fill(if dark_on_light { 0xFFFF_FFFF } else { 0 });
            SetBkMode(dc, TRANSPARENT);
            SetTextColor(dc, if dark_on_light { rgb(0, 0, 0) } else { rgb(255, 255, 255) });
            let _ = TextOutW(dc, 4, 4, &wtext);
            let _ = GdiFlush();
            let cover = |v: u32| -> u8 {
                let c = ((v & 0xFF) + ((v >> 8) & 0xFF) + ((v >> 16) & 0xFF)) / 3;
                (if dark_on_light { 255 - c } else { c }) as u8
            };
            let (mut x0, mut y0, mut x1, mut y1) = (w, h, -1, -1);
            for y in 0..h {
                for x in 0..w {
                    if cover(px[(y * w + x) as usize]) != 0 {
                        x0 = x0.min(x);
                        x1 = x1.max(x);
                        y0 = y0.min(y);
                        y1 = y1.max(y);
                    }
                }
            }
            if x1 >= 0 {
                out.w = x1 - x0 + 1;
                out.h = y1 - y0 + 1;
                out.a = Vec::with_capacity((out.w * out.h) as usize);
                for y in 0..out.h {
                    for x in 0..out.w {
                        out.a.push(cover(px[((y0 + y) * w + x0 + x) as usize]));
                    }
                }
            }
            SelectObject(dc, old_b);
            let _ = DeleteObject(HGDIOBJ(bmp.0));
        }
        SelectObject(dc, old_f);
        let _ = DeleteDC(dc);
        out
    }
}

/// Creates a ClearType font, falling back when the preferred face is not installed (GDI would
/// otherwise silently substitute another face).
fn make_font(px: i32, weight: i32, face: &str, fallback: &str, fallback_px: i32) -> HFONT {
    for (f, size) in [(face, px), (fallback, if fallback_px > 0 { fallback_px } else { px })] {
        unsafe {
            let h = CreateFontW(
                -size, 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY, 0, &windows::core::HSTRING::from(f),
            );
            if f == fallback {
                return h;
            }
            let dc = CreateCompatibleDC(None);
            let o = SelectObject(dc, HGDIOBJ(h.0));
            let mut got = [0u16; 64];
            GetTextFaceW(dc, Some(&mut got));
            SelectObject(dc, o);
            let _ = DeleteDC(dc);
            if batteryhub::util::from_wide(&got).eq_ignore_ascii_case(f) {
                return h;
            }
            let _ = DeleteObject(HGDIOBJ(h.0));
        }
    }
    unreachable!()
}

struct Piece {
    /// Index of the device this piece belongs to (for hover hit-testing), None for others.
    dev: Option<usize>,
    m: Mask,
    color: COLORREF,
    alpha: u32,
    gap_after: i32,
}

impl App {
    fn scale(&self) -> f32 {
        let dpi = unsafe { GetDpiForWindow(self.taskbar) };
        if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 }
    }

    fn visible(&self) -> Vec<DeviceStatus> {
        self.order
            .iter()
            .filter_map(|k| self.devices.get(k))
            .map(|t| t.status.clone())
            .filter(|s| s.connected)
            .collect()
    }

    /// Renders the strip and hands it to the layered window.
    fn paint(&mut self) {
        if self.strip.0.is_null() {
            return;
        }
        let s = self.scale();
        let px = |v: f32| (v * s).round() as i32;
        let light = ui::taskbar_is_light();
        // Same colours, fonts and spacing as the original C++ widget.
        let fg = if light { rgb(0, 0, 0) } else { rgb(255, 255, 255) };
        let red = ui::low_color(light);
        let icons = make_font(px(16.0), 400, "Segoe Fluent Icons", "Segoe MDL2 Assets", 0);
        let bolt = make_font(px(11.0), 400, "Segoe Fluent Icons", "Segoe MDL2 Assets", 0);
        let text = make_font(px(13.0), 400, "Segoe UI Variable Text", "Segoe UI", px(12.0));

        let mut pieces: Vec<Piece> = Vec::new();
        let devices = self.visible();
        if devices.is_empty() {
            // Battery-unknown glyph, so there is always something to right-click.
            pieces.push(Piece { dev: None, m: mask("\u{E996}", icons, light), color: fg, alpha: 110, gap_after: 0 });
        }
        for (i, d) in devices.iter().enumerate() {
            let alpha = if d.asleep { 120 } else { 255 };
            // Red at or below this device's own warning level (right-click menu, default 15%).
            let low = settings::is_low(d, settings::threshold(SETTINGS_KEY, &d.key));
            // Lightning bolt on power: green while charging, blue once it reached 100%.
            let bolt_color = ui::bolt_color(d, light);
            let mark = bolt_color.is_some();
            pieces.push(Piece { dev: Some(i), m: mask(&d.kind.glyph().to_string(), icons, light), color: fg, alpha, gap_after: px(5.0) });
            let t = if !d.level_text.is_empty() && d.percent < 0 {
                d.level_text.clone()
            } else if d.percent >= 0 {
                format!("{}%", d.percent)
            } else {
                "?".into()
            };
            pieces.push(Piece { dev: Some(i), m: mask(&t, text, light), color: if low { red } else { fg }, alpha, gap_after: if mark { px(2.0) } else { 0 } });
            if let Some(c) = bolt_color {
                pieces.push(Piece { dev: Some(i), m: mask("\u{E945}", bolt, light), color: c, alpha, gap_after: 0 });
            }
            if i + 1 < devices.len() {
                pieces.last_mut().unwrap().gap_after = px(14.0);
            }
        }
        if self.manual && self.polling {
            // Refresh started from the menu: a small refresh glyph until it finishes.
            pieces.last_mut().unwrap().gap_after = px(10.0);
            pieces.push(Piece { dev: None, m: mask("\u{E72C}", bolt, light), color: fg, alpha: 170, gap_after: 0 });
        }
        for f in [icons, bolt, text] {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(f.0));
            }
        }

        let mut tb = RECT::default();
        unsafe {
            let _ = GetClientRect(self.taskbar, &mut tb);
        }
        let h = (tb.bottom - tb.top).clamp(px(24.0), px(64.0));
        let pad = px(8.0);
        let w = pad * 2 + pieces.iter().map(|p| p.m.w + p.gap_after).sum::<i32>();
        // Alpha 1 everywhere: fully transparent pixels of a layered window let clicks fall through.
        let mut cv = Canvas { w, h, px: vec![0x0100_0000; (w * h) as usize] };
        let mut x = pad;
        self.hits.clear();
        for p in &pieces {
            if let Some(dev) = p.dev {
                // Each device owns its pieces plus half of the gap around them.
                match self.hits.last_mut() {
                    Some(h) if h.2 == dev => h.1 = x + p.m.w + p.gap_after / 2,
                    _ => {
                        let from = self.hits.last().map(|h| h.1).unwrap_or(0);
                        self.hits.push((from, x + p.m.w + p.gap_after / 2, dev));
                    }
                }
            }
            let y0 = (h - p.m.h) / 2;
            for yy in 0..p.m.h {
                for xx in 0..p.m.w {
                    let a = p.m.a[(yy * p.m.w + xx) as usize] as u32 * p.alpha / 255;
                    cv.blend(x + xx, y0 + yy, p.color, a);
                }
            }
            x += p.m.w + p.gap_after;
        }
        if let Some(last) = self.hits.last_mut() {
            last.1 = w;
        }
        self.size = SIZE { cx: w, cy: h };
        unsafe {
            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut c_void = std::ptr::null_mut();
            if let Ok(bmp) = CreateDIBSection(Some(mem), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
                std::ptr::copy_nonoverlapping(cv.px.as_ptr(), bits as *mut u32, cv.px.len());
                let ob = SelectObject(mem, HGDIOBJ(bmp.0));
                let src = POINT::default();
                let bf = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
                let _ = UpdateLayeredWindow(self.strip, Some(screen), None, Some(&self.size), Some(mem), Some(&src), COLORREF(0), Some(&bf), ULW_ALPHA);
                SelectObject(mem, ob);
                let _ = DeleteObject(HGDIOBJ(bmp.0));
            }
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
        }
        self.last_place.2 = -1; // force a reposition with the new width
        self.place();
    }

    // ---- taskbar placement ---------------------------------------------------------------------

    /// Creates the strip as a child of the taskbar (again, after Explorer restarted).
    fn ensure_strip(&mut self) -> bool {
        unsafe {
            if !self.strip.0.is_null() && IsWindow(Some(self.strip)).as_bool() {
                return true;
            }
            self.strip = HWND::default();
            let Ok(taskbar) = FindWindowW(w!("Shell_TrayWnd"), None) else { dbg_log("no Shell_TrayWnd"); return false };
            if FindWindowExW(Some(taskbar), None, w!("TrayNotifyWnd"), None).is_err() {
                dbg_log("no TrayNotifyWnd");
                return false;
            }
            self.taskbar = taskbar;
            let hinst = GetModuleHandleW(None).unwrap_or_default();
            let Ok(strip) = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_NOACTIVATE,
                STRIP_CLASS,
                w!(""),
                WS_CHILD | WS_CLIPSIBLINGS,
                0,
                0,
                1,
                1,
                Some(taskbar),
                None,
                Some(hinst.into()),
                None,
            ) else {
                dbg_log(&format!("CreateWindowExW failed: {:?}", windows::Win32::Foundation::GetLastError()));
                return false;
            };
            dbg_log(&format!("strip created {:?} parent {:?}", strip, taskbar));
            self.strip = strip;
            self.last_place = Default::default();
        }
        self.paint();
        unsafe {
            let _ = ShowWindow(self.strip, SW_SHOWNOACTIVATE);
        }
        true
    }

    /// Keeps the strip just left of the notification area (checked every second, like the original).
    fn place(&mut self) {
        if self.strip.0.is_null() {
            return;
        }
        unsafe {
            let Ok(tray) = FindWindowExW(Some(self.taskbar), None, w!("TrayNotifyWnd"), None) else { return };
            let (mut tr, mut bar) = (RECT::default(), RECT::default());
            let _ = GetWindowRect(tray, &mut tr);
            let _ = GetWindowRect(self.taskbar, &mut bar);
            if bar.bottom - bar.top > bar.right - bar.left {
                let _ = ShowWindow(self.strip, SW_HIDE); // vertical taskbar: not supported
                return;
            }
            let mut p = POINT { x: tr.left, y: bar.top };
            let _ = ScreenToClient(self.taskbar, &mut p);
            let mut left = p.x - self.size.cx - (2.0 * self.scale()) as i32;
            // Optional extra gap (DPI-independent px), e.g. to sit next to another taskbar widget.
            if let Some(off) = batteryhub::util::reg_get_dword(SETTINGS_KEY,"TaskbarOffset") {
                left -= (off as f32 * self.scale()) as i32;
            }
            let mut client = RECT::default();
            let _ = GetClientRect(self.taskbar, &mut client);
            let top = ((client.bottom - self.size.cy) / 2).max(0);

            // Windows 10 / ExplorerPatcher taskbar: narrow the task buttons so they never slide under us.
            if let Ok(rebar) = FindWindowExW(Some(self.taskbar), None, w!("ReBarWindow32"), None) {
                let mut rr = RECT::default();
                let _ = GetWindowRect(rebar, &mut rr);
                let mut tl = POINT { x: rr.left, y: rr.top };
                let _ = ScreenToClient(self.taskbar, &mut tl);
                if tl.x + (rr.right - rr.left) > left && left > tl.x {
                    let _ = MoveWindow(rebar, tl.x, tl.y, left - tl.x, rr.bottom - rr.top, true);
                }
            }
            let covered = GetWindow(self.strip, GW_HWNDPREV).is_ok_and(|h| !h.0.is_null());
            let key = (tr, bar, left);
            if !covered && self.last_place == key {
                return;
            }
            self.last_place = key;
            let _ = SetWindowPos(self.strip, Some(HWND_TOP), left, top, self.size.cx, self.size.cy, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        }
    }

    // ---- hover tooltip -------------------------------------------------------------------------

    fn create_tooltip(&mut self) {
        unsafe {
            let Ok(tip) = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                TOOLTIPS_CLASSW,
                None,
                WS_POPUP | WINDOW_STYLE(TTS_ALWAYSTIP | TTS_NOPREFIX),
                0, 0, 0, 0,
                Some(self.main),
                None,
                GetModuleHandleW(None).ok().map(|m| m.into()),
                None,
            ) else {
                return;
            };
            let mut ti = tool_info(self.main, &mut [0u16]);
            SendMessageW(tip, TTM_ADDTOOLW, None, Some(LPARAM(&mut ti as *mut _ as isize)));
            SendMessageW(tip, TTM_SETMAXTIPWIDTH, None, Some(LPARAM(480)));
            self.tip = tip;
        }
    }

    /// Text shown when hovering a device: name, level and charge state, connection, warning level.
    fn tip_text(&self, i: usize) -> String {
        let devices = self.visible();
        let Some(d) = devices.get(i) else { return "BatteryHub: no devices found".into() };
        let th = settings::threshold(SETTINGS_KEY, &d.key);
        let mut t = format!("{}\n{}", d.name, d.charge_text());
        if settings::is_low(d, th) {
            t += "  -  LOW BATTERY";
        }
        if !d.via.is_empty() {
            t += &format!("\nConnected via {}", d.via);
        }
        if !d.detail.is_empty() {
            t += &format!("\n{}", d.detail);
        }
        t += &format!("\nRed warning at {th}% or below (right-click to change)");
        t
    }

    fn hover(&mut self, x: i32) {
        if self.tip.0.is_null() {
            return;
        }
        unsafe {
            if !self.tracking {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: self.strip,
                    dwHoverTime: 0,
                };
                self.tracking = TrackMouseEvent(&mut tme).is_ok();
            }
            let hit = self.hits.iter().find(|h| x >= h.0 && x < h.1).map(|h| (h.2, h.0, h.1));
            let key = hit.map(|h| h.0).unwrap_or(usize::MAX);
            if self.tip_for == Some(key) {
                return;
            }
            self.tip_for = Some(key);
            // Same colours as the taskbar (re-checked each time, the theme can change at any time).
            ui::set_tooltip_theme(self.tip, !ui::taskbar_is_light());
            let mut text: Vec<u16> = self.tip_text(key).encode_utf16().chain(std::iter::once(0)).collect();
            let mut ti = tool_info(self.main, &mut text);
            SendMessageW(self.tip, TTM_UPDATETIPTEXTW, None, Some(LPARAM(&mut ti as *mut _ as isize)));
            SendMessageW(self.tip, TTM_TRACKACTIVATE, Some(WPARAM(1)), Some(LPARAM(&mut ti as *mut _ as isize)));
            // Above the strip, centred on the hovered device.
            let size = SendMessageW(self.tip, TTM_GETBUBBLESIZE, None, Some(LPARAM(&mut ti as *mut _ as isize))).0;
            let (tw, th) = ((size & 0xFFFF) as i32, ((size >> 16) & 0xFFFF) as i32);
            let mut rc = RECT::default();
            let _ = GetWindowRect(self.strip, &mut rc);
            let (from, to) = hit.map(|h| (h.1, h.2)).unwrap_or((0, self.size.cx));
            let cx = rc.left + (from + to) / 2 - tw / 2;
            let cy = rc.top - th - 4;
            let pos = ((cy as u32 & 0xFFFF) << 16) | (cx as u32 & 0xFFFF);
            SendMessageW(self.tip, TTM_TRACKPOSITION, None, Some(LPARAM(pos as isize)));
        }
    }

    fn hide_tip(&mut self) {
        self.tracking = false;
        self.tip_for = None;
        if !self.tip.0.is_null() {
            unsafe {
                let mut ti = tool_info(self.main, &mut [0u16]);
                SendMessageW(self.tip, TTM_TRACKACTIVATE, Some(WPARAM(0)), Some(LPARAM(&mut ti as *mut _ as isize)));
            }
        }
    }

    // ---- polling -------------------------------------------------------------------------------

    fn start_poll(&mut self) {
        if !self.polling {
            self.polling = true;
            // Enumerate the devices again only when something may have changed; a periodic read
            // reuses the device list and just reads the battery levels.
            let rescan = !self.scanned || self.devchange || self.forget;
            self.scanned = true;
            dbg_log(&format!("poll started (device change: {}, rescan: {rescan}, interval: {} min)", self.devchange, refresh_minutes()));
            ui::poll_async(self.main, WM_POLL_DONE, rescan);
        }
    }

    /// Menu refresh: `devices` = full rescan that drops devices that are gone right away;
    /// otherwise only the battery levels are read again (devices keep their grace period).
    fn manual_refresh(&mut self, devices: bool) {
        self.forget |= devices;
        self.manual = true;
        self.start_poll();
        self.paint(); // shows the ↻ indicator
    }

    fn on_result(&mut self, fresh: Vec<DeviceStatus>) {
        self.polling = false;
        self.loaded = true;
        self.manual = false;
        let devchange = std::mem::take(&mut self.devchange);
        if std::mem::take(&mut self.forget) {
            self.devices.clear();
            self.order.clear();
        }
        let keys: HashSet<String> = fresh.iter().map(|s| s.key.clone()).collect();
        // A device that misses one answer keeps its last value for a while instead of blinking out.
        self.devices.retain(|k, t| {
            if keys.contains(k) {
                return true;
            }
            let age = t.seen.elapsed().as_secs();
            if devchange && t.status.grace_sec == 0 {
                return false;
            }
            if t.status.grace_sec > 0 && age < t.status.grace_sec as u64 {
                t.status.asleep = true;
                return true;
            }
            age < MIN_GRACE_SEC
        });
        let now = Instant::now();
        for s in fresh {
            if !self.order.contains(&s.key) {
                self.order.push(s.key.clone());
            }
            // A device that reports "off" (headset switched off) disappears right away.
            if !s.connected {
                self.devices.remove(&s.key);
                continue;
            }
            self.devices.insert(s.key.clone(), Tracked { status: s, seen: now });
        }
        self.order.retain(|k| self.devices.contains_key(k));
        self.paint();
    }
}

/// Battery refresh interval in minutes (menu choice, default 1 minute).
fn refresh_minutes() -> u32 {
    batteryhub::util::reg_get_dword(SETTINGS_KEY, "RefreshMinutes")
        .filter(|m| INTERVALS.iter().any(|(v, _)| v == m))
        .unwrap_or(1)
}

/// Refresh when a device connects or disconnects (default on).
fn refresh_on_change() -> bool {
    batteryhub::util::reg_get_dword(SETTINGS_KEY, "RefreshOnDeviceChange").unwrap_or(1) != 0
}

fn save_dword(name: &str, value: u32) {
    if let Some(k) = batteryhub::install::create(batteryhub::install::HKCU, SETTINGS_KEY) {
        k.set_dword(name, value);
    }
}

fn tool_info(owner: HWND, text: &mut [u16]) -> TTTOOLINFOW {
    TTTOOLINFOW {
        cbSize: size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_TRACK | TTF_ABSOLUTE,
        hwnd: owner,
        uId: 1,
        lpszText: windows::core::PWSTR(text.as_mut_ptr()),
        ..Default::default()
    }
}

fn dbg_log(msg: &str) {
    if std::env::var_os("BH_DEBUG").is_some() {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(std::env::temp_dir().join("bh2.log")) {
            let _ = writeln!(f, "{msg}");
        }
    }
}

// ---- window procedures -------------------------------------------------------------------------

extern "system" fn strip_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_MOUSEMOVE => {
                with_app(|a| a.hover((lp.0 & 0xFFFF) as i16 as i32));
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                with_app(|a| a.hide_tip());
                LRESULT(0)
            }
            WM_RBUTTONUP => {
                let (main, devices) = with_app(|a| {
                    a.hide_tip();
                    (a.main, a.visible())
                })
                .unwrap_or_default();
                if let Ok(menu) = CreatePopupMenu() {
                    // One submenu per device: its charge state and the red warning level (50 ... 5%).
                    settings::append_device_menus(menu, &devices, SETTINGS_KEY);
                    if !devices.is_empty() {
                        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, windows::core::PCWSTR::null());
                    }
                    let _ = AppendMenuW(menu, MF_STRING, ID_REFRESH_DEVICES, w!("Refresh devices"));
                    let _ = AppendMenuW(menu, MF_STRING, ID_REFRESH_LEVELS, w!("Refresh battery levels"));
                    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, windows::core::PCWSTR::null());
                    // How often the battery levels are read, and whether a device change triggers a read.
                    if let Ok(sub) = CreatePopupMenu() {
                        let current = refresh_minutes();
                        for (j, (m, label)) in INTERVALS.iter().enumerate() {
                            let check = if *m == current { MF_CHECKED } else { MF_UNCHECKED };
                            let _ = AppendMenuW(sub, MF_STRING | check, ID_INTERVAL_BASE + j, &windows::core::HSTRING::from(*label));
                        }
                        let _ = AppendMenuW(menu, MF_STRING | MF_POPUP, sub.0 as usize, w!("Refresh battery every"));
                    }
                    let on_change = if refresh_on_change() { MF_CHECKED } else { MF_UNCHECKED };
                    let _ = AppendMenuW(menu, MF_STRING | on_change, ID_ON_CHANGE, w!("Refresh when a device connects or disconnects"));
                    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, windows::core::PCWSTR::null());
                    let _ = AppendMenuW(menu, MF_STRING, ID_EXIT, w!("Exit"));
                    let mut pt = POINT::default();
                    let _ = GetCursorPos(&mut pt);
                    // Menu colours follow the taskbar: dark on a dark taskbar, light on a light one.
                    ui::set_menu_theme(main, !ui::taskbar_is_light());
                    let _ = SetForegroundWindow(main);
                    let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, None, main, None);
                    let _ = PostMessageW(Some(main), WM_NULL, WPARAM(0), LPARAM(0));
                    let _ = DestroyMenu(menu);
                    match cmd.0 as usize {
                        ID_EXIT => {
                            let _ = DestroyWindow(main);
                        }
                        ID_REFRESH_DEVICES => {
                            with_app(|a| a.manual_refresh(true));
                        }
                        ID_REFRESH_LEVELS => {
                            with_app(|a| a.manual_refresh(false));
                        }
                        ID_ON_CHANGE => {
                            save_dword("RefreshOnDeviceChange", if refresh_on_change() { 0 } else { 1 });
                        }
                        id if (ID_INTERVAL_BASE..ID_INTERVAL_BASE + INTERVALS.len()).contains(&id) => {
                            let minutes = INTERVALS[id - ID_INTERVAL_BASE].0;
                            save_dword("RefreshMinutes", minutes);
                            SetTimer(Some(main), TIMER_POLL, minutes * 60_000, None);
                        }
                        id => {
                            if let Some((dev, level)) = settings::decode_menu_id(id) {
                                if let Some(d) = devices.get(dev) {
                                    settings::set_threshold(SETTINGS_KEY, &d.key, level);
                                    with_app(|a| a.paint());
                                }
                            }
                        }
                    }
                }
                LRESULT(0)
            }
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

extern "system" fn main_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_POLL_DONE => {
                let devices = ui::take_result(lp);
                with_app(|a| a.on_result(devices));
                // Hand pages used only during the read (enumeration, Bluetooth, fonts) back to
                // Windows until the next read, instead of keeping them resident for a minute or more.
                let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
                LRESULT(0)
            }
            WM_TIMER => {
                with_app(|a| match wp.0 {
                    TIMER_POLL => a.start_poll(),
                    TIMER_DEVCHANGE => {
                        let _ = KillTimer(Some(hwnd), TIMER_DEVCHANGE);
                        a.devchange = true;
                        a.start_poll();
                    }
                    _ => {
                        if a.ensure_strip() {
                            a.place();
                        }
                    }
                });
                LRESULT(0)
            }
            // A device was plugged in / removed (USB receiver, cable, Bluetooth link) or the PC woke
            // up: read again once things settle, if the user left that option on.
            WM_DEVICECHANGE_MSG if wp.0 == DBT_DEVNODES_CHANGED => {
                if refresh_on_change() {
                    SetTimer(Some(hwnd), TIMER_DEVCHANGE, 2500, None);
                }
                LRESULT(1)
            }
            WM_POWERBROADCAST if wp.0 == PBT_APMRESUMEAUTOMATIC => {
                if refresh_on_change() {
                    SetTimer(Some(hwnd), TIMER_DEVCHANGE, 5000, None);
                }
                LRESULT(1)
            }
            WM_SETTINGCHANGE | WM_DISPLAYCHANGE => {
                with_app(|a| a.paint()); // light/dark taskbar or DPI changed
                LRESULT(0)
            }
            WM_DESTROY => {
                with_app(|a| {
                    if !a.strip.0.is_null() {
                        let _ = DestroyWindow(a.strip);
                    }
                });
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => {
                let tc = with_app(|a| a.taskbar_created).unwrap_or(0);
                if tc != 0 && msg == tc {
                    with_app(|a| {
                        a.strip = HWND::default();
                        a.ensure_strip();
                    });
                    return LRESULT(0);
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
        }
    }
}

fn main() {
    unsafe {
        let _mutex = CreateMutexW(None, true, w!("Local\\BatteryHub.Taskbar"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // No text input anywhere: skip loading the IME / text services framework.
        let _ = windows::Win32::UI::Input::Ime::ImmDisableIME(u32::MAX);

        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let arrow = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        for (class, proc_) in [(MAIN_CLASS, main_proc as unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT), (STRIP_CLASS, strip_proc)] {
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(proc_),
                hInstance: hinst.into(),
                hCursor: arrow,
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassExW(&wc);
        }
        let Ok(main) = CreateWindowExW(WS_EX_TOOLWINDOW, MAIN_CLASS, w!("BatteryHub Taskbar"), WS_POPUP, 0, 0, 0, 0, None, None, Some(hinst.into()), None) else {
            return;
        };
        let taskbar_created = RegisterWindowMessageW(w!("TaskbarCreated"));
        with_app(|a| {
            a.main = main;
            a.taskbar_created = taskbar_created;
            a.create_tooltip();
            a.ensure_strip();
            a.start_poll();
        });
        SetTimer(Some(main), TIMER_POLL, refresh_minutes() * 60_000, None);
        SetTimer(Some(main), TIMER_PLACE, 1000, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
