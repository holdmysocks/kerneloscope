//! Kerneloscope: an animated, themed UEFI boot menu.
//!   main screen : one icon per OS over a drifting starfield, tools row, auto-boot countdown
//!   TAB         : bordered options panel with that OS's extra boot choices
//! Everything it draws is compiled in (assets/*.bin); its config is \EFI\kerneloscope\menu.conf.

#![no_std]
#![no_main]

extern crate alloc;

mod bg;
mod config;
mod edid;
mod gfx;
mod launch;
mod settings;
mod stars;

mod assets {
    include!(concat!(env!("OUT_DIR"), "/assets.rs"));
}

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;
use gfx::{Canvas, Font, Image};
use uefi::boot::{self, TimerTrigger};
use uefi::prelude::*;
use uefi::proto::console::gop::{BltOp, BltPixel, BltRegion, GraphicsOutput};
use uefi::proto::console::text::{Key, ScanCode};
use uefi::runtime::{self, ResetType, VariableAttributes, VariableVendor};

const ACCENT: u32 = 0x00ff_1f3d;
const TEXT: u32 = 0x00ec_ecf2;
const MUTED: u32 = 0x008e_8e9a;
const PANEL: u32 = 0x0016_070b;
const CONF_PATH: &str = "\\EFI\\kerneloscope\\menu.conf";

/// Progress/diagnostic line on the firmware text console (QEMU mirrors it to serial.log).
static DEBUG: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Diagnostics that print while the menu is on screen (frame rate, pacing) only appear with `debug on`:
/// on real hardware the firmware console draws text over the graphics.
fn debug_log(msg: &str) {
    if DEBUG.load(core::sync::atomic::Ordering::Relaxed) {
        log(msg);
    }
}

fn log(msg: &str) {
    // only with `debug on`: on real hardware the firmware console draws this text over the screen
    if !DEBUG.load(core::sync::atomic::Ordering::Relaxed) {
        return;
    }
    use core::fmt::Write;
    uefi::system::with_stdout(|o| {
        let _ = o.write_str(msg);
        let _ = o.write_str("\r\n");
    });
}

fn asset(name: &str) -> Option<&'static [u8]> {
    assets::ASSETS.iter().find(|(n, _)| *n == name).map(|(_, b)| *b)
}

// ------------------------------------------------------------------ timing (TSC, calibrated against boot-services stall)
fn rdtsc() -> u64 {
    unsafe { core::arch::x86_64::_rdtsc() }
}

struct Clock {
    hz: f32,
    last: u64,
}
impl Clock {
    fn new() -> Clock {
        let t0 = rdtsc();
        boot::stall(Duration::from_millis(50));
        let hz = ((rdtsc() - t0) * 20).max(1) as f32;
        Clock { hz, last: rdtsc() }
    }
    fn dt(&mut self) -> f32 {
        let now = rdtsc();
        let d = (now - self.last) as f32 / self.hz;
        self.last = now;
        d.min(0.25)
    }
}

// ------------------------------------------------------------------ theme (all images pre-scaled to the screen once)
struct Theme {
    s: f32,
    bg: Image,
    hints_main: Image,
    hints_sub: Image,
    hints_settings: Image,
    title_img: Image,
    sel_big: Image,
    sel_small: Image,
    arrow_l: Image,
    arrow_r: Image,
    tools: [Image; NTOOLS],
    label: Font,
    menu: Font,
    title: Font,
}

/// Assets are drawn at 2x the 2560x1600 layout (see tools/export-assets.ps1 -Density) so they stay
/// sharp on 4K / 5K screens; everything is downscaled once at start-up to the screen's layout scale.
const ASSET_DENSITY: f32 = 2.0;

fn load(name: &str, s: f32) -> Image {
    Image::decode(asset(name).unwrap_or_else(|| asset("os_linux").unwrap())).scaled(s / ASSET_DENSITY)
}

impl Theme {
    fn new(w: usize, h: usize) -> Theme {
        let s = (w as f32 / 2560.0).min(h as f32 / 1600.0);
        // background is generated for this exact screen size and shape
        let oy = ((h as f32 - 1600.0 * s) / 2.0) as i32;
        let bg = bg::generate(w, h, s, oy, &load("corner_tl", s), &load("corner_br", s));
        Theme {
            s,
            bg,
            hints_main: load("hints_main", s),
            hints_sub: load("hints_sub", s),
            title_img: load("title", s),
            sel_big: load("selection_big", s),
            sel_small: load("selection_small", s),
            arrow_l: load("arrow_left", s),
            arrow_r: load("arrow_right", s),
            tools: [load("func_shutdown", s), load("func_reset", s), load("func_firmware", s), load("func_menu", s)],
            hints_settings: load("hints_settings", s),
            label: Font::decode(asset("font_label").unwrap(), s / ASSET_DENSITY),
            menu: Font::decode(asset("font_menu").unwrap(), s / ASSET_DENSITY),
            title: Font::decode(asset("font_title").unwrap(), s / ASSET_DENSITY),
        }
    }
    fn px(&self, v: f32) -> i32 {
        (v * self.s) as i32
    }
}

// ------------------------------------------------------------------ menu model
struct Os {
    entry: config::Entry,
    icon: Image,
    icon_small: Image,
    volume: Option<uefi::Handle>,
}

enum Action {
    Boot(usize, Option<usize>), // entry, sub-choice (None = main)
    Shutdown,
    Reboot,
    Firmware,
}

fn smoother(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// 0..1 smooth pulse without libm
fn pulse(t: f32) -> f32 {
    let f = t - (t as i32) as f32;
    let tri = if f < 0.5 { f * 2.0 } else { 2.0 - f * 2.0 };
    ease(tri)
}

struct Ui {
    sel: usize,
    focus_tools: bool,
    tool: usize,
    scroll: usize,
    sub: Option<usize>,
    sub_anim: f32, // 0..1 panel open animation
    countdown: Option<f32>,
    time: f32,
    message: Option<(String, f32)>,
    intro: Option<f32>, // seconds into the start-up animation; None once it has finished or was skipped
    settings: Option<SettingsUi>,
    prefs: Prefs,
}

/// Tools row: shut down, restart, firmware setup, menu settings.
const NTOOLS: usize = 4;
const TOOL_LABELS: [&str; NTOOLS] = ["Shut down", "Restart", "Firmware settings (BIOS)", "Menu settings"];

fn draw(cv: &mut Canvas, th: &Theme, oses: &[Os], ui: &Ui, stars: &stars::Starfield) {
    let (w, h) = (cv.w as i32, cv.h as i32);
    let cx = w / 2;
    let oy = (h - th.px(1600.0)) / 2;
    cv.copy_from(&th.bg);
    // start-up: background rises out of black while the particles burst, then the menu fades/slides in
    let it = ui.intro.unwrap_or(f32::MAX);
    let stage = |start: f32, len: f32| smoother((it - start) / len);
    let bg_a = stage(0.5, 2.1);
    if bg_a < 1.0 {
        cv.fill_rect(0, 0, w, h, 0, ((1.0 - bg_a) * 256.0) as u32);
    }
    stars.draw(cv, ui.intro);
    if let Some(s) = &ui.settings {
        draw_settings(cv, th, oses, s);
        return;
    }
    if ui.sub.is_some() {
        draw_panel(cv, th, oses, ui);
        return;
    }
    // staggered fade-in at the end of the intro: title, OS row, tools + label, hints
    let end = stars::INTRO_SECONDS;
    let (a_title, a_row, a_tools, a_hint) = (stage(end - 1.7, 1.1), stage(end - 1.4, 1.1), stage(end - 1.15, 1.0), stage(end - 1.0, 1.0));
    let mul = |a: u32, k: f32| (a as f32 * k) as u32;
    let lift = |k: f32| ((1.0 - k) * th.px(46.0) as f32) as i32;
    if a_title <= 0.0 {
        return;
    }
    cv.draw(&th.title_img, cx - th.title_img.w as i32 / 2, oy + th.px(400.0) + lift(a_title) / 2, mul(256, a_title));
    let m = |a: u32| mul(a, a_row);
    let rise = lift(a_row);
    let mt = |a: u32| mul(a, a_tools);
    let rise_t = lift(a_tools);

    // ---- OS row: centred, scrolls with arrows when it doesn't fit
    let tile = th.px(300.0);
    let n = oses.len();
    let fit = (((w - th.px(420.0)) / tile).max(1) as usize).min(n);
    let row_y = oy + th.px(800.0) + rise;
    let x0 = cx - (fit as i32 * tile) / 2 + tile / 2;
    let breathe = m(200 + (56.0 * pulse(ui.time * 0.45)) as u32);
    for (slot, i) in (ui.scroll..(ui.scroll + fit).min(n)).enumerate() {
        let x = x0 + slot as i32 * tile;
        let selected = i == ui.sel && !ui.focus_tools;
        if selected {
            cv.draw_centered(&th.sel_big, x, row_y, breathe);
        }
        cv.draw_centered(&oses[i].icon, x, row_y, m(if selected || ui.focus_tools { 256 } else { 215 }));
    }
    if ui.scroll > 0 {
        cv.draw_centered(&th.arrow_l, x0 - tile * 3 / 4, row_y, m(256));
    }
    if ui.scroll + fit < n {
        cv.draw_centered(&th.arrow_r, x0 + (fit as i32 - 1) * tile + tile * 3 / 4, row_y, m(256));
    }

    // ---- tools row
    let ty = oy + th.px(1022.0) + rise_t;
    let tstep = th.px(136.0);
    for (i, img) in th.tools.iter().enumerate() {
        let x = cx + ((2 * i as i32 - (NTOOLS as i32 - 1)) * tstep) / 2;
        if ui.focus_tools && ui.tool == i {
            cv.draw_centered(&th.sel_small, x, ty, mt(200 + (56.0 * pulse(ui.time * 0.45)) as u32));
        }
        cv.draw_centered(img, x, ty, mt(256));
    }

    // ---- label + countdown / status message
    let label = if ui.focus_tools {
        TOOL_LABELS[ui.tool]
    } else {
        oses[ui.sel].entry.title.as_str()
    };
    let up = |s: &str| s.chars().map(|c| c.to_ascii_uppercase()).collect::<String>();
    th.label.draw_centered(cv, &up(label), cx, oy + th.px(1095.0) + rise_t, TEXT, mt(256), th.px(5.0));
    if let Some((m, _)) = &ui.message {
        th.menu.draw_centered(cv, m, cx, oy + th.px(1160.0), ACCENT, 256, 0);
    } else if let Some(t) = ui.countdown {
        let secs = t as u32 + 1;
        let msg = format!("Starting {} in {}s   -   press any key to stop", oses[ui.sel].entry.title, secs);
        th.menu.draw_centered(cv, &msg, cx, oy + th.px(1160.0), MUTED, 200, 0);
    }

    cv.draw_centered(&th.hints_main, cx, h - th.px(165.0) - oy + lift(a_hint) / 2, mul(256, a_hint));
}

/// Bordered, cut-corner panel with a header (icon, title, subtitle) and a list of rows. A row with a
/// value shows it right-aligned between < > arrows (settings page); plain rows are actions.
fn draw_list_panel(cv: &mut Canvas, th: &Theme, icon: &Image, title: &str, subtitle: &str, rows: &[(String, Option<String>)], sel: usize, anim: f32, hints: &Image) {
    let (w, h) = (cv.w as i32, cv.h as i32);
    let cx = w / 2;
    let oy = (h - th.px(1600.0)) / 2;
    let up = |s: &str| s.chars().map(|c| c.to_ascii_uppercase()).collect::<String>();
    let a = ease(anim);
    let pad = th.px(56.0);
    let row_h = th.px(66.0);
    let head_h = th.px(150.0);
    let gap = th.px(80.0);
    let label_w = rows.iter().map(|(t, _)| th.menu.width(t, 0)).max().unwrap_or(0);
    let value_w = rows.iter().filter_map(|(_, v)| v.as_ref()).map(|v| th.menu.width(&format!("<  {}  >", v), 0)).max().unwrap_or(0);
    let content_w = label_w + if value_w > 0 { gap + value_w } else { 0 };
    let pw = (content_w + pad * 2 + th.px(40.0)).max(th.px(980.0));
    let ph = head_h + rows.len() as i32 * row_h + pad / 2;
    // panel slides up slightly while fading in
    let px0 = cx - pw / 2;
    let py0 = (h - ph) / 2 - th.px(20.0) + ((1.0 - a) * th.px(40.0) as f32) as i32;
    let cut = th.px(34.0);
    let fa = (a * 256.0) as u32;
    cv.cut_glow(px0, py0, pw, ph, cut, ACCENT, (110.0 * a) as u32, th.px(14.0).max(4));
    cv.cut_frame(px0, py0, pw, ph, cut, th.px(3.0).max(2), ACCENT, fa, PANEL, (240.0 * a) as u32);
    // header: small icon + title
    cv.draw(icon, px0 + pad - th.px(10.0), py0 + (head_h - icon.h as i32) / 2, fa);
    let hx = px0 + pad + icon.w as i32 + th.px(14.0);
    th.title.draw(cv, &up(title), hx, py0 + th.px(42.0), TEXT, fa, th.px(4.0));
    th.label.draw(cv, subtitle, hx, py0 + th.px(88.0), ACCENT, (fa * 3) / 4, th.px(6.0));
    cv.fill_rect(px0 + pad, py0 + head_h - th.px(8.0), pw - pad * 2, th.px(2.0).max(1), ACCENT, fa / 3);
    // rows
    for (i, (t, value)) in rows.iter().enumerate() {
        let y = py0 + head_h + i as i32 * row_h;
        let active = i == sel;
        if active {
            cv.fill_rect(px0 + pad - th.px(16.0), y + th.px(6.0), pw - pad * 2 + th.px(32.0), row_h - th.px(12.0), ACCENT, fa / 5);
            cv.fill_rect(px0 + pad - th.px(16.0), y + th.px(6.0), th.px(6.0).max(2), row_h - th.px(12.0), ACCENT, fa);
        }
        let ty = y + (row_h - th.menu.cell_h as i32) / 2 + th.px(4.0);
        th.menu.draw(cv, t, px0 + pad + th.px(10.0), ty, if active { 0x00ff_ffff } else { 0x00b0_b0bc }, fa, 0);
        if let Some(v) = value {
            // right-aligned "<  value  >", arrows in the accent colour on the active row
            let right = px0 + pw - pad - th.px(10.0);
            let vw = th.menu.width(v, 0);
            let arrow_w = th.menu.width(">", 0);
            let sp = th.menu.width("  ", 0);
            let arrow_c = if active { ACCENT } else { 0x0060_6070 };
            th.menu.draw(cv, ">", right - arrow_w, ty, arrow_c, fa, 0);
            th.menu.draw(cv, v, right - arrow_w - sp - vw, ty, if active { 0x00ff_ffff } else { 0x00d0_d0da }, fa, 0);
            th.menu.draw(cv, "<", right - arrow_w - sp - vw - sp - arrow_w, ty, arrow_c, fa, 0);
        }
    }
    cv.draw_centered(hints, cx, h - th.px(165.0) - oy, 256);
}

/// TAB options panel: this OS's boot choices.
fn draw_panel(cv: &mut Canvas, th: &Theme, oses: &[Os], ui: &Ui) {
    let Some(sel) = ui.sub else { return };
    let os = &oses[ui.sel];
    let mut rows: Vec<(String, Option<String>)> = Vec::new();
    rows.push((format!("Boot {}", os.entry.title), None));
    for s in &os.entry.subs {
        rows.push((s.title.clone(), None));
    }
    rows.push((String::from("Back to main menu"), None));
    draw_list_panel(cv, th, &os.icon_small, &os.entry.title, "BOOT OPTIONS", &rows, sel, ui.sub_anim, &th.hints_sub);
}

/// Countdown choices offered on the settings page (0 = no countdown, wait for a key).
const TIMEOUTS: [u32; 7] = [0, 3, 5, 8, 10, 15, 30];

struct SettingsUi {
    row: usize,         // 0 default OS, 1 countdown, 2 animation, 3 save, 4 cancel
    default_idx: usize, // 0 = last used, n = oses[n - 1]
    timeout_idx: usize,
    intro: bool,
    anim: f32,
}

/// Effective preferences (menu.conf overridden by values saved from the settings page).
struct Prefs {
    default_last: bool,
    default_title: String,
    timeout: u32,
    intro: bool,
}

fn settings_rows(oses: &[Os], s: &SettingsUi) -> Vec<(String, Option<String>)> {
    let default = if s.default_idx == 0 { String::from("Last used") } else { oses[s.default_idx - 1].entry.title.clone() };
    let t = TIMEOUTS[s.timeout_idx];
    let timeout = if t == 0 { String::from("Off") } else { format!("{} s", t) };
    alloc::vec![
        (String::from("Start by default"), Some(default)),
        (String::from("Countdown"), Some(timeout)),
        (String::from("Startup animation"), Some(String::from(if s.intro { "On" } else { "Off" }))),
        (String::from("Save"), None),
        (String::from("Cancel"), None),
    ]
}

fn open_settings(p: &Prefs, oses: &[Os]) -> SettingsUi {
    let default_idx = if p.default_last { 0 } else { oses.iter().position(|o| o.entry.title == p.default_title).map(|i| i + 1).unwrap_or(0) };
    // nearest offered countdown to the current one
    let timeout_idx = (0..TIMEOUTS.len()).min_by_key(|&i| (TIMEOUTS[i] as i32 - p.timeout as i32).abs()).unwrap_or(0);
    SettingsUi { row: 0, default_idx, timeout_idx, intro: p.intro, anim: 0.0 }
}

/// Keys on the settings page: up/down pick a row, left/right change its value, Enter saves, Esc cancels.
fn settings_key(ui: &mut Ui, oses: &[Os], k: Key) {
    let Some(s) = ui.settings.as_mut() else { return };
    const ROWS: usize = 5;
    let step = |s: &mut SettingsUi, fwd: bool| match s.row {
        0 => {
            let n = oses.len() + 1;
            s.default_idx = if fwd { (s.default_idx + 1) % n } else { (s.default_idx + n - 1) % n };
        }
        1 => {
            let n = TIMEOUTS.len();
            s.timeout_idx = if fwd { (s.timeout_idx + 1).min(n - 1) } else { s.timeout_idx.saturating_sub(1) };
        }
        2 => s.intro = !s.intro,
        _ => {}
    };
    match k {
        Key::Special(ScanCode::UP) => s.row = (s.row + ROWS - 1) % ROWS,
        Key::Special(ScanCode::DOWN) => s.row = (s.row + 1) % ROWS,
        Key::Special(ScanCode::LEFT) => step(s, false),
        Key::Special(ScanCode::RIGHT) => step(s, true),
        Key::Special(ScanCode::ESCAPE) => ui.settings = None,
        Key::Printable(c) if c == '\r' || c == ' ' => {
            if s.row == 4 {
                ui.settings = None;
                return;
            }
            let default_last = s.default_idx == 0;
            let default_title = if default_last { String::new() } else { oses[s.default_idx - 1].entry.title.clone() };
            let saved = settings::Saved {
                default: Some(if default_last { String::from(settings::LAST) } else { default_title.clone() }),
                timeout: Some(TIMEOUTS[s.timeout_idx]),
                intro: Some(s.intro),
            };
            let ok = settings::save(&saved);
            ui.prefs = Prefs { default_last, default_title, timeout: TIMEOUTS[s.timeout_idx], intro: s.intro };
            ui.settings = None;
            ui.message = Some((String::from(if ok { "Settings saved" } else { "Couldn't save settings (firmware refused the write)" }), 3.0));
        }
        _ => {}
    }
}

fn draw_settings(cv: &mut Canvas, th: &Theme, oses: &[Os], s: &SettingsUi) {
    draw_list_panel(cv, th, &th.tools[3], "Kerneloscope", "MENU SETTINGS", &settings_rows(oses, s), s.row, s.anim, &th.hints_settings);
}

fn present(gop: &mut GraphicsOutput, cv: &Canvas) {
    // Canvas pixels are 0x00RRGGBB, byte order B,G,R,0 - identical to BltPixel
    let buf: &[BltPixel] = unsafe { core::slice::from_raw_parts(cv.px.as_ptr().cast(), cv.px.len()) };
    let _ = gop.blt(BltOp::BufferToVideo { buffer: buf, src: BltRegion::Full, dest: (0, 0), dims: (cv.w, cv.h) });
}

fn read_key() -> Option<Key> {
    uefi::system::with_stdin(|i| i.read_key()).ok().flatten()
}

/// Runs the menu until the user picks something. Owns the screen only while running, so the OS being
/// started afterwards can take over graphics output.
fn open_gop() -> boot::ScopedProtocol<GraphicsOutput> {
    let gh = boot::get_handle_for_protocol::<GraphicsOutput>().expect("no graphics");
    boot::open_protocol_exclusive::<GraphicsOutput>(gh).expect("gop")
}

/// Picks the video mode from menu.conf's `resolution`:
///   native (default) - the display's preferred resolution from its EDID, otherwise the largest mode
///   max              - the largest mode offered
///   auto             - keep the mode the firmware already set (often lower than native, so blurry)
///   <W>x<H>          - that exact mode if offered, otherwise auto
fn screen_size(want: &str) -> (usize, usize) {
    let native = if want == "native" { edid::native_resolution() } else { None };
    let mut gop = open_gop();
    let largest = |gop: &GraphicsOutput| {
        gop.modes().max_by_key(|m| {
            let (w, h) = m.info().resolution();
            w * h
        })
    };
    let target = match want {
        "native" => match native {
            Some(r) => gop.modes().find(|m| m.info().resolution() == r).or_else(|| largest(&gop)),
            None => largest(&gop),
        },
        "max" => largest(&gop),
        s => s.split_once(['x', 'X']).and_then(|(a, b)| Some((a.trim().parse::<usize>().ok()?, b.trim().parse::<usize>().ok()?)))
            .and_then(|r| {
                let found = gop.modes().find(|m| m.info().resolution() == r);
                if found.is_none() {
                    let list: Vec<String> = gop.modes().map(|m| { let (w, h) = m.info().resolution(); format!("{}x{}", w, h) }).collect();
                    log(&format!("kerneloscope: {}x{} not offered by the firmware; modes: {}", r.0, r.1, list.join(" ")));
                }
                found
            }),
    };
    if let Some(m) = target {
        if m.info().resolution() != gop.current_mode_info().resolution() {
            let _ = gop.set_mode(&m);
        }
    }
    gop.current_mode_info().resolution()
}

fn run_menu(oses: &[Os], ui: &mut Ui, stars: &mut stars::Starfield, th: &Theme) -> Action {
    let mut gop = open_gop();
    let (w, h) = (th.bg.w, th.bg.h);
    let mut cv = Canvas::new(w, h);
    let mut clock = Clock::new();
    let mut perf = (0.0f32, 0.0f32, 0u32, 0.0f32); // draw s, present s, frames, elapsed s
    let (mut nap_ok, mut nap_misses) = (true, 0u32);
    let nap = unsafe { boot::create_event(boot::EventType::TIMER, boot::Tpl::APPLICATION, None, None) }.expect("timer");
    let tile = th.px(300.0);
    let fit = (((w as i32 - th.px(420.0)) / tile).max(1) as usize).min(oses.len());

    let action = loop {
        let dt = clock.dt();
        ui.time += dt;
        stars.update(dt);
        if let Some(t) = ui.intro.as_mut() {
            *t += dt;
            if *t >= stars::INTRO_SECONDS {
                ui.intro = None;
            }
        }
        // the auto-boot countdown only starts once the intro has played
        if let Some(t) = ui.countdown.as_mut().filter(|_| ui.intro.is_none()) {
            *t -= dt;
            if *t <= 0.0 {
                break Action::Boot(ui.sel, None);
            }
        }
        if let Some((_, t)) = ui.message.as_mut() {
            *t -= dt;
            if *t <= 0.0 {
                ui.message = None;
            }
        }
        if ui.sub.is_some() {
            ui.sub_anim = (ui.sub_anim + dt * 5.0).min(1.0);
        }
        if let Some(s) = ui.settings.as_mut() {
            s.anim = (s.anim + dt * 5.0).min(1.0);
        }

        // ---- input (drain everything that arrived since the last frame)
        let mut chosen = None;
        while let Some(k) = read_key() {
            if ui.intro.is_some() {
                // first key just skips the intro (the countdown keeps running)
                ui.intro = None;
                continue;
            }
            if ui.settings.is_some() {
                settings_key(ui, oses, k);
                continue;
            }
            ui.countdown = None;
            let n_sub = oses[ui.sel].entry.subs.len() + 2;
            match (ui.sub, k) {
                (Some(i), Key::Special(ScanCode::UP)) => ui.sub = Some((i + n_sub - 1) % n_sub),
                (Some(i), Key::Special(ScanCode::DOWN)) => ui.sub = Some((i + 1) % n_sub),
                (Some(_), Key::Special(ScanCode::ESCAPE)) => ui.sub = None,
                (Some(_), Key::Printable(c)) if c == '\t' => ui.sub = None,
                (Some(i), Key::Printable(c)) if c == '\r' => {
                    if i == n_sub - 1 {
                        ui.sub = None;
                    } else {
                        chosen = Some(Action::Boot(ui.sel, if i == 0 { None } else { Some(i - 1) }));
                    }
                }
                (None, Key::Special(ScanCode::LEFT)) => {
                    if ui.focus_tools { ui.tool = (ui.tool + NTOOLS - 1) % NTOOLS } else if ui.sel > 0 { ui.sel -= 1 }
                }
                (None, Key::Special(ScanCode::RIGHT)) => {
                    if ui.focus_tools { ui.tool = (ui.tool + 1) % NTOOLS } else if ui.sel + 1 < oses.len() { ui.sel += 1 }
                }
                (None, Key::Special(ScanCode::DOWN)) => ui.focus_tools = true,
                (None, Key::Special(ScanCode::UP)) => ui.focus_tools = false,
                (None, Key::Special(ScanCode::FUNCTION_2)) | (None, Key::Special(ScanCode::INSERT)) if !ui.focus_tools => {
                    ui.sub = Some(0);
                    ui.sub_anim = 0.0;
                }
                (None, Key::Printable(c)) if c == '\t' && !ui.focus_tools => {
                    ui.sub = Some(0);
                    ui.sub_anim = 0.0;
                }
                (None, Key::Printable(c)) if (c == '\r' || c == ' ') && ui.focus_tools && ui.tool == 3 => {
                    ui.settings = Some(open_settings(&ui.prefs, oses));
                }
                (None, Key::Printable(c)) if c == '\r' || c == ' ' => {
                    chosen = Some(if ui.focus_tools {
                        [Action::Shutdown, Action::Reboot, Action::Firmware].into_iter().nth(ui.tool).unwrap()
                    } else {
                        Action::Boot(ui.sel, None)
                    });
                }
                _ => {}
            }
        }
        if let Some(a) = chosen {
            break a;
        }
        // keep the selected icon inside the visible window
        if ui.sel < ui.scroll {
            ui.scroll = ui.sel;
        }
        if ui.sel >= ui.scroll + fit {
            ui.scroll = ui.sel + 1 - fit;
        }

        let t0 = rdtsc();
        draw(&mut cv, th, oses, ui, stars);
        let t1 = rdtsc();
        present(&mut gop, &cv);
        let t2 = rdtsc();
        perf.0 += (t1 - t0) as f32 / clock.hz;
        perf.1 += (t2 - t1) as f32 / clock.hz;
        perf.2 += 1;
        perf.3 += dt;
        if perf.3 >= 2.0 {
            debug_log(&format!("kerneloscope perf: {:.1} fps, draw {:.1} ms, present {:.1} ms",
                perf.2 as f32 / perf.3, perf.0 * 1000.0 / perf.2 as f32, perf.1 * 1000.0 / perf.2 as f32));
            perf = (0.0, 0.0, 0, 0.0);
        }
        // pace to ~60 fps. Sleep on a firmware timer (the CPU halts instead of spinning, which matters
        // a lot inside a VM), then busy-wait only the last sliver for precise frame timing.
        let budget = 1.0 / 60.0;
        let spent = (rdtsc() - t0) as f32 / clock.hz;
        let left = budget - spent;
        if nap_ok && left > 0.004 {
            let want = left - 0.003;
            let before = rdtsc();
            if boot::set_timer(&nap, TimerTrigger::Relative(Duration::from_micros((want * 1_000_000.0) as u64))).is_ok() {
                let _ = boot::wait_for_event(&[unsafe { nap.unsafe_clone() }]);
            }
            // firmware timers are coarse on some machines and in VMs: if naps overshoot badly, stop napping
            let took = (rdtsc() - before) as f32 / clock.hz;
            if took > want * 1.5 + 0.003 {
                nap_misses += 1;
                if nap_misses >= 3 {
                    nap_ok = false;
                    debug_log("kerneloscope: firmware timer too coarse, pacing with the CPU clock instead");
                }
            }
        }
        // finish the frame on the TSC (no firmware calls, so no VM exits)
        while ((rdtsc() - t0) as f32 / clock.hz) < budget {
            core::hint::spin_loop();
        }
    };
    let _ = boot::close_event(nap);
    // hand over a clean black screen
    let _ = gop.blt(BltOp::VideoFill { color: BltPixel::new(0, 0, 0), dest: (0, 0), dims: (w, h) });
    action
}

fn boot_to_firmware() {
    let name = cstr16!("OsIndications");
    let mut buf = [0u8; 8];
    let cur = match runtime::get_variable(name, &VariableVendor::GLOBAL_VARIABLE, &mut buf) {
        Ok((d, _)) if d.len() == 8 => u64::from_le_bytes(buf),
        _ => 0,
    };
    let attrs = VariableAttributes::NON_VOLATILE | VariableAttributes::BOOTSERVICE_ACCESS | VariableAttributes::RUNTIME_ACCESS;
    let _ = runtime::set_variable(name, &VariableVendor::GLOBAL_VARIABLE, attrs, &(cur | 1).to_le_bytes());
    runtime::reset(ResetType::COLD, Status::SUCCESS, None);
}

#[entry]
fn main() -> Status {
    let _ = boot::set_watchdog_timer(0, 0x10000, None);
    // blank and hide the firmware text console straight away (no stray text before the graphics start)
    uefi::system::with_stdout(|o| {
        let _ = o.enable_cursor(false);
        let _ = o.clear();
    });
    let text = launch::read_own_file(CONF_PATH).and_then(|b| String::from_utf8(b).ok()).unwrap_or_default();
    let mut cfg = config::parse(text.trim_start_matches('\u{feff}'));
    if cfg.entries.is_empty() {
        // no config: offer whatever we can find
        for (t, i, l) in [("Windows", "win", "\\EFI\\Microsoft\\Boot\\bootmgfw.efi"), ("rEFInd", "linux", "\\EFI\\refind\\refind_x64.efi")] {
            let mut e = config::Entry { title: t.into(), icon: i.into(), ..Default::default() };
            e.main.loader = l.into();
            cfg.entries.push(e);
        }
    }
    DEBUG.store(cfg.debug, core::sync::atomic::Ordering::Relaxed);
    log(&format!("kerneloscope: {} entries in config", cfg.entries.len()));
    let (w, h) = screen_size(&cfg.resolution);
    log(&format!("kerneloscope: screen {}x{}", w, h));
    let theme = Theme::new(w, h);
    log("kerneloscope: theme loaded");
    // keep only entries whose loader exists somewhere (so a missing OS never shows up as a dead icon)
    let oses: Vec<Os> = cfg
        .entries
        .into_iter()
        .filter_map(|e| {
            let volume = launch::find(&e.main.loader);
            volume?;
            let icon = Image::decode(asset(&format!("os_{}", e.icon)).unwrap_or_else(|| asset("os_linux").unwrap()));
            Some(Os { icon_small: icon.scaled(theme.s * 0.5 / ASSET_DENSITY), icon: icon.scaled(theme.s / ASSET_DENSITY), entry: e, volume })
        })
        .collect();
    log(&format!("kerneloscope: {} bootable entries found", oses.len()));
    if oses.is_empty() {
        return Status::NOT_FOUND;
    }
    // settings saved from the in-menu settings page win over menu.conf
    let saved = settings::load();
    let default_last = saved.default.as_deref() == Some(settings::LAST);
    let default_title = match &saved.default {
        Some(d) if d == settings::LAST => String::new(),
        Some(d) => d.clone(),
        None => cfg.default.clone(),
    };
    let prefs = Prefs {
        default_last,
        default_title: default_title.clone(),
        timeout: saved.timeout.unwrap_or(cfg.timeout),
        intro: saved.intro.unwrap_or(cfg.intro),
    };
    let start_title = if default_last { settings::last().unwrap_or(cfg.default.clone()) } else { default_title };
    log(&format!("kerneloscope: default {:?} (last used: {}), countdown {}s, intro {}", start_title, default_last, prefs.timeout, prefs.intro));
    let mut ui = Ui {
        sel: oses.iter().position(|o| o.entry.title == start_title).unwrap_or(0),
        focus_tools: false,
        tool: 0,
        scroll: 0,
        sub: None,
        sub_anim: 0.0,
        countdown: if prefs.timeout > 0 { Some(prefs.timeout as f32) } else { None },
        time: 0.0,
        message: None,
        intro: if prefs.intro { Some(0.0) } else { None },
        settings: None,
        prefs,
    };
    let mut stars = stars::Starfield::new(w, h, theme.s, 0x5eed_1234);

    loop {
        let action = run_menu(&oses, &mut ui, &mut stars, &theme);
        match action {
            Action::Shutdown => runtime::reset(ResetType::SHUTDOWN, Status::SUCCESS, None),
            Action::Reboot => runtime::reset(ResetType::COLD, Status::SUCCESS, None),
            Action::Firmware => boot_to_firmware(),
            Action::Boot(i, sub) => {
                let os = &oses[i];
                let c = match sub { Some(k) => &os.entry.subs[k], None => &os.entry.main };
                let vol = if c.loader == os.entry.main.loader { os.volume } else { launch::find(&c.loader) };
                log(&format!("kerneloscope: starting {} ({})", c.title, c.loader));
                settings::set_last(&os.entry.title);
                let result = match vol {
                    Some(h) => launch::start(h, &c.loader, &c.options),
                    None => Err(String::from("file not found")),
                };
                ui.sub = None;
                if let Err(e) = result {
                    ui.message = Some((format!("Couldn't start {}: {}", c.title, e), 5.0));
                }
            }
        }
    }
}
