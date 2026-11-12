//! Dark, low-key theme in the spirit of Crunchyroll: near-black background,
//! charcoal surfaces, one orange accent. Fonts: Thai + CJK fallbacks + Phosphor icons.

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle,
    Visuals,
};
use std::path::PathBuf;

pub const BG: Color32 = Color32::from_rgb(0x0c, 0x0c, 0x0e);
pub const BAR: Color32 = Color32::from_rgb(0x14, 0x14, 0x16);
pub const PANEL: Color32 = Color32::from_rgb(0x23, 0x24, 0x28);
pub const PANEL_HI: Color32 = Color32::from_rgb(0x2f, 0x30, 0x35);
pub const CARD: Color32 = Color32::from_rgb(0x22, 0x22, 0x25);
pub const CARD_HI: Color32 = Color32::from_rgb(0x2c, 0x2a, 0x2b);
pub const SELECT: Color32 = Color32::from_rgb(0x3d, 0x1f, 0x17);
pub const LINE: Color32 = Color32::from_rgb(0x34, 0x35, 0x3b);
pub const ACCENT: Color32 = Color32::from_rgb(0xee, 0x55, 0x24);
pub const ACCENT_HI: Color32 = Color32::from_rgb(0xff, 0x70, 0x3e);
pub const TEXT: Color32 = Color32::from_rgb(0xf2, 0xf2, 0xf2);
pub const MUTED: Color32 = Color32::from_rgb(0xa0, 0xa1, 0xa8);
pub const DIM: Color32 = Color32::from_rgb(0x6b, 0x6e, 0x76);
pub const GOOD: Color32 = Color32::from_rgb(0x2a, 0xbd, 0x6c);
pub const WARN: Color32 = Color32::from_rgb(0xf2, 0xc0, 0x37);
pub const BAD: Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);

pub const BOLD: &str = "bold";

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BOLD.into()))
}

pub fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}

pub fn setup(ctx: &egui::Context) {
    setup_fonts(ctx);

    let mut v = Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.window_stroke = Stroke::new(1.0, LINE);
    v.window_corner_radius = CornerRadius::same(8);
    v.extreme_bg_color = BAR;
    v.faint_bg_color = PANEL;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    let r = CornerRadius::same(4);
    for (w, fill) in [
        (&mut v.widgets.noninteractive, BG),
        (&mut v.widgets.inactive, PANEL),
        (&mut v.widgets.hovered, PANEL_HI),
        (&mut v.widgets.active, LINE),
        (&mut v.widgets.open, PANEL_HI),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.corner_radius = r;
        w.bg_stroke = Stroke::NONE;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    // an outline so unchecked boxes, text fields and combo boxes can be seen on the dark panels,
    // turning to the accent colour under the pointer
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x4a, 0x4c, 0x55));
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT_HI);
    v.widgets.open.bg_stroke = Stroke::new(1.0, ACCENT);
    v.menu_corner_radius = CornerRadius::same(8);
    v.window_corner_radius = CornerRadius::same(10);
    v.popup_shadow = egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(140) };
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, MUTED);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    v.widgets.active.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    // Always the dark look, whatever the Windows theme is: egui keeps one style per theme and follows the system
    // by default, so on a light-mode PC the widgets (combo boxes, sliders, menus) came out white on our dark pages.
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, v.clone());
    ctx.set_visuals_of(egui::Theme::Light, v);

    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 8.0);
        s.spacing.button_padding = egui::vec2(12.0, 6.0);
        s.spacing.interact_size.y = 30.0;
        s.text_styles = [
            (TextStyle::Small, regular(12.5)),
            (TextStyle::Body, regular(15.0)),
            (TextStyle::Button, regular(15.0)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
            (TextStyle::Heading, bold(22.0)),
        ]
        .into();
    });
}

// ---------------------------------------------------------------- fonts

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(|p| p.to_path_buf())
}

fn read_first(cands: &[&str]) -> Option<(String, Vec<u8>)> {
    cands
        .iter()
        .find_map(|p| std::fs::read(p).ok().map(|b| (p.to_string(), b)))
}

fn win(f: &str) -> String {
    let root = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    format!("{root}\\Fonts\\{f}")
}

/// Fonts dropped in `fonts/` next to the exe win (e.g. Kanit / Noto Sans Thai):
/// a file whose name contains "bold" becomes the bold face, the first other one the body face.
fn bundled() -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    let mut reg = None;
    let mut bold = None;
    let Some(dir) = exe_dir() else {
        return (None, None);
    };
    for sub in ["fonts", "assets/fonts"] {
        let Ok(rd) = std::fs::read_dir(dir.join(sub)) else {
            continue;
        };
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        files.sort();
        for p in files {
            let name = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_lowercase();
            if !(name.ends_with(".ttf") || name.ends_with(".otf")) {
                continue;
            }
            if name.contains("bold") || name.contains("semibold") {
                if bold.is_none() {
                    bold = std::fs::read(&p).ok();
                }
            } else if reg.is_none()
                && !name.contains("italic")
                && !name.contains("light")
                && !name.contains("thin")
            {
                reg = std::fs::read(&p).ok();
            }
        }
    }
    (reg, bold)
}

fn setup_fonts(ctx: &egui::Context) {
    let mut f = FontDefinitions::default();
    let w = |x: &str| win(x);

    let (b_reg, b_bold) = bundled();
    let sys_reg = read_first(&[
        &w("LeelawUI.ttf"),
        &w("leelawad.ttf"),
        &w("tahoma.ttf"),
        "/usr/share/fonts/truetype/noto/NotoSansThai-Regular.ttf",
        "/usr/share/fonts/noto/NotoSansThai-Regular.ttf",
        "/usr/share/fonts/opentype/noto/NotoSansThai-Regular.otf",
        "/usr/share/fonts/opentype/tlwg/Loma.otf",
        "/System/Library/Fonts/Supplemental/Thonburi.ttc",
    ])
    .map(|x| x.1);
    let sys_bold = read_first(&[
        &w("LeelaUIb.ttf"),
        &w("leelawdb.ttf"),
        &w("tahomabd.ttf"),
        "/usr/share/fonts/truetype/noto/NotoSansThai-Bold.ttf",
        "/usr/share/fonts/noto/NotoSansThai-Bold.ttf",
        "/usr/share/fonts/opentype/tlwg/Loma-Bold.otf",
    ])
    .map(|x| x.1);
    let cjk = read_first(&[
        &w("YuGothM.ttc"),
        &w("meiryo.ttc"),
        &w("msgothic.ttc"),
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    ])
    .map(|x| x.1);

    let body = b_reg.or(sys_reg);
    let heavy = b_bold.or(sys_bold);

    let prop = f.families.entry(FontFamily::Proportional).or_default();
    if let Some(b) = body {
        f.font_data
            .insert("body".into(), FontData::from_owned(b).into());
        prop.insert(0, "body".into()); // Thai-capable body font first
    }
    if let Some(c) = cjk {
        f.font_data
            .insert("cjk".into(), FontData::from_owned(c).into());
        f.families
            .entry(FontFamily::Proportional)
            .or_default()
            .push("cjk".into());
    }

    // bold family: bold face first, then the whole proportional stack as fallback
    let mut bold_stack: Vec<String> = Vec::new();
    if let Some(b) = heavy {
        f.font_data
            .insert("heavy".into(), FontData::from_owned(b).into());
        bold_stack.push("heavy".into());
    }
    bold_stack.extend(
        f.families
            .get(&FontFamily::Proportional)
            .cloned()
            .unwrap_or_default(),
    );
    f.families.insert(FontFamily::Name(BOLD.into()), bold_stack);

    // monospace (player Info box): Thai / CJK fall back to the body fonts instead of showing boxes
    let mono = f.families.entry(FontFamily::Monospace).or_default();
    for k in ["body", "cjk"] {
        if f.font_data.contains_key(k) && !mono.iter().any(|x| x == k) {
            mono.push(k.into());
        }
    }

    // icons (inserted as 2nd fallback of the proportional stack)
    egui_phosphor::add_to_fonts(&mut f, egui_phosphor::Variant::Regular);
    if let Some(stack) = f.families.get_mut(&FontFamily::Name(BOLD.into())) {
        stack.insert(stack.len().min(2), "phosphor".into());
    }
    ctx.set_fonts(f);
}
