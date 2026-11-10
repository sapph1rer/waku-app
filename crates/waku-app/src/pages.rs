//! Home, search, My List, history and settings pages.

use crate::app::{Page, WakuApp};
use crate::theme::*;
use crate::widgets::{self, anime_card, card_grid, card_row, chip, section_title};
use egui::{CornerRadius, Frame, Margin, RichText, Stroke, Ui};
use egui_phosphor::regular as icon;
use waku_core::AnimeCard;
use waku_core::text::truncate;

pub fn ago(when: u64) -> String {
    let d = waku_core::store::now().saturating_sub(when);
    match d {
        0..=59 => "เมื่อสักครู่".into(),
        60..=3599 => format!("{} นาทีที่แล้ว", d / 60),
        3600..=86_399 => format!("{} ชม. ที่แล้ว", d / 3600),
        _ => format!("{} วันที่แล้ว", d / 86_400),
    }
}

fn card_frame() -> Frame {
    Frame::new()
        .fill(BAR)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::same(18))
        .stroke(Stroke::new(1.0, LINE))
}

impl WakuApp {
    /// Source tabs with health dots. Returns the clicked source id.
    fn source_chips(&mut self, ui: &mut Ui, selected: &str) -> Option<String> {
        let mut out = None;
        let list: Vec<(String, String)> = self
            .mgr
            .sources()
            .iter()
            .map(|s| (s.id().to_string(), s.name().to_string()))
            .collect();
        ui.horizontal_wrapped(|ui| {
            for (id, name) in list {
                let dot = self.health_of(&id);
                if chip(ui, &name, id == selected, Some(dot)).clicked() {
                    out = Some(id);
                }
            }
        });
        out
    }

    fn grid_of(&mut self, ui: &mut Ui, cards: &[AnimeCard], show_source: bool) {
        let mut open = None;
        card_grid(ui, cards, |ui, c| {
            let sub = if show_source {
                self.source_name(&c.source)
            } else {
                String::new()
            };
            let referer = self.referer(&c.source);
            if anime_card(ui, &mut self.images, c, &referer, &sub).clicked() {
                open = Some(c.clone());
            }
        });
        if let Some(c) = open {
            self.open(&c);
        }
    }

    // ------------------------------------------------------------ home

    pub fn home_page(&mut self, ui: &mut Ui) {
        if self.mgr.sources().is_empty() {
            widgets::empty(
                ui,
                icon::BROADCAST,
                "ไม่มีแหล่งที่เปิดใช้ — ไปที่ตั้งค่าเพื่อแก้ sources.toml",
            );
            return;
        }

        if !self.lib.history.is_empty() {
            section_title(ui, "ดูต่อ");
            let hist: Vec<_> = self.lib.history.iter().take(20).cloned().collect();
            let mut open = None;
            card_row(ui, "continue", &hist, |ui, h| {
                let sub = format!("{} · {}", h.ep_label, self.source_name(&h.card.source));
                let referer = self.referer(&h.card.source);
                if anime_card(ui, &mut self.images, &h.card, &referer, &sub).clicked() {
                    open = Some(h.card.clone());
                }
            });
            if let Some(c) = open {
                self.open(&c);
            }
            ui.add_space(18.0);
        }

        ui.horizontal(|ui| {
            section_title(ui, "อัปเดตล่าสุด");
        });
        let current = self.home_source.clone();
        if let Some(id) = self.source_chips(ui, &current) {
            self.home_source = id.clone();
            self.settings.home_source = id;
            self.settings.save();
        }
        ui.add_space(10.0);

        let src = self.home_source.clone();
        self.ensure_latest(&src);
        let (cards, loading, error, end) = match self.listings.get(&src) {
            Some(l) => (l.cards.clone(), l.loading, l.error.clone(), l.end),
            None => (Vec::new(), true, None, false),
        };
        if let Some(e) = &error {
            widgets::error_box(
                ui,
                &format!(
                    "{} โหลดไม่ได้: {}",
                    self.source_name(&src),
                    crate::app::first_line(e)
                ),
            );
            ui.horizontal(|ui| {
                if widgets::outline_button(ui, &format!("{}  ลองใหม่", icon::ARROWS_CLOCKWISE))
                    .clicked()
                {
                    self.retry_latest(&src);
                }
                ui.label(RichText::new("หรือเลือกแหล่งอื่นด้านบน").color(MUTED));
            });
            return;
        }
        self.grid_of(ui, &cards, false);
        ui.add_space(14.0);
        if loading {
            widgets::loading(ui, "กำลังโหลด…");
        } else if !end && !cards.is_empty() {
            ui.vertical_centered(|ui| {
                if widgets::outline_button(ui, "โหลดเพิ่ม").clicked() {
                    self.more_latest(&src);
                }
            });
        }
    }

    // ------------------------------------------------------------ search

    pub fn search_page(&mut self, ui: &mut Ui) {
        if self.search_q.is_empty() {
            widgets::empty(
                ui,
                icon::MAGNIFYING_GLASS,
                "พิมพ์ชื่อเรื่องในช่องค้นหาด้านบน แล้วกด Enter",
            );
            return;
        }
        ui.label(RichText::new(format!("ผลการค้นหา “{}”", self.search_q)).font(bold(26.0)));
        ui.add_space(4.0);
        let pending = self.search.iter().filter(|x| x.1.is_none()).count();
        if pending > 0 {
            widgets::loading(ui, &format!("กำลังค้นใน {pending} แหล่ง…"));
        }

        let groups = self.search.clone();
        let mut misses: Vec<String> = Vec::new();
        for (id, res) in &groups {
            match res {
                Some(Ok(cards)) if !cards.is_empty() => {
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("●").color(self.health_of(id)));
                        ui.label(RichText::new(self.source_name(id)).font(bold(19.0)));
                        ui.label(RichText::new(format!("{} เรื่อง", cards.len())).color(MUTED));
                    });
                    ui.add_space(4.0);
                    self.grid_of(ui, cards, false);
                }
                Some(Ok(_)) => misses.push(format!("{} (ไม่พบ)", self.source_name(id))),
                Some(Err(e)) => misses.push(format!(
                    "{} (ผิดพลาด: {})",
                    self.source_name(id),
                    truncate(&crate::app::first_line(e), 70)
                )),
                None => {}
            }
        }
        if pending == 0
            && groups
                .iter()
                .all(|g| !matches!(&g.1, Some(Ok(c)) if !c.is_empty()))
        {
            widgets::empty(
                ui,
                icon::MAGNIFYING_GLASS,
                "ไม่พบเรื่องนี้ในทุกแหล่ง — ลองสะกดอีกแบบ / ชื่อภาษาอังกฤษ / โรมาจิ",
            );
        }
        if !misses.is_empty() {
            ui.add_space(16.0);
            for m in misses {
                ui.label(
                    RichText::new(format!("{}  {m}", icon::CIRCLE))
                        .size(12.5)
                        .color(DIM),
                );
            }
        }
    }

    // ------------------------------------------------------------ library

    pub fn my_list_page(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("รายการของฉัน").font(bold(26.0)));
        ui.add_space(10.0);
        if self.lib.my_list.is_empty() {
            widgets::empty(
                ui,
                icon::BOOKMARK_SIMPLE,
                "ยังไม่มีเรื่องในรายการ — กด “+ รายการของฉัน” ในหน้าเรื่อง",
            );
            return;
        }
        let cards = self.lib.my_list.clone();
        self.grid_of(ui, &cards, true);
    }

    pub fn history_page(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("ประวัติการดู").font(bold(26.0)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !self.lib.history.is_empty()
                    && widgets::ghost_button(ui, &format!("{}  ล้างประวัติ", icon::TRASH)).clicked()
                {
                    self.lib.history.clear();
                    self.lib.save();
                }
            });
        });
        ui.add_space(10.0);
        if self.lib.history.is_empty() {
            widgets::empty(ui, icon::CLOCK_COUNTER_CLOCKWISE, "ยังไม่มีประวัติการดู");
            return;
        }
        let hist = self.lib.history.clone();
        let mut open = None;
        card_grid(ui, &hist, |ui, h| {
            let sub = format!("{} · {}", h.ep_label, ago(h.when));
            let referer = self.referer(&h.card.source);
            if anime_card(ui, &mut self.images, &h.card, &referer, &sub).clicked() {
                open = Some(h.card.clone());
            }
        });
        if let Some(c) = open {
            self.open(&c);
        }
    }

    // ------------------------------------------------------------ settings

    pub fn settings_page(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("ตั้งค่า").font(bold(26.0)));
        ui.add_space(12.0);
        let mut changed = false;

        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("การเล่น").font(bold(18.0)));
            ui.add_space(6.0);
            egui::Grid::new("play_settings")
                .num_columns(2)
                .spacing([24.0, 12.0])
                .show(ui, |ui| {
                    ui.label("Player");
                    ui.horizontal(|ui| {
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut self.settings.player_cmd)
                                .desired_width(320.0)
                                .background_color(PANEL)
                                .hint_text("mpv หรือ C:\\mpv\\mpv.exe"),
                        );
                        if r.lost_focus() {
                            changed = true;
                            self.check_player();
                        }
                        match self.player_ok {
                            None => {
                                ui.add(egui::Spinner::new().size(14.0));
                            }
                            Some(true) => {
                                ui.label(
                                    RichText::new(format!("{} พร้อมใช้", icon::CHECK_CIRCLE))
                                        .color(GOOD),
                                );
                            }
                            Some(false) => {
                                ui.label(
                                    RichText::new(format!(
                                        "{} ไม่พบ — วาง mpv.exe ไว้ในโฟลเดอร์ mpv/ ข้างโปรแกรม",
                                        icon::WARNING
                                    ))
                                    .color(BAD),
                                );
                            }
                        }
                    });
                    ui.end_row();

                    ui.label("คุณภาพ");
                    egui::ComboBox::from_id_salt("quality")
                        .selected_text(self.settings.quality.clone())
                        .show_ui(ui, |ui| {
                            for q in ["best", "1080p", "720p", "480p", "360p", "worst"] {
                                if ui
                                    .selectable_value(&mut self.settings.quality, q.to_string(), q)
                                    .changed()
                                {
                                    changed = true;
                                }
                            }
                        });
                    ui.end_row();

                    ui.label("สลับแหล่งอัตโนมัติ");
                    if ui
                        .checkbox(
                            &mut self.settings.auto_failover,
                            "ถ้าทุกเซิร์ฟเวอร์ของแหล่งนี้เปิดไม่ได้ ให้หาตอนเดียวกันจากแหล่งอื่นแล้วเล่นต่อ",
                        )
                        .changed()
                    {
                        changed = true;
                    }
                    ui.end_row();

                    ui.label("ขนาด UI");
                    if ui
                        .add(
                            egui::Slider::new(&mut self.settings.ui_scale, 0.8..=1.6)
                                .step_by(0.05)
                                .fixed_decimals(2),
                        )
                        .changed()
                    {
                        self.ctx.set_zoom_factor(self.settings.ui_scale);
                        changed = true;
                    }
                    ui.end_row();
                });
        });
        if changed {
            self.settings.save();
        }

        ui.add_space(16.0);
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("แหล่งอนิเมะ").font(bold(18.0)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::ghost_button(ui, &format!("{}  โหลดแหล่งใหม่", icon::ARROWS_CLOCKWISE)).clicked() {
                        self.reload_sources();
                    }
                    if widgets::ghost_button(ui, &format!("{}  เปิด sources.toml", icon::FOLDER_OPEN)).clicked() {
                        open_path(&waku_core::profile::config_dir());
                    }
                    if widgets::ghost_button(ui, &format!("{}  ตรวจสถานะ", icon::BROADCAST)).clicked() {
                        self.refresh_health();
                    }
                });
            });
            ui.label(
                RichText::new("ลำดับการสลับแหล่ง = ลำดับในรายการนี้ (ลากไม่ได้ — กดลูกศรเพื่อเลื่อน). แก้โดเมน / selector ได้ในไฟล์ sources.toml")
                    .size(12.5)
                    .color(MUTED),
            );
            ui.add_space(8.0);

            let mut order: Vec<String> = self.settings.source_order.clone();
            for s in self.mgr.sources() {
                if !order.iter().any(|x| x == s.id()) {
                    order.push(s.id().to_string());
                }
            }
            order.retain(|id| self.mgr.get(id).is_some());
            let mut swap: Option<(usize, usize)> = None;
            let mut test: Option<String> = None;

            for (i, id) in order.iter().enumerate() {
                let Some(src) = self.mgr.get(id) else { continue };
                Frame::new().fill(PANEL).corner_radius(CornerRadius::same(4)).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{}", i + 1)).color(DIM).monospace());
                        ui.label(RichText::new("●").color(self.health_of(id)));
                        ui.label(RichText::new(src.name()).font(bold(15.0)));
                        ui.label(RichText::new(src.base()).size(12.5).color(MUTED));
                        let h = self.health.get(id.as_str());
                        let st = match h {
                            None if self.health_loading => "กำลังตรวจ…".to_string(),
                            None => "—".to_string(),
                            Some(h) if h.ok => format!("{} ms", h.latency_ms),
                            Some(h) => truncate(&crate::app::first_line(h.error.as_deref().unwrap_or("error")), 60),
                        };
                        ui.label(RichText::new(st).size(12.5).color(MUTED));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button(format!("{}  ทดสอบ", icon::FLASK)).on_hover_text("ล่าสุด → หน้าเรื่อง → player → สตรีม").clicked() {
                                test = Some(id.clone());
                            }
                            if i + 1 < order.len() && ui.small_button(icon::CARET_DOWN).clicked() {
                                swap = Some((i, i + 1));
                            }
                            if i > 0 && ui.small_button(icon::CARET_UP).clicked() {
                                swap = Some((i, i - 1));
                            }
                        });
                    });
                    if !src.note().is_empty() {
                        ui.label(RichText::new(src.note()).size(12.0).color(DIM));
                    }
                    if let Some(d) = self.diag.get(id.as_str()) {
                        ui.add_space(4.0);
                        match d {
                            None => widgets::loading(ui, "กำลังทดสอบ…"),
                            Some(steps) => {
                                for s in steps {
                                    let (g, c) = if s.ok { (icon::CHECK_CIRCLE, GOOD) } else { (icon::X_CIRCLE, BAD) };
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(RichText::new(g).color(c));
                                        ui.label(RichText::new(&s.name).strong());
                                        ui.label(RichText::new(&s.detail).color(MUTED));
                                    });
                                    if !s.trace.is_empty() {
                                        egui::CollapsingHeader::new(RichText::new("รายละเอียด").size(12.0).color(DIM))
                                            .id_salt((id, &s.name))
                                            .show(ui, |ui| {
                                                for t in &s.trace {
                                                    ui.label(RichText::new(t).monospace().size(11.5).color(MUTED));
                                                }
                                            });
                                    }
                                }
                            }
                        }
                    }
                });
                ui.add_space(4.0);
            }
            if let Some((a, b)) = swap {
                order.swap(a, b);
                self.settings.source_order = order;
                self.settings.save();
            } else if self.settings.source_order != order {
                self.settings.source_order = order;
            }
            if let Some(id) = test {
                self.run_diag(&id);
            }

            for p in self.mgr.profiles().iter().filter(|p| !p.enabled) {
                ui.label(RichText::new(format!("{}  {} — ปิดอยู่ (enabled = false)", icon::CIRCLE, p.name)).size(12.5).color(DIM));
            }
        });

        ui.add_space(16.0);
        ui.label(
            RichText::new(format!(
                "ข้อมูลเก็บที่ {}  ·  ประวัติ {} เรื่อง  ·  รายการของฉัน {} เรื่อง",
                waku_core::profile::config_dir().display(),
                self.lib.history.len(),
                self.lib.my_list.len()
            ))
            .size(12.0)
            .color(DIM),
        );
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.go(Page::Home);
        }
    }
}

pub fn open_path(p: &std::path::Path) {
    let _ = std::fs::create_dir_all(p);
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(p).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(p).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(p).spawn();
}
