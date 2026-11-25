use crate::{
    embed_pill_into_taskbar, fetch_usage_internal, find_taskbar_hwnd, get_hwnd_scale,
    get_taskbar_rects, get_work_area_for_hwnd, get_work_area_for_point, is_autostart_enabled_sync,
    launch_antigravity_action, raise_hwnd_topmost, save_window_state_sync, set_autostart_sync,
    sync_pill_taskbar_zorder, to_wide, trim_process_working_set, AppState, DwmSetWindowAttribute,
    GetWindowLongW, GetWindowRect, SavedWindowState, SetLayeredWindowAttributes, SetWindowLongW,
    SetWindowPos, UsageSnapshot, WinPoint, WinRect, DWMWA_BORDER_COLOR, DWMWA_COLOR_DEFAULT,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, FULL_HEIGHT_LOGICAL, FULL_WIDTH_LOGICAL,
    GWL_EXSTYLE, LWA_ALPHA, PILL_HEIGHT_LOGICAL, PILL_WIDTH_LOGICAL, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, VK_LBUTTON, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_POPUP, WS_VISIBLE,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;

const WM_DESTROY: u32 = 0x0002;
const WM_PAINT: u32 = 0x000F;
const WM_CLOSE: u32 = 0x0010;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_SETCURSOR: u32 = 0x0020;
const WM_TIMER: u32 = 0x0113;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_MOUSEWHEEL: u32 = 0x020A;
const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_APP_SNAPSHOT_UPDATED: u32 = 0x8001;

const TIMER_HOVER_POLL: usize = 1;
const TIMER_PILL_FLIP: usize = 2;
const TIMER_MINUTE_TICK: usize = 3;
const TIMER_API_POLL: usize = 4;

const IDC_ARROW: *const u16 = 32512 as *const u16;
const IDC_HAND: *const u16 = 32649 as *const u16;

const BI_RGB: u32 = 0;
const DIB_RGB_COLORS: u32 = 0;
const SRCCOPY: u32 = 0x00CC0020;
const TRANSPARENT: i32 = 1;

const DT_LEFT: u32 = 0x00000000;
const DT_CENTER: u32 = 0x00000001;
const DT_RIGHT: u32 = 0x00000002;
const DT_VCENTER: u32 = 0x00000004;
const DT_WORDBREAK: u32 = 0x00000010;
const DT_SINGLELINE: u32 = 0x00000020;
const DT_END_ELLIPSIS: u32 = 0x00008000;
const DT_NOPREFIX: u32 = 0x00000800;

const FW_NORMAL: i32 = 400;
const FW_MEDIUM: i32 = 500;
const FW_SEMIBOLD: i32 = 600;
const FW_BOLD: i32 = 700;
const DEFAULT_CHARSET: u32 = 1;
const OUT_DEFAULT_PRECIS: u32 = 0;
const CLIP_DEFAULT_PRECIS: u32 = 0;
const CLEARTYPE_QUALITY: u32 = 5;
const DEFAULT_PITCH: u32 = 0;

const TPM_LEFTALIGN: u32 = 0x0000;
const TPM_TOPALIGN: u32 = 0x0000;
const TPM_RETURNCMD: u32 = 0x0100;
const TPM_NONOTIFY: u32 = 0x0080;
const MF_STRING: u32 = 0x0000;
const MF_CHECKED: u32 = 0x0008;

const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: isize = -4;

#[repr(C)]
struct WndClassExW {
    cb_size: u32,
    style: u32,
    lpfn_wnd_proc: unsafe extern "system" fn(isize, u32, usize, isize) -> isize,
    cb_cls_extra: i32,
    cb_wnd_extra: i32,
    h_instance: isize,
    h_icon: isize,
    h_cursor: isize,
    hbr_background: isize,
    lpsz_menu_name: *const u16,
    lpsz_class_name: *const u16,
    h_icon_sm: isize,
}

#[repr(C)]
#[derive(Default)]
struct PaintStruct {
    hdc: isize,
    f_erase: i32,
    rc_paint: WinRect,
    f_restore: i32,
    f_inc_update: i32,
    rgb_reserved: [u8; 32],
}

#[repr(C)]
#[derive(Default)]
struct BitmapInfoHeader {
    bi_size: u32,
    bi_width: i32,
    bi_height: i32,
    bi_planes: u16,
    bi_bit_count: u16,
    bi_compression: u32,
    bi_size_image: u32,
    bi_x_pels_per_meter: i32,
    bi_y_pels_per_meter: i32,
    bi_clr_used: u32,
    bi_clr_important: u32,
}

#[repr(C)]
#[derive(Default)]
struct BitmapInfo {
    bmi_header: BitmapInfoHeader,
    bmi_colors: [u32; 1],
}

#[repr(C)]
#[derive(Default)]
struct Msg {
    hwnd: isize,
    message: u32,
    w_param: usize,
    l_param: isize,
    time: u32,
    pt: WinPoint,
}

#[repr(C)]
struct TrackMouseEventStruct {
    cb_size: u32,
    dw_flags: u32,
    hwnd_track: isize,
    dw_hover_time: u32,
}

#[link(name = "user32")]
extern "system" {
    fn SetProcessDpiAwarenessContext(value: isize) -> i32;
    fn RegisterClassExW(lpwcx: *const WndClassExW) -> u16;
    fn CreateWindowExW(
        dw_ex_style: u32,
        lp_class_name: *const u16,
        lp_window_name: *const u16,
        dw_style: u32,
        x: i32,
        y: i32,
        n_width: i32,
        n_height: i32,
        h_wnd_parent: isize,
        h_menu: isize,
        h_instance: isize,
        lp_param: *const std::ffi::c_void,
    ) -> isize;
    fn DefWindowProcW(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> isize;
    fn GetMessageW(lp_msg: *mut Msg, hwnd: isize, msg_filter_min: u32, msg_filter_max: u32) -> i32;
    fn TranslateMessage(lp_msg: *const Msg) -> i32;
    fn DispatchMessageW(lp_msg: *const Msg) -> isize;
    fn PostQuitMessage(n_exit_code: i32);
    fn PostMessageW(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> i32;
    fn BeginPaint(hwnd: isize, lp_paint: *mut PaintStruct) -> isize;
    fn EndPaint(hwnd: isize, lp_paint: *const PaintStruct) -> i32;
    fn InvalidateRect(hwnd: isize, lp_rect: *const WinRect, b_erase: i32) -> i32;
    fn GetClientRect(hwnd: isize, lp_rect: *mut WinRect) -> i32;
    fn ShowWindow(hwnd: isize, n_cmd_show: i32) -> i32;
    fn IsWindowVisible(hwnd: isize) -> i32;
    fn SetTimer(hwnd: isize, n_id_event: usize, u_elapse: u32, lp_timer_func: usize) -> usize;
    fn LoadCursorW(h_instance: isize, lp_cursor_name: *const u16) -> isize;
    fn SetCursor(h_cursor: isize) -> isize;
    fn DrawTextW(
        hdc: isize,
        lpch_text: *const u16,
        cch_text: i32,
        lprc: *mut WinRect,
        format: u32,
    ) -> i32;
    fn CreatePopupMenu() -> isize;
    fn AppendMenuW(h_menu: isize, u_flags: u32, u_id_new_item: usize, lp_new_item: *const u16)
        -> i32;
    fn TrackPopupMenu(
        h_menu: isize,
        u_flags: u32,
        x: i32,
        y: i32,
        n_reserved: i32,
        hwnd: isize,
        prc_rect: *const WinRect,
    ) -> i32;
    fn DestroyMenu(h_menu: isize) -> i32;
    fn SetForegroundWindow(hwnd: isize) -> i32;
    fn TrackMouseEvent(lp_event_track: *mut TrackMouseEventStruct) -> i32;
    fn GetCursorPos(lp_point: *mut WinPoint) -> i32;
    fn GetAsyncKeyState(vkey: i32) -> i16;
}

#[link(name = "gdi32")]
extern "system" {
    fn CreateCompatibleDC(hdc: isize) -> isize;
    fn DeleteDC(hdc: isize) -> i32;
    fn CreateDIBSection(
        hdc: isize,
        pbmi: *const BitmapInfo,
        usage: u32,
        ppv_bits: *mut *mut u32,
        h_section: isize,
        offset: u32,
    ) -> isize;
    fn SelectObject(hdc: isize, h: isize) -> isize;
    fn DeleteObject(ho: isize) -> i32;
    fn BitBlt(
        hdc: isize,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        hdc_src: isize,
        x1: i32,
        y1: i32,
        rop: u32,
    ) -> i32;
    fn CreateFontW(
        c_height: i32,
        c_width: i32,
        c_escapement: i32,
        c_orientation: i32,
        c_weight: i32,
        b_italic: u32,
        b_underline: u32,
        b_strike_out: u32,
        i_char_set: u32,
        i_out_precision: u32,
        i_clip_precision: u32,
        i_quality: u32,
        i_pitch_and_family: u32,
        psz_face_name: *const u16,
    ) -> isize;
    fn SetBkMode(hdc: isize, mode: i32) -> i32;
    fn SetTextColor(hdc: isize, color: u32) -> u32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(lp_module_name: *const u16) -> isize;
}

const LOCALE_LIST: &[(&str, &str, &str)] = &[
    ("et", "Eesti (ET)", include_str!("../locales/et.json")),
    (
        "en-US",
        "English (US)",
        include_str!("../locales/en-US.json"),
    ),
    (
        "en-GB",
        "English (UK)",
        include_str!("../locales/en-GB.json"),
    ),
    ("zh-CN", "简体中文", include_str!("../locales/zh-CN.json")),
    ("lv", "Latviešu (LV)", include_str!("../locales/lv.json")),
    ("lt", "Lietuvių (LT)", include_str!("../locales/lt.json")),
    ("fi", "Suomi (FI)", include_str!("../locales/fi.json")),
    ("de", "Deutsch (DE)", include_str!("../locales/de.json")),
    ("sv", "Svenska (SV)", include_str!("../locales/sv.json")),
    ("pl", "Polski (PL)", include_str!("../locales/pl.json")),
    ("fr", "Français (FR)", include_str!("../locales/fr.json")),
    ("es", "Español (ES)", include_str!("../locales/es.json")),
    (
        "uk",
        "Українська (UK)",
        include_str!("../locales/uk.json"),
    ),
];

fn load_locales_map() -> HashMap<String, Value> {
    let mut map = HashMap::new();
    for (code, _, raw) in LOCALE_LIST {
        if let Ok(val) = serde_json::from_str::<Value>(raw) {
            map.insert((*code).to_string(), val);
        }
    }
    map
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HitZone {
    None,
    ToggleMode,
    LangSelect,
    Autostart,
    InfoToggle,
    Refresh,
    Quit,
    SwitchAccount,
    LoginAction,
    OpacitySlider,
}

#[derive(Clone, Copy)]
struct HitBox {
    zone: HitZone,
    rect: WinRect,
}

struct UiShared {
    app_state: Arc<AppState>,
    rt_handle: tokio::runtime::Handle,
    locales: HashMap<String, Value>,
    current_locale: String,
    anchor_x: i32,
    is_dragging: bool,
    taskbar_color: Option<String>,
    window_opacity: u8,
    show_remaining: bool,
    last_snapshot: Option<UsageSnapshot>,
    pill_hwnd: isize,
    main_hwnd: isize,
    pill_mode_weekly: bool,
    show_info: bool,
    autostart_enabled: bool,
    is_loading: bool,
    error_code: Option<String>,
    hover_zone: HitZone,
    dragging_opacity: bool,
    scroll_offset_y: i32,
    hit_boxes: Vec<HitBox>,
    slider_rect: WinRect,
    enter_ticks: u32,
    leave_ticks: u32,
    last_tray_hwnd: isize,
    was_taskbar_fg: bool,
}

static GLOBAL_UI: OnceLock<Arc<StdMutex<UiShared>>> = OnceLock::new();

fn get_ui() -> Option<Arc<StdMutex<UiShared>>> {
    GLOBAL_UI.get().cloned()
}

impl UiShared {
    fn persist_state(&self) {
        save_window_state_sync(&SavedWindowState {
            anchor_x: self.anchor_x,
            opacity: self.window_opacity,
            locale: Some(self.current_locale.clone()),
            show_remaining: Some(self.show_remaining),
        });
    }

    fn t(&self, path: &str) -> String {
        let pointer = format!("/{}", path.replace('.', "/"));
        if let Some(loc) = self.locales.get(&self.current_locale) {
            if let Some(s) = loc.pointer(&pointer).and_then(|v| v.as_str()) {
                return s.to_string();
            }
        }
        if let Some(en) = self.locales.get("en-US") {
            if let Some(s) = en.pointer(&pointer).and_then(|v| v.as_str()) {
                return s.to_string();
            }
        }
        path.to_string()
    }

    fn format_seconds(&self, seconds: Option<i64>) -> String {
        let Some(sec) = seconds else {
            return self.t("time.none");
        };
        if sec <= 0 {
            return self.t("time.now");
        }
        let d = sec / 86400;
        let h = (sec % 86400) / 3600;
        let m = (sec % 3600) / 60;
        let s = sec % 60;
        if d > 0 {
            self.t("time.daysHours")
                .replace("{d}", &d.to_string())
                .replace("{h}", &h.to_string())
        } else if h > 0 {
            self.t("time.hoursMins")
                .replace("{h}", &h.to_string())
                .replace("{m}", &m.to_string())
        } else if m > 0 {
            self.t("time.mins").replace("{m}", &m.to_string())
        } else {
            self.t("time.secs").replace("{s}", &s.to_string())
        }
    }
}

// ============================================================================
// Anti-Aliased 32-Bit DIBSection Software + ClearType GDI Canvas
// ============================================================================

struct Canvas<'a> {
    hdc: isize,
    pixels: &'a mut [u32],
    w: i32,
    h: i32,
}

#[inline]
fn blend_pixel(dst: u32, r: u32, g: u32, b: u32, alpha: u32) -> u32 {
    if alpha == 0 {
        return dst;
    }
    if alpha >= 255 {
        return 0xFF000000 | (r << 16) | (g << 8) | b;
    }
    let inv = 255 - alpha;
    let dr = (dst >> 16) & 0xFF;
    let dg = (dst >> 8) & 0xFF;
    let db = dst & 0xFF;
    let nr = (r * alpha + dr * inv) / 255;
    let ng = (g * alpha + dg * inv) / 255;
    let nb = (b * alpha + db * inv) / 255;
    0xFF000000 | (nr << 16) | (ng << 8) | nb
}

#[inline]
fn sdf_round_rect(px: f32, py: f32, cx: f32, cy: f32, half_w: f32, half_h: f32, r: f32) -> f32 {
    let qx = (px - cx).abs() - (half_w - r);
    let qy = (py - cy).abs() - (half_h - r);
    let out_d = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let in_d = qx.max(qy).min(0.0);
    out_d + in_d - r
}

impl<'a> Canvas<'a> {
    fn clear(&mut self, rgb: u32) {
        let val = 0xFF000000 | (rgb & 0xFFFFFF);
        self.pixels.fill(val);
    }

    fn fill_round_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        rgb: u32,
        alpha: u8,
    ) {
        if w <= 0.0 || h <= 0.0 || alpha == 0 {
            return;
        }
        let r_col = (rgb >> 16) & 0xFF;
        let g_col = (rgb >> 8) & 0xFF;
        let b_col = rgb & 0xFF;
        let rad = radius.min(w * 0.5).min(h * 0.5).max(0.0);
        let cx = x + w * 0.5;
        let cy = y + h * 0.5;
        let hw = w * 0.5;
        let hh = h * 0.5;

        let min_x = (x.floor() as i32 - 1).max(0);
        let max_x = ((x + w).ceil() as i32 + 1).min(self.w - 1);
        let min_y = (y.floor() as i32 - 1).max(0);
        let max_y = ((y + h).ceil() as i32 + 1).min(self.h - 1);

        for py in min_y..=max_y {
            let row_off = (py * self.w) as usize;
            let fy = py as f32 + 0.5;
            for px in min_x..=max_x {
                let fx = px as f32 + 0.5;
                let d = sdf_round_rect(fx, fy, cx, cy, hw, hh, rad);
                let cov = (0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    let a = (alpha as f32 * cov).round() as u32;
                    let idx = row_off + px as usize;
                    self.pixels[idx] = blend_pixel(self.pixels[idx], r_col, g_col, b_col, a);
                }
            }
        }
    }

    fn fill_round_rect_gradient(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        rgb_start: u32,
        rgb_end: u32,
        alpha: u8,
    ) {
        if w <= 0.0 || h <= 0.0 || alpha == 0 {
            return;
        }
        let r0 = ((rgb_start >> 16) & 0xFF) as f32;
        let g0 = ((rgb_start >> 8) & 0xFF) as f32;
        let b0 = (rgb_start & 0xFF) as f32;
        let r1 = ((rgb_end >> 16) & 0xFF) as f32;
        let g1 = ((rgb_end >> 8) & 0xFF) as f32;
        let b1 = (rgb_end & 0xFF) as f32;

        let rad = radius.min(w * 0.5).min(h * 0.5).max(0.0);
        let cx = x + w * 0.5;
        let cy = y + h * 0.5;
        let hw = w * 0.5;
        let hh = h * 0.5;

        let min_x = (x.floor() as i32 - 1).max(0);
        let max_x = ((x + w).ceil() as i32 + 1).min(self.w - 1);
        let min_y = (y.floor() as i32 - 1).max(0);
        let max_y = ((y + h).ceil() as i32 + 1).min(self.h - 1);

        for py in min_y..=max_y {
            let row_off = (py * self.w) as usize;
            let fy = py as f32 + 0.5;
            for px in min_x..=max_x {
                let fx = px as f32 + 0.5;
                let d = sdf_round_rect(fx, fy, cx, cy, hw, hh, rad);
                let cov = (0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    let t = ((fx - x) / w.max(1.0)).clamp(0.0, 1.0);
                    let r_col = (r0 + (r1 - r0) * t).round() as u32;
                    let g_col = (g0 + (g1 - g0) * t).round() as u32;
                    let b_col = (b0 + (b1 - b0) * t).round() as u32;
                    let a = (alpha as f32 * cov).round() as u32;
                    let idx = row_off + px as usize;
                    self.pixels[idx] = blend_pixel(self.pixels[idx], r_col, g_col, b_col, a);
                }
            }
        }
    }

    fn stroke_round_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        thickness: f32,
        rgb: u32,
        alpha: u8,
    ) {
        if w <= 0.0 || h <= 0.0 || alpha == 0 {
            return;
        }
        let r_col = (rgb >> 16) & 0xFF;
        let g_col = (rgb >> 8) & 0xFF;
        let b_col = rgb & 0xFF;
        let rad = radius.min(w * 0.5).min(h * 0.5).max(0.0);
        let cx = x + w * 0.5;
        let cy = y + h * 0.5;
        let hw = w * 0.5;
        let hh = h * 0.5;
        let half_t = thickness * 0.5;

        let min_x = (x.floor() as i32 - 2).max(0);
        let max_x = ((x + w).ceil() as i32 + 2).min(self.w - 1);
        let min_y = (y.floor() as i32 - 2).max(0);
        let max_y = ((y + h).ceil() as i32 + 2).min(self.h - 1);

        for py in min_y..=max_y {
            let row_off = (py * self.w) as usize;
            let fy = py as f32 + 0.5;
            for px in min_x..=max_x {
                let fx = px as f32 + 0.5;
                let d = sdf_round_rect(fx, fy, cx, cy, hw, hh, rad);
                let ring_d = (d + half_t).abs() - half_t;
                let cov = (0.5 - ring_d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    let a = (alpha as f32 * cov).round() as u32;
                    let idx = row_off + px as usize;
                    self.pixels[idx] = blend_pixel(self.pixels[idx], r_col, g_col, b_col, a);
                }
            }
        }
    }

    fn fill_circle(&mut self, cx: f32, cy: f32, r: f32, rgb: u32, alpha: u8) {
        let r_col = (rgb >> 16) & 0xFF;
        let g_col = (rgb >> 8) & 0xFF;
        let b_col = rgb & 0xFF;
        let min_x = ((cx - r - 2.0).floor() as i32).max(0);
        let max_x = ((cx + r + 2.0).ceil() as i32).min(self.w - 1);
        let min_y = ((cy - r - 2.0).floor() as i32).max(0);
        let max_y = ((cy + r + 2.0).ceil() as i32).min(self.h - 1);

        for py in min_y..=max_y {
            let row_off = (py * self.w) as usize;
            let fy = py as f32 + 0.5;
            for px in min_x..=max_x {
                let fx = px as f32 + 0.5;
                let dist = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
                let cov = (r + 0.5 - dist).clamp(0.0, 1.0);
                if cov > 0.0 {
                    let a = (alpha as f32 * cov).round() as u32;
                    let idx = row_off + px as usize;
                    self.pixels[idx] = blend_pixel(self.pixels[idx], r_col, g_col, b_col, a);
                }
            }
        }
    }

    fn stroke_circle(&mut self, cx: f32, cy: f32, r: f32, thickness: f32, rgb: u32, alpha: u8) {
        let r_col = (rgb >> 16) & 0xFF;
        let g_col = (rgb >> 8) & 0xFF;
        let b_col = rgb & 0xFF;
        let half_t = thickness * 0.5;
        let min_x = ((cx - r - thickness - 2.0).floor() as i32).max(0);
        let max_x = ((cx + r + thickness + 2.0).ceil() as i32).min(self.w - 1);
        let min_y = ((cy - r - thickness - 2.0).floor() as i32).max(0);
        let max_y = ((cy + r + thickness + 2.0).ceil() as i32).min(self.h - 1);

        for py in min_y..=max_y {
            let row_off = (py * self.w) as usize;
            let fy = py as f32 + 0.5;
            for px in min_x..=max_x {
                let fx = px as f32 + 0.5;
                let dist = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
                let ring_d = (dist - r).abs() - half_t;
                let cov = (0.5 - ring_d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    let a = (alpha as f32 * cov).round() as u32;
                    let idx = row_off + px as usize;
                    self.pixels[idx] = blend_pixel(self.pixels[idx], r_col, g_col, b_col, a);
                }
            }
        }
    }

    fn draw_line_aa(
        &mut self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        thickness: f32,
        rgb: u32,
        alpha: u8,
    ) {
        let r_col = (rgb >> 16) & 0xFF;
        let g_col = (rgb >> 8) & 0xFF;
        let b_col = rgb & 0xFF;
        let half_t = thickness * 0.5;
        let min_x = ((x0.min(x1) - thickness - 2.0).floor() as i32).max(0);
        let max_x = ((x0.max(x1) + thickness + 2.0).ceil() as i32).min(self.w - 1);
        let min_y = ((y0.min(y1) - thickness - 2.0).floor() as i32).max(0);
        let max_y = ((y0.max(y1) + thickness + 2.0).ceil() as i32).min(self.h - 1);

        let dx = x1 - x0;
        let dy = y1 - y0;
        let len_sq = (dx * dx + dy * dy).max(0.0001);

        for py in min_y..=max_y {
            let row_off = (py * self.w) as usize;
            let fy = py as f32 + 0.5;
            for px in min_x..=max_x {
                let fx = px as f32 + 0.5;
                let t = (((fx - x0) * dx + (fy - y0) * dy) / len_sq).clamp(0.0, 1.0);
                let proj_x = x0 + t * dx;
                let proj_y = y0 + t * dy;
                let dist = ((fx - proj_x).powi(2) + (fy - proj_y).powi(2)).sqrt();
                let cov = (half_t + 0.5 - dist).clamp(0.0, 1.0);
                if cov > 0.0 {
                    let a = (alpha as f32 * cov).round() as u32;
                    let idx = row_off + px as usize;
                    self.pixels[idx] = blend_pixel(self.pixels[idx], r_col, g_col, b_col, a);
                }
            }
        }
    }

    fn draw_text(
        &mut self,
        text: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        font: isize,
        rgb: u32,
        flags: u32,
    ) {
        if text.is_empty() {
            return;
        }
        let r = (rgb >> 16) & 0xFF;
        let g = (rgb >> 8) & 0xFF;
        let b = rgb & 0xFF;
        let colorref = (b << 16) | (g << 8) | r;
        let wide: Vec<u16> = text.encode_utf16().collect();
        let mut rc = WinRect {
            left: x.round() as i32,
            top: y.round() as i32,
            right: (x + w).round() as i32,
            bottom: (y + h).round() as i32,
        };
        unsafe {
            let old_font = SelectObject(self.hdc, font);
            SetBkMode(self.hdc, TRANSPARENT);
            SetTextColor(self.hdc, colorref);
            let _ = DrawTextW(
                self.hdc,
                wide.as_ptr(),
                wide.len() as i32,
                &mut rc,
                flags | DT_NOPREFIX,
            );
            SelectObject(self.hdc, old_font);
        }
        // Restore alpha byte to 0xFF where GDI ClearType wrote 0x00 in top byte
        let min_x = rc.left.max(0);
        let max_x = (rc.right - 1).min(self.w - 1);
        let min_y = rc.top.max(0);
        let max_y = (rc.bottom - 1).min(self.h - 1);
        for py in min_y..=max_y {
            let row_off = (py * self.w) as usize;
            for px in min_x..=max_x {
                self.pixels[row_off + px as usize] |= 0xFF000000;
            }
        }
    }
}

struct FontSet {
    pill_label: isize,
    pill_pct: isize,
    brand: isize,
    badge: isize,
    section: isize,
    hero_title: isize,
    hero_pct: isize,
    card_title: isize,
    card_pct: isize,
    small: isize,
    body: isize,
}

impl FontSet {
    fn new(scale: f32, locale: &str) -> Self {
        let face_ui = to_wide(if locale == "zh-CN" {
            "Microsoft YaHei UI"
        } else {
            "Segoe UI"
        });
        let make = |px: f32, weight: i32| unsafe {
            CreateFontW(
                -(px * scale).round() as i32,
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                DEFAULT_PITCH,
                face_ui.as_ptr(),
            )
        };
        Self {
            pill_label: make(10.5, FW_SEMIBOLD),
            pill_pct: make(10.5, FW_BOLD),
            brand: make(13.0, FW_BOLD),
            badge: make(10.0, FW_SEMIBOLD),
            section: make(10.0, FW_BOLD),
            hero_title: make(11.5, FW_BOLD),
            hero_pct: make(16.0, FW_BOLD),
            card_title: make(10.5, FW_SEMIBOLD),
            card_pct: make(12.0, FW_BOLD),
            small: make(10.0, FW_MEDIUM),
            body: make(11.0, FW_NORMAL),
        }
    }
}

impl Drop for FontSet {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.pill_label);
            DeleteObject(self.pill_pct);
            DeleteObject(self.brand);
            DeleteObject(self.badge);
            DeleteObject(self.section);
            DeleteObject(self.hero_title);
            DeleteObject(self.hero_pct);
            DeleteObject(self.card_title);
            DeleteObject(self.card_pct);
            DeleteObject(self.small);
            DeleteObject(self.body);
        }
    }
}

fn parse_rgb_string(s: Option<&str>) -> u32 {
    let Some(raw) = s else {
        return 0x1C2028;
    };
    let inner = raw.trim().trim_start_matches("rgb(").trim_end_matches(')');
    let parts: Vec<u32> = inner
        .split(',')
        .filter_map(|p| p.trim().parse::<u32>().ok())
        .collect();
    if parts.len() == 3 {
        ((parts[0] & 0xFF) << 16) | ((parts[1] & 0xFF) << 8) | (parts[2] & 0xFF)
    } else {
        0x1C2028
    }
}

fn status_colors(rem_frac: f64) -> (u32, u32, u32) {
    // Returns (text_rgb, bar_start_rgb, bar_end_rgb)
    let rem_pct = rem_frac * 100.0;
    if rem_pct <= 10.0 {
        (0xF87171, 0xEF4444, 0xF87171)
    } else if rem_pct <= 30.0 {
        (0xFBBF24, 0xF59E0B, 0xFBBF24)
    } else {
        (0x34D399, 0x38BDF8, 0x34D399)
    }
}

// ============================================================================
// Taskbar Pill Renderer (`pill_hwnd`, 124 x 38 logical)
// ============================================================================

fn paint_pill_window(hwnd: isize, ui: &UiShared) {
    let mut ps = PaintStruct::default();
    let paint_dc = unsafe { BeginPaint(hwnd, &mut ps) };
    if paint_dc == 0 {
        return;
    }

    let mut rc = WinRect::default();
    unsafe {
        GetClientRect(hwnd, &mut rc);
    }
    let w = (rc.right - rc.left).max(1);
    let h = (rc.bottom - rc.top).max(1);
    let scale = get_hwnd_scale(hwnd) as f32;

    let mem_dc = unsafe { CreateCompatibleDC(paint_dc) };
    let bmi = BitmapInfo {
        bmi_header: BitmapInfoHeader {
            bi_size: std::mem::size_of::<BitmapInfoHeader>() as u32,
            bi_width: w,
            bi_height: -h, // top-down
            bi_planes: 1,
            bi_bit_count: 32,
            bi_compression: BI_RGB,
            ..Default::default()
        },
        bmi_colors: [0],
    };
    let mut bits_ptr: *mut u32 = std::ptr::null_mut();
    let hbm = unsafe {
        CreateDIBSection(
            mem_dc,
            &bmi,
            DIB_RGB_COLORS,
            &mut bits_ptr,
            0,
            0,
        )
    };
    if hbm != 0 && !bits_ptr.is_null() {
        let old_bm = unsafe { SelectObject(mem_dc, hbm) };
        let pixels = unsafe { std::slice::from_raw_parts_mut(bits_ptr, (w * h) as usize) };
        let mut canvas = Canvas {
            hdc: mem_dc,
            pixels,
            w,
            h,
        };

        let show_rem = ui.show_remaining;
        let tb_rgb = parse_rgb_string(ui.taskbar_color.as_deref());
        canvas.clear(tb_rgb);

        let flyout_open = unsafe { IsWindowVisible(ui.main_hwnd) != 0 };
        let pad = 1.5 * scale;
        let pw = w as f32 - pad * 2.0;
        let ph = h as f32 - pad * 2.0;
        canvas.fill_round_rect(pad, pad, pw, ph, 7.5 * scale, 0x0F141E, 195);
        if flyout_open {
            canvas.stroke_round_rect(pad, pad, pw, ph, 7.5 * scale, 1.0 * scale, 0x38BDF8, 165);
        } else {
            canvas.stroke_round_rect(pad, pad, pw, ph, 7.5 * scale, 1.0 * scale, 0x94A3B8, 60);
        }

        let fonts = FontSet::new(scale, &ui.current_locale);
        let win_key = if ui.pill_mode_weekly { "weekly" } else { "5h" };
        let win_tag = ui.t(if ui.pill_mode_weekly {
            "mini.weekTag"
        } else {
            "mini.fiveHourTag"
        });

        if let Some(ref snap) = ui.last_snapshot.as_ref().filter(|s| s.authenticated) {
            let mut min_rem = 1.0_f64;
            for b in &snap.buckets {
                if b.remaining_fraction < min_rem {
                    min_rem = b.remaining_fraction;
                }
            }
            let (dot_col, _, _) = status_colors(min_rem);
            let dot_x = 9.5 * scale;
            let dot_y = h as f32 * 0.5;
            canvas.fill_circle(dot_x, dot_y, 5.0 * scale, dot_col, 55);
            canvas.fill_circle(dot_x, dot_y, 3.0 * scale, dot_col, 255);

            let gem_b = snap
                .buckets
                .iter()
                .find(|b| b.group_key == "gemini" && b.window == win_key)
                .or_else(|| snap.buckets.iter().find(|b| b.group_key == "gemini"));

            let flash_m = snap
                .models
                .iter()
                .find(|m| m.family_key == "gemini_3_8_flash");
            let gem4_m = snap
                .models
                .iter()
                .filter(|m| m.family_key.starts_with("gemini_4"))
                .min_by(|a, b| {
                    a.remaining_fraction
                        .partial_cmp(&b.remaining_fraction)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });

            let r1_rem = if win_key == "5h" {
                flash_m
                    .map(|m| m.remaining_fraction)
                    .or_else(|| gem_b.map(|b| b.remaining_fraction))
                    .unwrap_or(1.0)
            } else {
                gem_b.map(|b| b.remaining_fraction).unwrap_or(1.0)
            };
            let r2_rem = if win_key == "5h" {
                gem4_m
                    .map(|m| m.remaining_fraction)
                    .or_else(|| gem_b.map(|b| b.remaining_fraction))
                    .unwrap_or(1.0)
            } else {
                gem_b.map(|b| b.remaining_fraction).unwrap_or(1.0)
            };

            let rows = [
                (format!("Flash {}", win_tag), r1_rem, 3.5 * scale),
                (format!("Gem 4 {}", win_tag), r2_rem, 19.5 * scale),
            ];

            let text_left = 17.5 * scale;
            let text_w = w as f32 - text_left - 6.5 * scale;

            for (label, rem_frac, ry) in rows {
                let val_pct = if show_rem {
                    (rem_frac * 1000.0).round() / 10.0
                } else {
                    ((1.0 - rem_frac) * 1000.0).round() / 10.0
                };
                let (txt_col, bar_s, bar_e) = status_colors(rem_frac);
                canvas.draw_text(
                    &label,
                    text_left,
                    ry,
                    text_w,
                    12.5 * scale,
                    fonts.pill_label,
                    0xE2E8F0,
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE,
                );
                let pct_str = format!("{:.1}%", val_pct);
                canvas.draw_text(
                    &pct_str,
                    text_left,
                    ry,
                    text_w,
                    12.5 * scale,
                    fonts.pill_pct,
                    txt_col,
                    DT_RIGHT | DT_VCENTER | DT_SINGLELINE,
                );

                let bar_y = ry + 12.2 * scale;
                let bar_h = 2.0 * scale;
                canvas.fill_round_rect(text_left, bar_y, text_w, bar_h, 1.0 * scale, 0x1E293B, 210);
                let fill_w = (text_w * (val_pct as f32 / 100.0)).clamp(0.0, text_w);
                if fill_w > 0.5 {
                    canvas.fill_round_rect_gradient(
                        text_left,
                        bar_y,
                        fill_w,
                        bar_h,
                        1.0 * scale,
                        bar_s,
                        bar_e,
                        255,
                    );
                }
            }
        } else {
            let dot_x = 9.5 * scale;
            let dot_y = h as f32 * 0.5;
            canvas.fill_circle(dot_x, dot_y, 3.0 * scale, 0xF87171, 255);
            let text_left = 18.0 * scale;
            let text_w = w as f32 - text_left - 6.0 * scale;
            canvas.draw_text(
                &ui.t("mini.quotaTitle"),
                text_left,
                5.0 * scale,
                text_w,
                14.0 * scale,
                fonts.pill_label,
                0xE2E8F0,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE,
            );
            canvas.draw_text(
                &ui.t("mini.hoverHint"),
                text_left,
                19.0 * scale,
                text_w,
                13.0 * scale,
                fonts.small,
                0x94A3B8,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE,
            );
        }

        unsafe {
            let _ = BitBlt(paint_dc, 0, 0, w, h, mem_dc, 0, 0, SRCCOPY);
            SelectObject(mem_dc, old_bm);
            DeleteObject(hbm);
        }
    }
    unsafe {
        DeleteDC(mem_dc);
        EndPaint(hwnd, &ps);
    }
}

// ============================================================================
// Vector Icons for Flyout Header & Cards
// ============================================================================

fn draw_bolt_icon(canvas: &mut Canvas, cx: f32, cy: f32, s: f32, rgb: u32) {
    let t = 1.45 * s;
    canvas.draw_line_aa(cx + 0.8 * s, cy - 5.2 * s, cx - 3.8 * s, cy + 0.6 * s, t, rgb, 255);
    canvas.draw_line_aa(cx - 3.8 * s, cy + 0.6 * s, cx + 0.2 * s, cy + 0.6 * s, t, rgb, 255);
    canvas.draw_line_aa(cx + 0.2 * s, cy + 0.6 * s, cx - 0.8 * s, cy + 5.2 * s, t, rgb, 255);
    canvas.draw_line_aa(cx - 0.8 * s, cy + 5.2 * s, cx + 3.8 * s, cy - 0.6 * s, t, rgb, 255);
    canvas.draw_line_aa(cx + 3.8 * s, cy - 0.6 * s, cx - 0.2 * s, cy - 0.6 * s, t, rgb, 255);
    canvas.draw_line_aa(cx - 0.2 * s, cy - 0.6 * s, cx + 0.8 * s, cy - 5.2 * s, t, rgb, 255);
}

fn draw_info_icon(canvas: &mut Canvas, cx: f32, cy: f32, s: f32, rgb: u32) {
    canvas.stroke_circle(cx, cy, 5.2 * s, 1.4 * s, rgb, 255);
    canvas.fill_circle(cx, cy - 2.3 * s, 0.9 * s, rgb, 255);
    canvas.draw_line_aa(cx, cy - 0.3 * s, cx, cy + 2.8 * s, 1.4 * s, rgb, 255);
}

fn draw_refresh_icon(canvas: &mut Canvas, cx: f32, cy: f32, s: f32, rgb: u32) {
    canvas.stroke_circle(cx, cy, 4.6 * s, 1.4 * s, rgb, 255);
    canvas.draw_line_aa(cx + 2.2 * s, cy - 4.6 * s, cx + 4.8 * s, cy - 4.2 * s, 1.4 * s, rgb, 255);
    canvas.draw_line_aa(cx + 4.8 * s, cy - 4.2 * s, cx + 4.2 * s, cy - 1.6 * s, 1.4 * s, rgb, 255);
}

fn draw_power_icon(canvas: &mut Canvas, cx: f32, cy: f32, s: f32, rgb: u32) {
    canvas.stroke_circle(cx, cy + 0.6 * s, 4.6 * s, 1.45 * s, rgb, 255);
    // Clear small gap at top of circle and draw vertical power bar
    canvas.fill_circle(cx, cy - 4.0 * s, 2.2 * s, 0x0F1624, 255);
    canvas.draw_line_aa(cx, cy - 5.4 * s, cx, cy - 0.6 * s, 1.55 * s, rgb, 255);
}

// ============================================================================
// Flyout Dashboard Renderer (`main_hwnd`, 368 x 440 logical)
// ============================================================================

fn paint_flyout_window(hwnd: isize, ui: &mut UiShared) {
    let mut ps = PaintStruct::default();
    let paint_dc = unsafe { BeginPaint(hwnd, &mut ps) };
    if paint_dc == 0 {
        return;
    }

    let mut rc = WinRect::default();
    unsafe {
        GetClientRect(hwnd, &mut rc);
    }
    let w = (rc.right - rc.left).max(1);
    let h = (rc.bottom - rc.top).max(1);
    let scale = get_hwnd_scale(hwnd) as f32;

    let mem_dc = unsafe { CreateCompatibleDC(paint_dc) };
    let bmi = BitmapInfo {
        bmi_header: BitmapInfoHeader {
            bi_size: std::mem::size_of::<BitmapInfoHeader>() as u32,
            bi_width: w,
            bi_height: -h,
            bi_planes: 1,
            bi_bit_count: 32,
            bi_compression: BI_RGB,
            ..Default::default()
        },
        bmi_colors: [0],
    };
    let mut bits_ptr: *mut u32 = std::ptr::null_mut();
    let hbm = unsafe {
        CreateDIBSection(
            mem_dc,
            &bmi,
            DIB_RGB_COLORS,
            &mut bits_ptr,
            0,
            0,
        )
    };
    if hbm != 0 && !bits_ptr.is_null() {
        let old_bm = unsafe { SelectObject(mem_dc, hbm) };
        let pixels = unsafe { std::slice::from_raw_parts_mut(bits_ptr, (w * h) as usize) };
        let mut canvas = Canvas {
            hdc: mem_dc,
            pixels,
            w,
            h,
        };

        ui.hit_boxes.clear();
        let fonts = FontSet::new(scale, &ui.current_locale);
        let show_rem = ui.show_remaining;
        let opacity = ui.window_opacity;
        let snap_opt = ui.last_snapshot.clone();

        // 1. Base Flyout Surface + Header + Footer
        canvas.clear(0x0B0F19);
        let header_h = 40.0 * scale;
        let footer_h = 32.0 * scale;
        let footer_y = h as f32 - footer_h;

        canvas.fill_round_rect(0.0, 0.0, w as f32, header_h, 0.0, 0x0F1624, 255);
        canvas.draw_line_aa(
            0.0,
            header_h,
            w as f32,
            header_h,
            1.0 * scale,
            0x94A3B8,
            36,
        );

        // Status dot + Brand Title + Tier Badge
        let authed = snap_opt.as_ref().map(|s| s.authenticated).unwrap_or(false);
        let dot_col = if ui.is_loading {
            0xFBBF24
        } else if authed {
            0x34D399
        } else {
            0xF87171
        };
        canvas.fill_circle(14.0 * scale, header_h * 0.5, 5.5 * scale, dot_col, 50);
        canvas.fill_circle(14.0 * scale, header_h * 0.5, 3.5 * scale, dot_col, 255);

        let brand_str = ui.t("brandTitle");
        canvas.draw_text(
            &brand_str,
            24.0 * scale,
            0.0,
            78.0 * scale,
            header_h,
            fonts.brand,
            0xE0F2FE,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );

        let raw_tier = if let Some(ref s) = snap_opt {
            if !s.installed {
                ui.t("status.missingInstall")
            } else if !s.authenticated {
                ui.t("status.notSignedIn")
            } else {
                s.paid_tier_name
                    .clone()
                    .or_else(|| s.tier_name.clone())
                    .unwrap_or_else(|| ui.t("status.freeTier"))
            }
        } else {
            "...".to_string()
        };
        let tier_str = raw_tier.replace("Google AI ", "AI ");
        let tier_x = 104.0 * scale;
        let tier_y = 11.0 * scale;
        let tier_w = 50.0 * scale;
        let tier_h = 18.0 * scale;
        canvas.fill_round_rect(tier_x, tier_y, tier_w, tier_h, 9.0 * scale, 0x312E81, 140);
        canvas.stroke_round_rect(
            tier_x,
            tier_y,
            tier_w,
            tier_h,
            9.0 * scale,
            1.0 * scale,
            0x818CF8,
            95,
        );
        canvas.draw_text(
            &tier_str,
            tier_x + 5.0 * scale,
            tier_y,
            tier_w - 10.0 * scale,
            tier_h,
            fonts.badge,
            0xE0E7FF,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );

        // Right-side Header Controls
        let mut cur_right = w as f32 - 8.0 * scale;
        let btn_size = 24.0 * scale;
        let btn_y = (header_h - btn_size) * 0.5;

        // (a) Quit button (`⏻`)
        cur_right -= btn_size;
        let quit_rc = WinRect {
            left: cur_right.round() as i32,
            top: btn_y.round() as i32,
            right: (cur_right + btn_size).round() as i32,
            bottom: (btn_y + btn_size).round() as i32,
        };
        if ui.hover_zone == HitZone::Quit {
            canvas.fill_round_rect(
                cur_right,
                btn_y,
                btn_size,
                btn_size,
                6.0 * scale,
                0xEF4444,
                55,
            );
        }
        draw_power_icon(
            &mut canvas,
            cur_right + btn_size * 0.5,
            btn_y + btn_size * 0.5,
            scale,
            if ui.hover_zone == HitZone::Quit {
                0xF87171
            } else {
                0x94A3B8
            },
        );
        ui.hit_boxes.push(HitBox {
            zone: HitZone::Quit,
            rect: quit_rc,
        });

        // (b) Refresh button (`↻`)
        cur_right -= btn_size + 2.0 * scale;
        let ref_rc = WinRect {
            left: cur_right.round() as i32,
            top: btn_y.round() as i32,
            right: (cur_right + btn_size).round() as i32,
            bottom: (btn_y + btn_size).round() as i32,
        };
        if ui.hover_zone == HitZone::Refresh {
            canvas.fill_round_rect(
                cur_right,
                btn_y,
                btn_size,
                btn_size,
                6.0 * scale,
                0x38BDF8,
                40,
            );
        }
        draw_refresh_icon(
            &mut canvas,
            cur_right + btn_size * 0.5,
            btn_y + btn_size * 0.5,
            scale,
            if ui.hover_zone == HitZone::Refresh {
                0xF1F5F9
            } else {
                0x94A3B8
            },
        );
        ui.hit_boxes.push(HitBox {
            zone: HitZone::Refresh,
            rect: ref_rc,
        });

        // (c) Info Toggle button (`ⓘ`)
        cur_right -= btn_size + 2.0 * scale;
        let info_rc = WinRect {
            left: cur_right.round() as i32,
            top: btn_y.round() as i32,
            right: (cur_right + btn_size).round() as i32,
            bottom: (btn_y + btn_size).round() as i32,
        };
        if ui.show_info || ui.hover_zone == HitZone::InfoToggle {
            canvas.fill_round_rect(
                cur_right,
                btn_y,
                btn_size,
                btn_size,
                6.0 * scale,
                0x38BDF8,
                if ui.show_info { 55 } else { 35 },
            );
        }
        draw_info_icon(
            &mut canvas,
            cur_right + btn_size * 0.5,
            btn_y + btn_size * 0.5,
            scale,
            if ui.show_info {
                0x38BDF8
            } else if ui.hover_zone == HitZone::InfoToggle {
                0xF1F5F9
            } else {
                0x94A3B8
            },
        );
        ui.hit_boxes.push(HitBox {
            zone: HitZone::InfoToggle,
            rect: info_rc,
        });

        // (d) Autostart Toggle button (`⚡`)
        cur_right -= btn_size + 2.0 * scale;
        let auto_rc = WinRect {
            left: cur_right.round() as i32,
            top: btn_y.round() as i32,
            right: (cur_right + btn_size).round() as i32,
            bottom: (btn_y + btn_size).round() as i32,
        };
        if ui.autostart_enabled || ui.hover_zone == HitZone::Autostart {
            canvas.fill_round_rect(
                cur_right,
                btn_y,
                btn_size,
                btn_size,
                6.0 * scale,
                0x38BDF8,
                if ui.autostart_enabled { 55 } else { 35 },
            );
        }
        draw_bolt_icon(
            &mut canvas,
            cur_right + btn_size * 0.5,
            btn_y + btn_size * 0.5,
            scale,
            if ui.autostart_enabled {
                0x38BDF8
            } else if ui.hover_zone == HitZone::Autostart {
                0xF1F5F9
            } else {
                0x94A3B8
            },
        );
        ui.hit_boxes.push(HitBox {
            zone: HitZone::Autostart,
            rect: auto_rc,
        });

        // (e) Language selector pill (`ET ▾`)
        let lang_w = 34.0 * scale;
        let lang_h = 20.0 * scale;
        let lang_y = (header_h - lang_h) * 0.5;
        cur_right -= lang_w + 4.0 * scale;
        let lang_rc = WinRect {
            left: cur_right.round() as i32,
            top: lang_y.round() as i32,
            right: (cur_right + lang_w).round() as i32,
            bottom: (lang_y + lang_h).round() as i32,
        };
        canvas.fill_round_rect(cur_right, lang_y, lang_w, lang_h, 5.0 * scale, 0x192030, 220);
        canvas.stroke_round_rect(
            cur_right,
            lang_y,
            lang_w,
            lang_h,
            5.0 * scale,
            1.0 * scale,
            if ui.hover_zone == HitZone::LangSelect {
                0x38BDF8
            } else {
                0x94A3B8
            },
            if ui.hover_zone == HitZone::LangSelect {
                130
            } else {
                60
            },
        );
        let short_lang = ui
            .current_locale
            .split('-')
            .next()
            .unwrap_or("EN")
            .to_uppercase();
        canvas.draw_text(
            &format!("{}▾", short_lang),
            cur_right,
            lang_y,
            lang_w,
            lang_h,
            fonts.badge,
            if ui.hover_zone == HitZone::LangSelect {
                0xF1F5F9
            } else {
                0x94A3B8
            },
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        ui.hit_boxes.push(HitBox {
            zone: HitZone::LangSelect,
            rect: lang_rc,
        });

        // (f) Mode toggle pill (`Alles %` / `Kasutatud %`)
        let mode_str = if show_rem {
            ui.t("mode.remaining")
        } else {
            ui.t("mode.used")
        };
        let mode_w = 58.0 * scale;
        let mode_h = 20.0 * scale;
        let mode_y = (header_h - mode_h) * 0.5;
        cur_right -= mode_w + 4.0 * scale;
        let mode_rc = WinRect {
            left: cur_right.round() as i32,
            top: mode_y.round() as i32,
            right: (cur_right + mode_w).round() as i32,
            bottom: (mode_y + mode_h).round() as i32,
        };
        canvas.fill_round_rect(cur_right, mode_y, mode_w, mode_h, 5.0 * scale, 0x192030, 220);
        canvas.stroke_round_rect(
            cur_right,
            mode_y,
            mode_w,
            mode_h,
            5.0 * scale,
            1.0 * scale,
            if ui.hover_zone == HitZone::ToggleMode {
                0x38BDF8
            } else {
                0x94A3B8
            },
            if ui.hover_zone == HitZone::ToggleMode {
                130
            } else {
                60
            },
        );
        canvas.draw_text(
            &mode_str,
            cur_right + 3.0 * scale,
            mode_y,
            mode_w - 6.0 * scale,
            mode_h,
            fonts.badge,
            0x38BDF8,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
        ui.hit_boxes.push(HitBox {
            zone: HitZone::ToggleMode,
            rect: mode_rc,
        });

        // 2. Main Content Body
        let body_top = header_h + 8.0 * scale;
        let body_pad_x = 10.0 * scale;
        let content_w = w as f32 - body_pad_x * 2.0;

        if ui.show_info {
            paint_info_view(&mut canvas, ui, &fonts, snap_opt.as_ref(), body_top, body_pad_x, content_w, scale);
        } else if let Some(ref snap) = snap_opt {
            if !snap.authenticated {
                paint_unauthed_view(&mut canvas, ui, &fonts, snap, body_top, body_pad_x, content_w, scale);
            } else {
                paint_dashboard_view(
                    &mut canvas,
                    ui,
                    &fonts,
                    snap,
                    show_rem,
                    body_top,
                    footer_y - 6.0 * scale,
                    body_pad_x,
                    content_w,
                    scale,
                );
            }
        } else {
            canvas.draw_text(
                &ui.t("status.loading"),
                body_pad_x,
                body_top + 40.0 * scale,
                content_w,
                30.0 * scale,
                fonts.body,
                0x94A3B8,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );
        }

        // 3. Footer Bar
        canvas.fill_round_rect(0.0, footer_y, w as f32, footer_h, 0.0, 0x090D15, 255);
        canvas.draw_line_aa(
            0.0,
            footer_y,
            w as f32,
            footer_y,
            1.0 * scale,
            0x94A3B8,
            36,
        );

        let email_str = snap_opt
            .as_ref()
            .filter(|s| s.authenticated && !s.email.is_empty())
            .map(|s| format!("{} ⇄", s.email))
            .unwrap_or_else(|| ui.t("status.notConnected"));

        let acct_x = 10.0 * scale;
        let acct_w = 148.0 * scale;
        let acct_rc = WinRect {
            left: acct_x.round() as i32,
            top: (footer_y + 4.0 * scale).round() as i32,
            right: (acct_x + acct_w).round() as i32,
            bottom: (footer_y + footer_h - 4.0 * scale).round() as i32,
        };
        canvas.draw_text(
            &email_str,
            acct_x,
            footer_y,
            acct_w,
            footer_h,
            fonts.small,
            if ui.hover_zone == HitZone::SwitchAccount {
                0x38BDF8
            } else {
                0x94A3B8
            },
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
        ui.hit_boxes.push(HitBox {
            zone: HitZone::SwitchAccount,
            rect: acct_rc,
        });

        let sess_count = snap_opt.as_ref().map(|s| s.active_sessions).unwrap_or(0);
        let sess_str = format!(
            "• {}",
            ui.t("status.activeSessions")
                .replace("{count}", &sess_count.to_string())
        );
        canvas.draw_text(
            &sess_str,
            162.0 * scale,
            footer_y,
            78.0 * scale,
            footer_h,
            fonts.small,
            if sess_count > 0 { 0x34D399 } else { 0x64748B },
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );

        // Opacity Slider + Updated Time on Footer Right
        let upd_str = snap_opt
            .as_ref()
            .map(|s| {
                if s.updated_at.len() >= 5 {
                    s.updated_at[..5].to_string()
                } else {
                    s.updated_at.clone()
                }
            })
            .unwrap_or_else(|| "--:--".to_string());
        let time_w = 34.0 * scale;
        let time_x = w as f32 - time_w - 10.0 * scale;
        canvas.draw_text(
            &upd_str,
            time_x,
            footer_y,
            time_w,
            footer_h,
            fonts.small,
            0x64748B,
            DT_RIGHT | DT_VCENTER | DT_SINGLELINE,
        );

        let sl_w = 54.0 * scale;
        let sl_x = time_x - sl_w - 8.0 * scale;
        let sl_y = footer_y + footer_h * 0.5;
        let sl_rc = WinRect {
            left: (sl_x - 4.0 * scale).round() as i32,
            top: (footer_y + 4.0 * scale).round() as i32,
            right: (sl_x + sl_w + 4.0 * scale).round() as i32,
            bottom: (footer_y + footer_h - 4.0 * scale).round() as i32,
        };
        ui.slider_rect = WinRect {
            left: sl_x.round() as i32,
            top: sl_rc.top,
            right: (sl_x + sl_w).round() as i32,
            bottom: sl_rc.bottom,
        };
        ui.hit_boxes.push(HitBox {
            zone: HitZone::OpacitySlider,
            rect: sl_rc,
        });

        canvas.fill_round_rect(
            sl_x,
            sl_y - 1.5 * scale,
            sl_w,
            3.0 * scale,
            1.5 * scale,
            0x1E293B,
            255,
        );
        let norm_op = ((opacity.clamp(45, 100) - 45) as f32) / 55.0;
        let knob_x = sl_x + norm_op * sl_w;
        canvas.fill_round_rect(
            sl_x,
            sl_y - 1.5 * scale,
            norm_op * sl_w,
            3.0 * scale,
            1.5 * scale,
            0x38BDF8,
            220,
        );
        canvas.fill_circle(knob_x, sl_y, 4.2 * scale, 0x38BDF8, 255);

        // Outer subtle frame
        canvas.stroke_round_rect(
            0.5 * scale,
            0.5 * scale,
            w as f32 - 1.0 * scale,
            h as f32 - 1.0 * scale,
            8.0 * scale,
            1.0 * scale,
            0x94A3B8,
            45,
        );

        unsafe {
            let _ = BitBlt(paint_dc, 0, 0, w, h, mem_dc, 0, 0, SRCCOPY);
            SelectObject(mem_dc, old_bm);
            DeleteObject(hbm);
        }
    }
    unsafe {
        DeleteDC(mem_dc);
        EndPaint(hwnd, &ps);
    }
}

fn paint_dashboard_view(
    canvas: &mut Canvas,
    ui: &UiShared,
    fonts: &FontSet,
    snap: &UsageSnapshot,
    show_rem: bool,
    top_y: f32,
    bottom_y: f32,
    pad_x: f32,
    content_w: f32,
    scale: f32,
) {
    // 1. Hero Section: 2 Side-by-Side Group Cards (`Gemini` and `Claude`)
    let gap = 7.0 * scale;
    let card_w = (content_w - gap) * 0.5;
    let hero_h = 88.0 * scale;

    let groups = [
        ("gemini", ui.t("groups.geminiShort"), pad_x),
        ("claude_gpt", ui.t("groups.claudeShort"), pad_x + card_w + gap),
    ];

    for (gkey, gtitle, gx) in groups {
        canvas.fill_round_rect(gx, top_y, card_w, hero_h, 8.0 * scale, 0x131B2E, 235);
        canvas.stroke_round_rect(
            gx,
            top_y,
            card_w,
            hero_h,
            8.0 * scale,
            1.0 * scale,
            0x94A3B8,
            42,
        );

        canvas.draw_text(
            &gtitle,
            gx + 8.0 * scale,
            top_y + 5.0 * scale,
            card_w - 16.0 * scale,
            15.0 * scale,
            fonts.hero_title,
            0xE2E8F0,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );

        let b_5h = snap
            .buckets
            .iter()
            .find(|b| b.group_key == gkey && b.window == "5h");
        let b_wk = snap
            .buckets
            .iter()
            .find(|b| b.group_key == gkey && b.window == "weekly");

        let sub_w = (card_w - 20.0 * scale) * 0.5;
        let sub_cols = [
            (ui.t("windowTags.5h"), b_5h, gx + 8.0 * scale),
            (ui.t("windowTags.weekly"), b_wk, gx + 12.0 * scale + sub_w),
        ];

        for (wlabel, b_opt, sx) in sub_cols {
            let sy = top_y + 24.0 * scale;
            let badge_w = 34.0 * scale;
            // Window pill badge
            canvas.fill_round_rect(sx, sy, badge_w, 14.0 * scale, 4.0 * scale, 0x1E293B, 255);
            canvas.draw_text(
                &wlabel,
                sx,
                sy,
                badge_w,
                14.0 * scale,
                fonts.badge,
                0x38BDF8,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );

            let rem_frac = b_opt.map(|b| b.remaining_fraction).unwrap_or(1.0);
            let pct = if show_rem {
                b_opt.map(|b| b.remaining_percent).unwrap_or(100.0)
            } else {
                b_opt.map(|b| b.used_percent).unwrap_or(0.0)
            };
            let (txt_col, bar_s, bar_e) = status_colors(rem_frac);

            canvas.draw_text(
                &format!("{:.1}%", pct),
                sx,
                sy + 16.0 * scale,
                sub_w,
                20.0 * scale,
                fonts.hero_pct,
                txt_col,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE,
            );

            let bar_y = sy + 39.0 * scale;
            let bar_h = 4.0 * scale;
            canvas.fill_round_rect(sx, bar_y, sub_w, bar_h, 2.0 * scale, 0x0B0F19, 255);
            let fill_w = (sub_w * (pct as f32 / 100.0)).clamp(0.0, sub_w);
            if fill_w > 0.5 {
                canvas.fill_round_rect_gradient(
                    sx,
                    bar_y,
                    fill_w,
                    bar_h,
                    2.0 * scale,
                    bar_s,
                    bar_e,
                    255,
                );
            }

            let timer_str = format!(
                "⏱ {}",
                ui.format_seconds(b_opt.and_then(|b| b.reset_seconds))
            );
            canvas.draw_text(
                &timer_str,
                sx,
                sy + 46.0 * scale,
                sub_w,
                13.0 * scale,
                fonts.small,
                0x94A3B8,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
            );
        }
    }

    // 2. Per-Model Quotas Grid
    let sec_y = top_y + hero_h + 10.0 * scale;
    let models_title = ui.t("tabs.models").to_uppercase();
    canvas.draw_text(
        &models_title,
        pad_x + 2.0 * scale,
        sec_y,
        120.0 * scale,
        16.0 * scale,
        fonts.section,
        0x94A3B8,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    let count_str = snap.models.len().to_string();
    let badge_x = pad_x + 62.0 * scale;
    canvas.fill_round_rect(
        badge_x,
        sec_y + 1.0 * scale,
        20.0 * scale,
        14.0 * scale,
        7.0 * scale,
        0x1E293B,
        230,
    );
    canvas.draw_text(
        &count_str,
        badge_x,
        sec_y + 1.0 * scale,
        20.0 * scale,
        14.0 * scale,
        fonts.badge,
        0x38BDF8,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );

    let grid_top = sec_y + 20.0 * scale;
    let m_card_h = 52.0 * scale;
    let m_gap_y = 6.0 * scale;

    for (idx, m) in snap.models.iter().enumerate() {
        let col = idx % 2;
        let row = idx / 2;
        let mx = pad_x + col as f32 * (card_w + gap);
        let my = grid_top + row as f32 * (m_card_h + m_gap_y) - (ui.scroll_offset_y as f32 * scale);

        if my < grid_top - 2.0 * scale || my + m_card_h > bottom_y + 2.0 * scale {
            continue;
        }

        canvas.fill_round_rect(mx, my, card_w, m_card_h, 7.0 * scale, 0x131B2E, 225);
        canvas.stroke_round_rect(
            mx,
            my,
            card_w,
            m_card_h,
            7.0 * scale,
            1.0 * scale,
            0x94A3B8,
            38,
        );

        let short_name = m
            .display_name
            .replace("Gemini ", "Gem ")
            .replace("Hydrogen ", "Hydro ")
            .replace("Claude Opus 5.5", "Claude Opus")
            .replace("Claude Sonnet 5.5", "Claude Sonnet")
            .replace(" (Medium)", " (Med)");
        canvas.draw_text(
            &short_name,
            mx + 8.0 * scale,
            my + 5.0 * scale,
            card_w - 56.0 * scale,
            15.0 * scale,
            fonts.card_title,
            0xF1F5F9,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );

        let pct = if show_rem {
            m.remaining_percent
        } else {
            m.used_percent
        };
        let (txt_col, bar_s, bar_e) = status_colors(m.remaining_fraction);

        canvas.draw_text(
            &format!("{:.1}%", pct),
            mx + card_w - 52.0 * scale,
            my + 5.0 * scale,
            44.0 * scale,
            15.0 * scale,
            fonts.card_pct,
            txt_col,
            DT_RIGHT | DT_VCENTER | DT_SINGLELINE,
        );

        let timer_str = format!("⏱ {}", ui.format_seconds(m.reset_seconds));
        canvas.draw_text(
            &timer_str,
            mx + 8.0 * scale,
            my + 21.0 * scale,
            card_w - 16.0 * scale,
            13.0 * scale,
            fonts.small,
            0x94A3B8,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );

        let bar_x = mx + 8.0 * scale;
        let bar_w = card_w - 16.0 * scale;
        let bar_y = my + 39.0 * scale;
        let bar_h = 4.0 * scale;
        canvas.fill_round_rect(bar_x, bar_y, bar_w, bar_h, 2.0 * scale, 0x0B0F19, 255);
        let fill_w = (bar_w * (pct as f32 / 100.0)).clamp(0.0, bar_w);
        if fill_w > 0.5 {
            canvas.fill_round_rect_gradient(
                bar_x,
                bar_y,
                fill_w,
                bar_h,
                2.0 * scale,
                bar_s,
                bar_e,
                255,
            );
        }
    }
}

fn paint_info_view(
    canvas: &mut Canvas,
    ui: &UiShared,
    fonts: &FontSet,
    snap_opt: Option<&UsageSnapshot>,
    top_y: f32,
    pad_x: f32,
    content_w: f32,
    scale: f32,
) {
    let mut cy = top_y;

    // Card 1: Connection Info
    let c1_h = 104.0 * scale;
    canvas.fill_round_rect(pad_x, cy, content_w, c1_h, 8.0 * scale, 0x131B2E, 235);
    canvas.stroke_round_rect(pad_x, cy, content_w, c1_h, 8.0 * scale, 1.0 * scale, 0x94A3B8, 42);
    canvas.draw_text(
        &ui.t("info.connTitle"),
        pad_x + 10.0 * scale,
        cy + 8.0 * scale,
        content_w - 20.0 * scale,
        16.0 * scale,
        fonts.hero_title,
        0x38BDF8,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    let (src, email, proj) = if let Some(s) = snap_opt {
        (
            if s.auth_source.is_empty() {
                "—"
            } else {
                &s.auth_source
            },
            if s.email.is_empty() { "—" } else { &s.email },
            if s.project_id.is_empty() {
                "—"
            } else {
                &s.project_id
            },
        )
    } else {
        ("—", "—", "—")
    };

    let info_lines = [
        format!("{} {}", ui.t("info.sourceLabel"), src),
        format!("{} {}", ui.t("info.accountLabel"), email),
        format!("{} {}", ui.t("info.projectLabel"), proj),
        ui.t(if ui.autostart_enabled {
            "info.autostartOn"
        } else {
            "info.autostartOff"
        }),
    ];
    for (i, line) in info_lines.iter().enumerate() {
        canvas.draw_text(
            line,
            pad_x + 10.0 * scale,
            cy + (28.0 + i as f32 * 17.0) * scale,
            content_w - 20.0 * scale,
            16.0 * scale,
            fonts.body,
            0xCBD5E1,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
    }

    cy += c1_h + 8.0 * scale;

    // Card 2: About
    let c2_h = 96.0 * scale;
    canvas.fill_round_rect(pad_x, cy, content_w, c2_h, 8.0 * scale, 0x131B2E, 235);
    canvas.stroke_round_rect(pad_x, cy, content_w, c2_h, 8.0 * scale, 1.0 * scale, 0x94A3B8, 42);
    canvas.draw_text(
        &format!("{} v{}", ui.t("appTitle"), env!("CARGO_PKG_VERSION")),
        pad_x + 10.0 * scale,
        cy + 8.0 * scale,
        content_w - 20.0 * scale,
        16.0 * scale,
        fonts.hero_title,
        0x38BDF8,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    canvas.draw_text(
        &ui.t("info.aboutDesc"),
        pad_x + 10.0 * scale,
        cy + 28.0 * scale,
        content_w - 20.0 * scale,
        c2_h - 34.0 * scale,
        fonts.body,
        0x94A3B8,
        DT_LEFT | DT_WORDBREAK,
    );

    cy += c2_h + 8.0 * scale;

    // Card 3: Author & Repo
    let c3_h = 84.0 * scale;
    canvas.fill_round_rect(pad_x, cy, content_w, c3_h, 8.0 * scale, 0x131B2E, 235);
    canvas.stroke_round_rect(pad_x, cy, content_w, c3_h, 8.0 * scale, 1.0 * scale, 0x94A3B8, 42);
    canvas.draw_text(
        &ui.t("info.repoTitle"),
        pad_x + 10.0 * scale,
        cy + 8.0 * scale,
        content_w - 20.0 * scale,
        16.0 * scale,
        fonts.hero_title,
        0x38BDF8,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    let repo_lines = [
        format!("{} sidequest-chain", ui.t("info.authorLabel")),
        format!(
            "{} sidequest-chain/antigravity-quota-monitor",
            ui.t("info.githubLabel")
        ),
        format!("{} {}", ui.t("info.licenseLabel"), ui.t("info.licenseValue")),
    ];
    for (i, line) in repo_lines.iter().enumerate() {
        canvas.draw_text(
            line,
            pad_x + 10.0 * scale,
            cy + (28.0 + i as f32 * 17.0) * scale,
            content_w - 20.0 * scale,
            16.0 * scale,
            fonts.body,
            0xCBD5E1,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
    }
}

fn paint_unauthed_view(
    canvas: &mut Canvas,
    ui: &mut UiShared,
    fonts: &FontSet,
    snap: &UsageSnapshot,
    top_y: f32,
    pad_x: f32,
    content_w: f32,
    scale: f32,
) {
    let card_y = top_y + 36.0 * scale;
    let card_h = 170.0 * scale;
    canvas.fill_round_rect(pad_x, card_y, content_w, card_h, 10.0 * scale, 0x131B2E, 240);
    canvas.stroke_round_rect(
        pad_x,
        card_y,
        content_w,
        card_h,
        10.0 * scale,
        1.0 * scale,
        0x38BDF8,
        75,
    );

    let title = if !snap.installed {
        ui.t("onboarding.missingTitle")
    } else {
        ui.t("onboarding.loginTitle")
    };
    let desc = if !snap.installed {
        ui.t("onboarding.missingDesc")
    } else {
        ui.t("onboarding.loginDesc")
    };
    let btn_txt = if !snap.installed {
        ui.t("onboarding.missingAction")
    } else {
        ui.t("onboarding.loginAction")
    };

    canvas.draw_text(
        &title,
        pad_x + 16.0 * scale,
        card_y + 18.0 * scale,
        content_w - 32.0 * scale,
        22.0 * scale,
        fonts.brand,
        0xF1F5F9,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );
    canvas.draw_text(
        &desc,
        pad_x + 18.0 * scale,
        card_y + 48.0 * scale,
        content_w - 36.0 * scale,
        54.0 * scale,
        fonts.body,
        0x94A3B8,
        DT_CENTER | DT_WORDBREAK,
    );

    let btn_w = content_w - 48.0 * scale;
    let btn_h = 34.0 * scale;
    let btn_x = pad_x + 24.0 * scale;
    let btn_y = card_y + card_h - 48.0 * scale;
    canvas.fill_round_rect_gradient(
        btn_x,
        btn_y,
        btn_w,
        btn_h,
        7.0 * scale,
        0x0284C7,
        0x2563EB,
        255,
    );
    canvas.draw_text(
        &btn_txt,
        btn_x,
        btn_y,
        btn_w,
        btn_h,
        fonts.card_title,
        0xFFFFFF,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );
    ui.hit_boxes.push(HitBox {
        zone: HitZone::LoginAction,
        rect: WinRect {
            left: btn_x.round() as i32,
            top: btn_y.round() as i32,
            right: (btn_x + btn_w).round() as i32,
            bottom: (btn_y + btn_h).round() as i32,
        },
    });
}

// ============================================================================
// Window Placement & Async Background Refresh Helpers
// ============================================================================

fn apply_flyout_style_and_opacity(hwnd: isize, opacity: u8) {
    unsafe {
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE);
        let base_ex = (ex | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) & !0x00040000;
        let corner_pref: u32 = DWMWCP_ROUND;
        let border_col: u32 = DWMWA_COLOR_DEFAULT;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner_pref,
            std::mem::size_of::<u32>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &border_col,
            std::mem::size_of::<u32>() as u32,
        );
        if opacity >= 100 {
            SetWindowLongW(hwnd, GWL_EXSTYLE, base_ex & !WS_EX_LAYERED);
        } else {
            SetWindowLongW(hwnd, GWL_EXSTYLE, base_ex | WS_EX_LAYERED);
            let alpha = ((opacity as u32 * 255) / 100).clamp(115, 255) as u8;
            let _ = SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA);
        }
        let _ = SetWindowPos(
            hwnd,
            0,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

fn show_flyout_above_pill(main_hwnd: isize, pill_hwnd: isize, opacity: u8) {
    let mut pill_rect = WinRect::default();
    unsafe {
        let _ = GetWindowRect(pill_hwnd, &mut pill_rect);
    }
    let scale = get_hwnd_scale(main_hwnd);
    let work = get_work_area_for_point(pill_rect.left, pill_rect.top);
    let (tb_rect, _) = get_taskbar_rects(scale, work);

    let full_w = (FULL_WIDTH_LOGICAL * scale).round() as i32;
    let full_h = (FULL_HEIGHT_LOGICAL * scale).round() as i32;
    let margin = (8.0 * scale).round() as i32;

    let min_x = work.left + margin;
    let max_x = (work.right - full_w - margin).max(min_x);
    let fx = pill_rect.left.clamp(min_x, max_x);
    let fy = (tb_rect.top - full_h - margin).max(work.top + margin);

    apply_flyout_style_and_opacity(main_hwnd, opacity);
    unsafe {
        let _ = SetWindowPos(main_hwnd, -1, fx, fy, full_w, full_h, SWP_NOACTIVATE);
        let _ = ShowWindow(main_hwnd, 5); // SW_SHOW
    }
    raise_hwnd_topmost(main_hwnd);
}

fn trigger_async_refresh(ui_arc: &Arc<StdMutex<UiShared>>, launch_ide: bool) {
    let (state, handle, pill_h, main_h) = {
        let Ok(mut ui) = ui_arc.lock() else { return };
        ui.is_loading = true;
        (
            Arc::clone(&ui.app_state),
            ui.rt_handle.clone(),
            ui.pill_hwnd,
            ui.main_hwnd,
        )
    };
    unsafe {
        InvalidateRect(pill_h, std::ptr::null(), 0);
        InvalidateRect(main_h, std::ptr::null(), 0);
    }
    let ui_clone = Arc::clone(ui_arc);
    handle.spawn(async move {
        if launch_ide {
            launch_antigravity_action(&state).await;
        }
        let res = fetch_usage_internal(&state).await;
        if let Ok(mut ui) = ui_clone.lock() {
            ui.is_loading = false;
            match res {
                Ok(snap) => {
                    ui.last_snapshot = Some(snap);
                    ui.error_code = None;
                }
                Err(e) => {
                    ui.error_code = Some(e);
                }
            }
        }
        unsafe {
            let _ = PostMessageW(pill_h, WM_APP_SNAPSHOT_UPDATED, 0, 0);
            let _ = PostMessageW(main_h, WM_APP_SNAPSHOT_UPDATED, 0, 0);
        }
        trim_process_working_set();
    });
}

// ============================================================================
// Win32 Window Procedures (`pill_wnd_proc` and `flyout_wnd_proc`)
// ============================================================================

unsafe extern "system" fn pill_wnd_proc(
    hwnd: isize,
    msg: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            if let Some(ui_arc) = get_ui() {
                if let Ok(ui) = ui_arc.lock() {
                    paint_pill_window(hwnd, &ui);
                    return 0;
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_SETCURSOR => {
            SetCursor(LoadCursorW(0, IDC_HAND));
            1
        }
        WM_LBUTTONDOWN => {
            if let Some(ui_arc) = get_ui() {
                let handle = {
                    let Ok(mut ui) = ui_arc.lock() else {
                        return 0;
                    };
                    ui.is_dragging = true;
                    ui.rt_handle.clone()
                };
                let mut start_cursor = WinPoint::default();
                let mut pill_rect = WinRect::default();
                GetCursorPos(&mut start_cursor);
                GetWindowRect(hwnd, &mut pill_rect);
                let grab_offset_x = (start_cursor.x - pill_rect.left).max(0);
                let ui_drag = Arc::clone(&ui_arc);

                handle.spawn(async move {
                    let scale = get_hwnd_scale(hwnd);
                    let work = get_work_area_for_hwnd(hwnd);
                    let (tb_rect, _) = get_taskbar_rects(scale, work);
                    let mut final_rel_x: Option<i32> = None;

                    loop {
                        tokio::time::sleep(Duration::from_millis(16)).await;
                        let down = (GetAsyncKeyState(VK_LBUTTON) as u16 & 0x8000) != 0;
                        if !down {
                            break;
                        }
                        let mut cur = WinPoint::default();
                        GetCursorPos(&mut cur);
                        if (cur.x - start_cursor.x).abs() >= 4 {
                            let desired_rel_x = (cur.x - grab_offset_x - tb_rect.left).max(4);
                            let (rx, _) = embed_pill_into_taskbar(hwnd, desired_rel_x, scale, work);
                            final_rel_x = Some(rx);
                        }
                    }

                    if let Some(rx) = final_rel_x {
                        let (_, sampled_col) = embed_pill_into_taskbar(hwnd, rx, scale, work);
                        if let Ok(mut ui) = ui_drag.lock() {
                            ui.anchor_x = rx;
                            ui.is_dragging = false;
                            if sampled_col.is_some() {
                                ui.taskbar_color = sampled_col;
                            }
                            ui.persist_state();
                        }
                        InvalidateRect(hwnd, std::ptr::null(), 0);
                    } else if let Ok(mut ui) = ui_drag.lock() {
                        ui.is_dragging = false;
                    }
                });
            }
            0
        }
        WM_TIMER => {
            if wparam == TIMER_HOVER_POLL {
                if let Some(ui_arc) = get_ui() {
                    let mut show_now: Option<(isize, isize, u8)> = None;
                    let mut hide_now: Option<(isize, isize)> = None;

                    if let Ok(mut ui) = ui_arc.lock() {
                        let lbutton_down = (GetAsyncKeyState(VK_LBUTTON) as u16 & 0x8000) != 0;
                        let mut cursor = WinPoint::default();
                        let mut pill_rect = WinRect::default();
                        let mut main_rect = WinRect::default();
                        GetCursorPos(&mut cursor);
                        GetWindowRect(ui.pill_hwnd, &mut pill_rect);
                        GetWindowRect(ui.main_hwnd, &mut main_rect);

                        let scale = get_hwnd_scale(ui.pill_hwnd);
                        let pad = (14.0 * scale).round() as i32;
                        let work = get_work_area_for_hwnd(ui.pill_hwnd);

                        if !ui.is_dragging {
                            let ui_ref: &mut UiShared = &mut *ui;
                            if let Some(col) = sync_pill_taskbar_zorder(
                                ui_ref.pill_hwnd,
                                &mut ui_ref.last_tray_hwnd,
                                &mut ui_ref.was_taskbar_fg,
                                ui_ref.anchor_x,
                            ) {
                                ui_ref.taskbar_color = Some(col);
                            }
                        }

                        let inside_pill = cursor.x >= pill_rect.left
                            && cursor.x <= pill_rect.right
                            && cursor.y >= pill_rect.top
                            && cursor.y <= pill_rect.bottom;

                        let flyout_vis = IsWindowVisible(ui.main_hwnd) != 0;

                        if !ui.is_dragging {
                            if !flyout_vis {
                                ui.leave_ticks = 0;
                                if inside_pill && !lbutton_down {
                                    ui.enter_ticks += 1;
                                    if ui.enter_ticks >= 3 {
                                        ui.enter_ticks = 0;
                                        show_now = Some((ui.main_hwnd, ui.pill_hwnd, ui.window_opacity));
                                    }
                                } else {
                                    ui.enter_ticks = 0;
                                }
                            } else {
                                ui.enter_ticks = 0;
                                let (tb_rect, _) = get_taskbar_rects(scale, work);
                                let inside_flyout_or_bridge = cursor.x >= main_rect.left - pad
                                    && cursor.x <= main_rect.right + pad
                                    && cursor.y >= main_rect.top - pad
                                    && cursor.y <= tb_rect.bottom + pad;

                                if inside_pill || inside_flyout_or_bridge || lbutton_down {
                                    ui.leave_ticks = 0;
                                } else {
                                    ui.leave_ticks += 1;
                                    if ui.leave_ticks >= 6 {
                                        ui.leave_ticks = 0;
                                        hide_now = Some((ui.main_hwnd, ui.pill_hwnd));
                                    }
                                }
                            }
                        }
                    }

                    if let Some((mh, ph, op)) = show_now {
                        show_flyout_above_pill(mh, ph, op);
                        InvalidateRect(ph, std::ptr::null(), 0);
                        InvalidateRect(mh, std::ptr::null(), 0);
                    } else if let Some((mh, ph)) = hide_now {
                        ShowWindow(mh, 0); // SW_HIDE
                        InvalidateRect(ph, std::ptr::null(), 0);
                    }
                }
            } else if wparam == TIMER_PILL_FLIP {
                if let Some(ui_arc) = get_ui() {
                    if let Ok(mut ui) = ui_arc.lock() {
                        ui.pill_mode_weekly = !ui.pill_mode_weekly;
                    }
                    InvalidateRect(hwnd, std::ptr::null(), 0);
                }
            } else if wparam == TIMER_MINUTE_TICK {
                if let Some(ui_arc) = get_ui() {
                    if let Ok(mut ui) = ui_arc.lock() {
                        if let Some(ref mut snap) = ui.last_snapshot {
                            for b in &mut snap.buckets {
                                if let Some(ref mut sec) = b.reset_seconds {
                                    *sec = (*sec - 60).max(0);
                                }
                            }
                            for m in &mut snap.models {
                                if let Some(ref mut sec) = m.reset_seconds {
                                    *sec = (*sec - 60).max(0);
                                }
                            }
                        }
                        if IsWindowVisible(ui.main_hwnd) != 0 {
                            InvalidateRect(ui.main_hwnd, std::ptr::null(), 0);
                        }
                    }
                }
            } else if wparam == TIMER_API_POLL {
                if let Some(ui_arc) = get_ui() {
                    trigger_async_refresh(&ui_arc, false);
                }
            }
            0
        }
        WM_APP_SNAPSHOT_UPDATED => {
            InvalidateRect(hwnd, std::ptr::null(), 0);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe extern "system" fn flyout_wnd_proc(
    hwnd: isize,
    msg: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            if let Some(ui_arc) = get_ui() {
                if let Ok(mut ui) = ui_arc.lock() {
                    paint_flyout_window(hwnd, &mut ui);
                    return 0;
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_SETCURSOR => {
            if let Some(ui_arc) = get_ui() {
                if let Ok(ui) = ui_arc.lock() {
                    if ui.hover_zone != HitZone::None {
                        SetCursor(LoadCursorW(0, IDC_HAND));
                        return 1;
                    }
                }
            }
            SetCursor(LoadCursorW(0, IDC_ARROW));
            1
        }
        WM_MOUSEMOVE => {
            let x = (lparam & 0xFFFF) as i16 as i32;
            let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
            let mut tme = TrackMouseEventStruct {
                cb_size: std::mem::size_of::<TrackMouseEventStruct>() as u32,
                dw_flags: 0x00000002, // TME_LEAVE
                hwnd_track: hwnd,
                dw_hover_time: 0,
            };
            let _ = TrackMouseEvent(&mut tme);

            if let Some(ui_arc) = get_ui() {
                let mut need_repaint = false;
                let mut new_opacity: Option<u8> = None;

                if let Ok(mut ui) = ui_arc.lock() {
                    if ui.dragging_opacity {
                        let sl = ui.slider_rect;
                        let w = (sl.right - sl.left).max(1) as f32;
                        let norm = ((x - sl.left) as f32 / w).clamp(0.0, 1.0);
                        let pct = (45.0 + norm * 55.0).round() as u8;
                        if ui.window_opacity != pct {
                            ui.window_opacity = pct;
                            new_opacity = Some(pct);
                            need_repaint = true;
                        }
                    } else {
                        let mut found = HitZone::None;
                        for hb in &ui.hit_boxes {
                            if x >= hb.rect.left
                                && x <= hb.rect.right
                                && y >= hb.rect.top
                                && y <= hb.rect.bottom
                            {
                                found = hb.zone;
                                break;
                            }
                        }
                        if found != ui.hover_zone {
                            ui.hover_zone = found;
                            need_repaint = true;
                        }
                    }
                }

                if let Some(pct) = new_opacity {
                    apply_flyout_style_and_opacity(hwnd, pct);
                }
                if need_repaint {
                    InvalidateRect(hwnd, std::ptr::null(), 0);
                }
            }
            0
        }
        WM_MOUSELEAVE => {
            if let Some(ui_arc) = get_ui() {
                if let Ok(mut ui) = ui_arc.lock() {
                    if ui.hover_zone != HitZone::None || ui.dragging_opacity {
                        ui.hover_zone = HitZone::None;
                        ui.dragging_opacity = false;
                        InvalidateRect(hwnd, std::ptr::null(), 0);
                    }
                }
            }
            0
        }
        WM_LBUTTONDOWN => {
            let x = (lparam & 0xFFFF) as i16 as i32;
            let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;

            if let Some(ui_arc) = get_ui() {
                let mut hit = HitZone::None;
                let mut lang_menu_pos: Option<(i32, i32, String)> = None;

                if let Ok(mut ui) = ui_arc.lock() {
                    for hb in &ui.hit_boxes {
                        if x >= hb.rect.left
                            && x <= hb.rect.right
                            && y >= hb.rect.top
                            && y <= hb.rect.bottom
                        {
                            hit = hb.zone;
                            if hit == HitZone::LangSelect {
                                let mut wrc = WinRect::default();
                                GetWindowRect(hwnd, &mut wrc);
                                lang_menu_pos = Some((
                                    wrc.left + hb.rect.left,
                                    wrc.top + hb.rect.bottom + 2,
                                    ui.current_locale.clone(),
                                ));
                            }
                            break;
                        }
                    }

                    match hit {
                        HitZone::ToggleMode => {
                            ui.show_remaining = !ui.show_remaining;
                            ui.persist_state();
                            InvalidateRect(ui.pill_hwnd, std::ptr::null(), 0);
                            InvalidateRect(hwnd, std::ptr::null(), 0);
                        }
                        HitZone::Autostart => {
                            let next = !ui.autostart_enabled;
                            if let Ok(actual) = set_autostart_sync(next) {
                                ui.autostart_enabled = actual;
                            }
                            InvalidateRect(hwnd, std::ptr::null(), 0);
                        }
                        HitZone::InfoToggle => {
                            ui.show_info = !ui.show_info;
                            InvalidateRect(hwnd, std::ptr::null(), 0);
                        }
                        HitZone::Quit => {
                            PostQuitMessage(0);
                        }
                        HitZone::OpacitySlider => {
                            ui.dragging_opacity = true;
                            let sl = ui.slider_rect;
                            let w = (sl.right - sl.left).max(1) as f32;
                            let norm = ((x - sl.left) as f32 / w).clamp(0.0, 1.0);
                            let pct = (45.0 + norm * 55.0).round() as u8;
                            ui.window_opacity = pct;
                            apply_flyout_style_and_opacity(hwnd, pct);
                            InvalidateRect(hwnd, std::ptr::null(), 0);
                        }
                        _ => {}
                    }
                }

                if hit == HitZone::Refresh {
                    trigger_async_refresh(&ui_arc, false);
                } else if hit == HitZone::SwitchAccount || hit == HitZone::LoginAction {
                    trigger_async_refresh(&ui_arc, true);
                } else if let Some((mx, my, cur_loc)) = lang_menu_pos {
                    let hmenu = CreatePopupMenu();
                    if hmenu != 0 {
                        for (idx, (code, label, _)) in LOCALE_LIST.iter().enumerate() {
                            let wide = to_wide(label);
                            let flags = if *code == cur_loc {
                                MF_STRING | MF_CHECKED
                            } else {
                                MF_STRING
                            };
                            AppendMenuW(hmenu, flags, idx + 1, wide.as_ptr());
                        }
                        SetForegroundWindow(hwnd);
                        let cmd = TrackPopupMenu(
                            hmenu,
                            TPM_LEFTALIGN | TPM_TOPALIGN | TPM_RETURNCMD | TPM_NONOTIFY,
                            mx,
                            my,
                            0,
                            hwnd,
                            std::ptr::null(),
                        );
                        DestroyMenu(hmenu);
                        if cmd > 0 && (cmd as usize) <= LOCALE_LIST.len() {
                            let chosen = LOCALE_LIST[cmd as usize - 1].0.to_string();
                            if let Ok(mut ui) = ui_arc.lock() {
                                ui.current_locale = chosen;
                                ui.persist_state();
                                InvalidateRect(ui.pill_hwnd, std::ptr::null(), 0);
                            }
                            InvalidateRect(hwnd, std::ptr::null(), 0);
                        }
                    }
                }
            }
            0
        }
        WM_LBUTTONUP => {
            if let Some(ui_arc) = get_ui() {
                if let Ok(mut ui) = ui_arc.lock() {
                    if ui.dragging_opacity {
                        ui.dragging_opacity = false;
                        ui.persist_state();
                    }
                }
            }
            0
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) & 0xFFFF) as i16;
            if let Some(ui_arc) = get_ui() {
                if let Ok(mut ui) = ui_arc.lock() {
                    let max_scroll = ui
                        .last_snapshot
                        .as_ref()
                        .map(|s| ((s.models.len() as i32 + 1) / 2 - 3).max(0) * 58)
                        .unwrap_or(0);
                    let step = if delta > 0 { -28 } else { 28 };
                    ui.scroll_offset_y = (ui.scroll_offset_y + step).clamp(0, max_scroll);
                    InvalidateRect(hwnd, std::ptr::null(), 0);
                }
            }
            0
        }
        WM_APP_SNAPSHOT_UPDATED => {
            InvalidateRect(hwnd, std::ptr::null(), 0);
            0
        }
        WM_CLOSE => {
            ShowWindow(hwnd, 0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

pub fn run_native_win32_ui(
    app_state: Arc<AppState>,
    rt_handle: tokio::runtime::Handle,
    init_rel_x: i32,
    init_opacity: u8,
    init_locale: String,
    init_show_rem: bool,
) {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let h_instance = GetModuleHandleW(std::ptr::null());

        let pill_class_name = to_wide("AGYPillNativeWnd");
        let flyout_class_name = to_wide("AGYFlyoutNativeWnd");

        let pill_wc = WndClassExW {
            cb_size: std::mem::size_of::<WndClassExW>() as u32,
            style: 0x0003, // CS_HREDRAW | CS_VREDRAW
            lpfn_wnd_proc: pill_wnd_proc,
            cb_cls_extra: 0,
            cb_wnd_extra: 0,
            h_instance,
            h_icon: 0,
            h_cursor: LoadCursorW(0, IDC_HAND),
            hbr_background: 0,
            lpsz_menu_name: std::ptr::null(),
            lpsz_class_name: pill_class_name.as_ptr(),
            h_icon_sm: 0,
        };
        RegisterClassExW(&pill_wc);

        let flyout_wc = WndClassExW {
            cb_size: std::mem::size_of::<WndClassExW>() as u32,
            style: 0x0003,
            lpfn_wnd_proc: flyout_wnd_proc,
            cb_cls_extra: 0,
            cb_wnd_extra: 0,
            h_instance,
            h_icon: 0,
            h_cursor: LoadCursorW(0, IDC_ARROW),
            hbr_background: 0,
            lpsz_menu_name: std::ptr::null(),
            lpsz_class_name: flyout_class_name.as_ptr(),
            h_icon_sm: 0,
        };
        RegisterClassExW(&flyout_wc);

        let pill_title = to_wide("AGY Taskbar Widget");
        let pill_hwnd = CreateWindowExW(
            (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) as u32,
            pill_class_name.as_ptr(),
            pill_title.as_ptr(),
            (WS_POPUP | WS_VISIBLE) as u32,
            0,
            0,
            PILL_WIDTH_LOGICAL as i32,
            PILL_HEIGHT_LOGICAL as i32,
            0,
            0,
            h_instance,
            std::ptr::null(),
        );

        let flyout_title = to_wide("Antigravity Quota Monitor");
        let main_hwnd = CreateWindowExW(
            (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) as u32,
            flyout_class_name.as_ptr(),
            flyout_title.as_ptr(),
            WS_POPUP as u32,
            100,
            100,
            FULL_WIDTH_LOGICAL as i32,
            FULL_HEIGHT_LOGICAL as i32,
            0,
            0,
            h_instance,
            std::ptr::null(),
        );

        apply_flyout_style_and_opacity(main_hwnd, init_opacity);

        let scale = get_hwnd_scale(pill_hwnd);
        let work = get_work_area_for_hwnd(pill_hwnd);
        let (rel_x, sampled_tb) = embed_pill_into_taskbar(pill_hwnd, init_rel_x, scale, work);

        let ui_shared = Arc::new(StdMutex::new(UiShared {
            app_state: Arc::clone(&app_state),
            rt_handle: rt_handle.clone(),
            locales: load_locales_map(),
            current_locale: init_locale,
            anchor_x: rel_x,
            is_dragging: false,
            taskbar_color: sampled_tb,
            window_opacity: init_opacity,
            show_remaining: init_show_rem,
            last_snapshot: None,
            pill_hwnd,
            main_hwnd,
            pill_mode_weekly: false,
            show_info: false,
            autostart_enabled: is_autostart_enabled_sync(),
            is_loading: true,
            error_code: None,
            hover_zone: HitZone::None,
            dragging_opacity: false,
            scroll_offset_y: 0,
            hit_boxes: Vec::with_capacity(12),
            slider_rect: WinRect::default(),
            enter_ticks: 0,
            leave_ticks: 0,
            last_tray_hwnd: find_taskbar_hwnd(),
            was_taskbar_fg: false,
        }));

        let _ = GLOBAL_UI.set(Arc::clone(&ui_shared));
        InvalidateRect(pill_hwnd, std::ptr::null(), 0);

        // Start Win32 timers on `pill_hwnd`
        SetTimer(pill_hwnd, TIMER_HOVER_POLL, 45, 0);
        SetTimer(pill_hwnd, TIMER_PILL_FLIP, 10_000, 0);
        SetTimer(pill_hwnd, TIMER_MINUTE_TICK, 60_000, 0);
        SetTimer(pill_hwnd, TIMER_API_POLL, 300_000, 0);

        // Kick off initial quota fetch
        trigger_async_refresh(&ui_shared, false);

        let mut msg = Msg::default();
        while GetMessageW(&mut msg, 0, 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

