//! Minimal UI translation: English (default) and Thai.
//!
//! Strings live next to where they are used: `tr("Home", "หน้าแรก")`, and for
//! formatted text `trf!("Episode {n}", "ตอนที่ {n}")` (both templates capture the
//! same variables). Adding a language = one more variant + one more argument.

use std::sync::atomic::{AtomicU8, Ordering};
use waku_core::{Episode, text::fmt_num};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En = 0,
    Th = 1,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::En, Lang::Th];

    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Th => "th",
        }
    }
    /// Name shown in the language picker (always in its own language).
    pub fn native(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Th => "ไทย",
        }
    }
    pub fn from_code(c: &str) -> Lang {
        match c {
            "th" => Lang::Th,
            _ => Lang::En,
        }
    }
}

static LANG: AtomicU8 = AtomicU8::new(0);

pub fn set(l: Lang) {
    LANG.store(l as u8, Ordering::Relaxed);
}

pub fn lang() -> Lang {
    if LANG.load(Ordering::Relaxed) == 1 {
        Lang::Th
    } else {
        Lang::En
    }
}

pub fn is_th() -> bool {
    lang() == Lang::Th
}

pub fn tr<'a>(en: &'a str, th: &'a str) -> &'a str {
    if is_th() { th } else { en }
}

#[macro_export]
macro_rules! trf {
    ($en:literal, $th:literal) => {
        if $crate::i18n::is_th() {
            format!($th)
        } else {
            format!($en)
        }
    };
}

/// Tags the engine reports in English, shown in the UI language.
pub fn tag(t: &str) -> String {
    match (t, is_th()) {
        ("Sub", true) => "ซับไทย".into(),
        ("Dub", true) => "พากย์ไทย".into(),
        ("Ongoing", true) => "ยังไม่จบ".into(),
        ("Completed", true) => "จบแล้ว".into(),
        _ => t.to_string(),
    }
}

/// "Episode 3" / "ตอนที่ 3" (+ dub marker) built from the number, not the site label.
pub fn ep_name(e: &Episode) -> String {
    ep_name_num(e.num, &e.label)
}

pub fn ep_name_num(num: f32, raw_label: &str) -> String {
    let n = fmt_num(num);
    let dub = raw_label.contains("(Dub)") || raw_label.contains("พากย์");
    let mut s = trf!("Episode {n}", "ตอนที่ {n}");
    if dub {
        s.push_str(tr(" (Dub)", " (พากย์)"));
    }
    s
}

/// Short form used on buttons: "S2 · E5" / "S2 · ตอน 5".
pub fn ep_short(e: &Episode) -> String {
    let (s, n) = (e.season, fmt_num(e.num));
    trf!("S{s} · E{n}", "ซีซั่น {s} · ตอน {n}")
}

/// Names of the source-tester steps (the engine reports English keys).
pub fn step(name: &str) -> String {
    if !is_th() {
        return name.to_string();
    }
    match name {
        "connect" => "เชื่อมต่อ".into(),
        "latest" => "หน้าล่าสุด".into(),
        "detail" => "หน้าเรื่อง".into(),
        "episodes" => "ตอน".into(),
        "player" => "player".into(),
        other => other.replacen("stream:", "สตรีม:", 1),
    }
}

pub fn ago(when: u64) -> String {
    let d = waku_core::store::now().saturating_sub(when);
    let (m, h, day) = (d / 60, d / 3600, d / 86_400);
    match d {
        0..=59 => tr("just now", "เมื่อสักครู่").into(),
        60..=3599 => trf!("{m} min ago", "{m} นาทีที่แล้ว"),
        3600..=86_399 => trf!("{h} h ago", "{h} ชม. ที่แล้ว"),
        _ => trf!("{day} d ago", "{day} วันที่แล้ว"),
    }
}
