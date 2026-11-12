//! Source health page: is each website up, how fast are its lists / search / episode pages / links / playback,
//! how often do they fail and with what. Numbers come from `waku_core::srcstats` (every real use is counted)
//! plus "Test" runs on demand.

use crate::app::WakuApp;
use crate::i18n::tr;
use crate::theme::*;
use crate::widgets;
use egui::{CornerRadius, Frame, Margin, RichText, Sense, Ui, pos2, vec2};
use egui_phosphor::regular as icon;
use waku_core::srcstats;

fn op_name(op: &str) -> &'static str {
    match op {
        "ping" => tr("Reachable", "เข้าถึงเว็บ"),
        "list" => tr("Latest list", "รายการล่าสุด"),
        "search" => tr("Search", "ค้นหา"),
        "detail" => tr("Series page", "หน้าเรื่อง"),
        "resolve" => tr("Find video link", "หาลิงก์วิดีโอ"),
        "play" => tr("Start playing", "เริ่มเล่น"),
        _ => "",
    }
}

fn fmt_ms(ms: u64) -> String {
    if ms >= 10_000 { format!("{:.0} s", ms as f64 / 1000.0) } else if ms >= 1000 { format!("{:.1} s", ms as f64 / 1000.0) } else { format!("{ms} ms") }
}

fn ago(at: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let d = now.saturating_sub(at);
    if d < 90 {
        tr("just now", "เมื่อกี้").into()
    } else if d < 3600 {
        let m = d / 60;
        crate::trf!("{m} min ago", "{m} นาทีที่แล้ว")
    } else if d < 86_400 {
        let h = d / 3600;
        crate::trf!("{h} h ago", "{h} ชม.ที่แล้ว")
    } else {
        let n = d / 86_400;
        crate::trf!("{n} days ago", "{n} วันที่แล้ว")
    }
}

/// Speed verdict for an operation's median time.
fn speed_color(op: &str, ms: u64) -> egui::Color32 {
    let (good, ok) = match op {
        "ping" => (800, 2500),
        "list" | "search" | "detail" => (2500, 8000),
        "resolve" => (4000, 12000),
        _ => (5000, 12000),
    };
    if ms <= good { GOOD } else if ms <= ok { WARN } else { BAD }
}

impl WakuApp {
    /// Run the checks the page offers for one source (or all): reachability, latest list, a search.
    pub fn test_source(&mut self, id: Option<String>) {
        let ids: Vec<String> = match id {
            Some(i) => vec![i],
            None => self.mgr.sources().iter().map(|s| s.id().to_string()).collect(),
        };
        for id in ids {
            if !self.src_testing.insert(id.clone()) {
                continue;
            }
            let (mgr, tx, ctx) = (self.mgr.clone(), self.tx.clone(), self.ctx.clone());
            std::thread::spawn(move || {
                if let Some(s) = mgr.get(&id) {
                    let r = mgr.http().ping(&format!("{}/", s.base()));
                    srcstats::record(&id, "ping", *r.as_ref().unwrap_or(&0), r.as_ref().map(|_| ()).map_err(|e| format!("{e:#}")));
                    let _ = mgr.latest(&id, 1);
                    let _ = mgr.search(&id, "one piece");
                }
                let _ = tx.send(crate::app::Msg::SourceTested(id));
                ctx.request_repaint();
            });
        }
    }

    pub fn sources_page(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Source health", "สถานะแหล่ง")).font(bold(28.0)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::ghost_button(ui, &format!("{}  {}", icon::TRASH, tr("Reset numbers", "ล้างสถิติ"))).clicked() {
                    srcstats::clear();
                }
                let busy = !self.src_testing.is_empty();
                let label = if busy { tr("Testing…", "กำลังทดสอบ…") } else { tr("Test all", "ทดสอบทั้งหมด") };
                if widgets::play_button(ui, label).clicked() && !busy {
                    self.test_source(None);
                    self.refresh_health();
                }
            });
        });
        ui.label(
            RichText::new(tr(
                "Counted from everything the app does (last 60 tries of each). Middle time of the successful tries; colours: green fast, yellow slow, red very slow.",
                "นับจากทุกครั้งที่แอปใช้งานจริง (60 ครั้งล่าสุดของแต่ละอย่าง) เวลาเป็นค่ากลางของครั้งที่สำเร็จ สี: เขียวเร็ว เหลืองช้า แดงช้ามาก",
            ))
            .size(12.5)
            .color(MUTED),
        );
        ui.add_space(14.0);

        let ids: Vec<(String, String)> = self.mgr.sources().iter().map(|s| (s.id().to_string(), s.base().to_string())).collect();
        let mut test = None;
        for (id, base) in ids {
            let name = self.source_name(&id);
            let h = self.health.get(&id).cloned();
            Frame::new().fill(BAR).corner_radius(CornerRadius::same(12)).inner_margin(Margin::same(16)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (col, text) = match &h {
                        Some(h) if h.ok => (GOOD, format!("{} · {}", tr("online", "ออนไลน์"), fmt_ms(h.latency_ms as u64))),
                        Some(_) => (BAD, tr("offline", "ออฟไลน์").to_string()),
                        None => (DIM, tr("not checked", "ยังไม่ได้ตรวจ").to_string()),
                    };
                    let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                    ui.painter().circle_filled(r.center(), 5.0, col);
                    ui.label(RichText::new(&name).font(bold(18.0)));
                    ui.label(RichText::new(text).size(13.0).color(col));
                    ui.label(RichText::new(&base).size(12.0).color(DIM));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.src_testing.contains(&id) {
                            ui.add(egui::Spinner::new().size(16.0).color(ACCENT));
                        } else if widgets::ghost_button(ui, &format!("{}  {}", icon::PLAY_CIRCLE, tr("Test", "ทดสอบ"))).clicked() {
                            test = Some(id.clone());
                        }
                    });
                });
                if let Some(e) = h.as_ref().and_then(|h| h.error.as_ref()) {
                    ui.add(egui::Label::new(RichText::new(crate::app::first_line(e)).size(12.0).color(BAD)).truncate()).on_hover_text(e.as_str());
                }
                ui.add_space(8.0);
                // one column per operation
                let ops = &srcstats::OPS[1..];
                let w = (ui.available_width() - 12.0 * (ops.len() as f32 - 1.0)) / ops.len() as f32;
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    for op in ops {
                        let s = srcstats::summary(&id, op);
                        let samples = srcstats::samples(&id, op);
                        ui.allocate_ui_with_layout(vec2(w, 120.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.set_width(w);
                            Frame::new().fill(PANEL).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(10)).show(ui, |ui| {
                                ui.set_width(w - 20.0);
                                ui.label(RichText::new(op_name(op)).size(12.5).color(MUTED));
                                match s.median_ms {
                                    Some(ms) => ui.label(RichText::new(fmt_ms(ms)).font(bold(20.0)).color(speed_color(op, ms))),
                                    None => ui.label(RichText::new("—").font(bold(20.0)).color(DIM)),
                                };
                                let rate = s.rate().map(|r| format!("{:.0}% {}", r * 100.0, tr("ok", "สำเร็จ"))).unwrap_or_default();
                                let last = s.last.as_ref().map(|l| ago(l.at)).unwrap_or_default();
                                ui.label(RichText::new(format!("{rate}  {} {last}", if s.tries > 0 { format!("· {}×", s.tries) } else { String::new() })).size(11.5).color(DIM));
                                // last 30 tries: bar height = time, red = failed
                                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
                                let tail: Vec<_> = samples.iter().rev().take(30).rev().collect();
                                if !tail.is_empty() {
                                    let max = tail.iter().map(|x| x.ms).max().unwrap_or(1).max(1) as f32;
                                    let bw = (r.width() / 30.0).max(2.0);
                                    for (i, x) in tail.iter().enumerate() {
                                        let hgt = if x.ok { (x.ms as f32 / max).clamp(0.12, 1.0) * r.height() } else { r.height() };
                                        let x0 = r.min.x + i as f32 * bw;
                                        let col = if x.ok { speed_color(op, x.ms).gamma_multiply(0.85) } else { BAD.gamma_multiply(0.6) };
                                        ui.painter().rect_filled(egui::Rect::from_min_max(pos2(x0, r.max.y - hgt), pos2(x0 + bw - 1.0, r.max.y)), CornerRadius::same(1), col);
                                    }
                                }
                                if let Some(e) = &s.last_err {
                                    ui.add(egui::Label::new(RichText::new(format!("{} {}: {}", icon::WARNING, ago(e.at), e.err.clone().unwrap_or_default())).size(11.0).color(BAD)).truncate())
                                        .on_hover_text(e.err.clone().unwrap_or_default());
                                }
                            });
                        });
                    }
                });
            });
            ui.add_space(12.0);
        }
        if let Some(id) = test {
            self.test_source(Some(id));
        }
    }
}
