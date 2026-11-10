//! Home, Browse, Search, My List, History and Settings.

use crate::app::{Page, WakuApp};
use crate::i18n::{self, Lang, tr};
use crate::theme::*;
use crate::trf;
use crate::widgets::{self, anime_card, backdrop, card_grid, card_row, chip, section_title};
use egui::{CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui, pos2, vec2};
use egui_phosphor::regular as icon;
use waku_core::AnimeCard;
use waku_core::text::truncate;

const PAD: i8 = 36;

fn card_frame() -> Frame {
    Frame::new()
        .fill(BAR)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(18))
        .stroke(Stroke::new(1.0, LINE))
}

impl WakuApp {
    /// Source pills with health dots. Returns the clicked source id.
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

    fn row_of(&mut self, ui: &mut Ui, id: &str, cards: &[AnimeCard], show_source: bool) {
        let mut open = None;
        card_row(ui, id, cards, |ui, c| {
            let sub = if show_source {
                self.source_name(&c.source)
            } else {
                c.badge.clone().unwrap_or_default()
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
            Frame::new().inner_margin(Margin::same(PAD)).show(ui, |ui| {
                widgets::empty(
                    ui,
                    icon::BROADCAST,
                    tr(
                        "No sources enabled — edit sources.toml in Settings",
                        "ไม่มีแหล่งที่เปิดใช้ — แก้ sources.toml ในหน้าตั้งค่า",
                    ),
                );
            });
            return;
        }
        let home = self.home_source.clone();
        self.ensure_latest(&home);
        let featured: Vec<AnimeCard> = self
            .listings
            .get(&home)
            .map(|l| l.cards.iter().take(6).cloned().collect())
            .unwrap_or_default();

        // ---------------------------------------------------------- hero
        let w = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(w, 470.0), Sense::hover());
        if featured.is_empty() {
            ui.painter().rect_filled(rect, CornerRadius::ZERO, BAR);
            let loading = self.listings.get(&home).is_none_or(|l| l.loading);
            let msg = if loading {
                tr("Loading…", "กำลังโหลด…").to_string()
            } else {
                self.listings
                    .get(&home)
                    .and_then(|l| l.error.clone())
                    .map(|e| crate::app::first_line(&e))
                    .unwrap_or_default()
            };
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                msg,
                regular(15.0),
                MUTED,
            );
        } else {
            let t = ui.input(|i| i.time);
            let idx = ((t / 9.0) as usize) % featured.len();
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs(1));
            let f = featured[idx].clone();
            let referer = self.referer(&f.source);
            backdrop(ui, &mut self.images, f.poster.as_deref(), &referer, rect);
            let inner = rect.shrink2(vec2(PAD as f32, 0.0));
            let mut act_open = false;
            let mut act_list = false;
            let mut hero = ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_min_max(
                pos2(inner.min.x, inner.min.y + 150.0),
                pos2(inner.min.x + (w * 0.5).max(420.0), inner.max.y),
            )));
            let sn = self.source_name(&f.source).to_uppercase();
            hero.label(
                RichText::new(trf!("FEATURED ON {sn}", "แนะนำจาก {sn}"))
                    .size(12.0)
                    .color(ACCENT_HI)
                    .strong(),
            );
            hero.add_space(4.0);
            widgets::clamp_label(&mut hero, &f.title, bold(36.0), TEXT, 2);
            hero.add_space(8.0);
            if let Some(b) = &f.badge {
                hero.horizontal(|ui| widgets::badge(ui, b, true));
                hero.add_space(8.0);
            }
            hero.add_space(10.0);
            hero.horizontal(|ui| {
                if widgets::primary_button(
                    ui,
                    &format!("{}  {}", icon::PLAY, tr("Start Watching", "เริ่มดู")),
                )
                .clicked()
                {
                    act_open = true;
                }
                let in_list = self.lib.in_list(&f.url);
                let label = if in_list {
                    format!("{}  {}", icon::CHECK, tr("In My List", "อยู่ในรายการ"))
                } else {
                    format!("{}  {}", icon::PLUS, tr("My List", "รายการของฉัน"))
                };
                if widgets::secondary_button(ui, &label).clicked() {
                    act_list = true;
                }
            });
            // carousel dots
            let dots_y = rect.max.y - 26.0;
            for i in 0..featured.len() {
                let c = pos2(inner.min.x + 8.0 + i as f32 * 22.0, dots_y);
                let r =
                    egui::Rect::from_center_size(c, vec2(if i == idx { 18.0 } else { 8.0 }, 6.0));
                ui.painter().rect_filled(
                    r,
                    CornerRadius::same(3),
                    if i == idx { ACCENT } else { DIM },
                );
            }
            if act_open {
                self.open(&f);
            }
            if act_list {
                self.lib.toggle_list(&f);
            }
        }

        // ---------------------------------------------------------- rows
        Frame::new()
            .inner_margin(Margin {
                left: PAD,
                right: PAD,
                top: 6,
                bottom: 0,
            })
            .show(ui, |ui| {
                if !self.lib.history.is_empty() {
                    section_title(ui, tr("Continue Watching", "ดูต่อ"));
                    let hist: Vec<_> = self.lib.history.iter().take(20).cloned().collect();
                    let mut open = None;
                    card_row(ui, "continue", &hist, |ui, h| {
                        let sub = format!(
                            "{} · {}",
                            i18n::ep_name_num(h.ep_num, &h.ep_label),
                            self.source_name(&h.card.source)
                        );
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

                let ids: Vec<String> = self
                    .mgr
                    .sources()
                    .iter()
                    .map(|s| s.id().to_string())
                    .collect();
                let mut ordered: Vec<String> = vec![home.clone()];
                ordered.extend(ids.into_iter().filter(|i| *i != home));
                for id in ordered {
                    self.ensure_latest(&id);
                    let (cards, loading, err) = match self.listings.get(&id) {
                        Some(l) => (l.cards.clone(), l.loading, l.error.is_some()),
                        None => (Vec::new(), true, false),
                    };
                    if err {
                        continue; // broken sources stay out of the home page (see Browse / Settings)
                    }
                    let name = self.source_name(&id);
                    let mut see_all = false;
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(trf!("New on {name}", "ใหม่ใน {name}")).font(bold(21.0)),
                        );
                        ui.label(RichText::new("●").color(self.health_of(&id)));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::ghost_button(
                                ui,
                                &format!("{}  {}", tr("See all", "ดูทั้งหมด"), icon::CARET_RIGHT),
                            )
                            .clicked()
                            {
                                see_all = true;
                            }
                        });
                    });
                    ui.add_space(4.0);
                    if loading && cards.is_empty() {
                        widgets::loading(ui, tr("Loading…", "กำลังโหลด…"));
                    } else {
                        self.row_of(
                            ui,
                            &format!("row-{id}"),
                            &cards.into_iter().take(20).collect::<Vec<_>>(),
                            false,
                        );
                    }
                    ui.add_space(18.0);
                    if see_all {
                        self.home_source = id.clone();
                        self.go(Page::Browse);
                    }
                }
            });
    }

    // ------------------------------------------------------------ browse

    pub fn browse_page(&mut self, ui: &mut Ui) {
        ui.label(RichText::new(tr("Browse", "เรียกดู")).font(bold(28.0)));
        ui.add_space(8.0);
        let current = self.home_source.clone();
        if let Some(id) = self.source_chips(ui, &current) {
            self.home_source = id.clone();
            self.settings.home_source = id;
            self.settings.save();
        }
        ui.add_space(14.0);

        let src = self.home_source.clone();
        self.ensure_latest(&src);
        let (cards, loading, error, end) = match self.listings.get(&src) {
            Some(l) => (l.cards.clone(), l.loading, l.error.clone(), l.end),
            None => (Vec::new(), true, None, false),
        };
        if let Some(e) = &error {
            let name = self.source_name(&src);
            let err = crate::app::first_line(e);
            widgets::error_box(
                ui,
                &trf!(
                    "{name} could not be loaded: {err}",
                    "{name} โหลดไม่ได้: {err}"
                ),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if widgets::outline_button(
                    ui,
                    &format!("{}  {}", icon::ARROWS_CLOCKWISE, tr("Retry", "ลองใหม่")),
                )
                .clicked()
                {
                    self.retry_latest(&src);
                }
                ui.label(
                    RichText::new(tr("or pick another source above", "หรือเลือกแหล่งอื่นด้านบน"))
                        .color(MUTED),
                );
            });
            return;
        }
        self.grid_of(ui, &cards, false);
        ui.add_space(14.0);
        if loading {
            widgets::loading(ui, tr("Loading…", "กำลังโหลด…"));
        } else if !end && !cards.is_empty() {
            ui.vertical_centered(|ui| {
                if widgets::outline_button(ui, tr("Load more", "โหลดเพิ่ม")).clicked()
                {
                    self.more_latest(&src);
                }
            });
        }
    }

    // ------------------------------------------------------------ search

    pub fn search_page(&mut self, ui: &mut Ui) {
        let te = egui::TextEdit::singleline(&mut self.search_box)
            .hint_text(format!(
                "{}  {}",
                icon::MAGNIFYING_GLASS,
                tr("Search anime on every source…", "ค้นหาอนิเมะจากทุกแหล่ง…")
            ))
            .font(regular(20.0))
            .background_color(PANEL)
            .margin(Margin::symmetric(16, 12))
            .desired_width(f32::INFINITY);
        let r = ui.add(te);
        if self.search_focus {
            r.request_focus();
            self.search_focus = false;
        }
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let q = self.search_box.clone();
            self.run_search(&q);
        }
        ui.add_space(18.0);

        if self.search_q.is_empty() {
            widgets::empty(
                ui,
                icon::MAGNIFYING_GLASS,
                tr(
                    "Type a title and press Enter — English, Romaji or Thai",
                    "พิมพ์ชื่อเรื่องแล้วกด Enter — อังกฤษ / โรมาจิ / ไทย",
                ),
            );
            return;
        }
        let q = self.search_q.clone();
        ui.label(RichText::new(trf!("Results for “{q}”", "ผลการค้นหา “{q}”")).font(bold(24.0)));
        let pending = self.search.iter().filter(|x| x.1.is_none()).count();
        if pending > 0 {
            widgets::loading(
                ui,
                &trf!("Searching {pending} source(s)…", "กำลังค้นใน {pending} แหล่ง…"),
            );
        }

        let groups = self.search.clone();
        let mut misses: Vec<String> = Vec::new();
        for (id, res) in &groups {
            let name = self.source_name(id);
            match res {
                Some(Ok(cards)) if !cards.is_empty() => {
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("●").color(self.health_of(id)));
                        ui.label(RichText::new(&name).font(bold(19.0)));
                        let n = cards.len();
                        ui.label(RichText::new(trf!("{n} titles", "{n} เรื่อง")).color(MUTED));
                    });
                    ui.add_space(4.0);
                    self.grid_of(ui, cards, false);
                }
                Some(Ok(_)) => misses.push(trf!("{name} (no match)", "{name} (ไม่พบ)")),
                Some(Err(e)) => {
                    let err = truncate(&crate::app::first_line(e), 70);
                    misses.push(trf!("{name} (error: {err})", "{name} (ผิดพลาด: {err})"))
                }
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
                tr(
                    "Not found on any source — try another spelling",
                    "ไม่พบในทุกแหล่ง — ลองสะกดแบบอื่น",
                ),
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
        ui.label(RichText::new(tr("My List", "รายการของฉัน")).font(bold(28.0)));
        ui.add_space(12.0);
        if self.lib.my_list.is_empty() {
            widgets::empty(
                ui,
                icon::BOOKMARK_SIMPLE,
                tr(
                    "Nothing here yet — press “+ My List” on a title",
                    "ยังไม่มีเรื่องในรายการ — กด “+ รายการของฉัน” ในหน้าเรื่อง",
                ),
            );
            return;
        }
        let cards = self.lib.my_list.clone();
        self.grid_of(ui, &cards, true);
    }

    pub fn history_page(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("History", "ประวัติการดู")).font(bold(28.0)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !self.lib.history.is_empty()
                    && widgets::ghost_button(
                        ui,
                        &format!("{}  {}", icon::TRASH, tr("Clear history", "ล้างประวัติ")),
                    )
                    .clicked()
                {
                    self.lib.history.clear();
                    self.lib.save();
                }
            });
        });
        ui.add_space(12.0);
        if self.lib.history.is_empty() {
            widgets::empty(
                ui,
                icon::CLOCK_COUNTER_CLOCKWISE,
                tr("No watch history yet", "ยังไม่มีประวัติการดู"),
            );
            return;
        }
        let hist = self.lib.history.clone();
        let mut open = None;
        card_grid(ui, &hist, |ui, h| {
            let sub = format!(
                "{} · {}",
                i18n::ep_name_num(h.ep_num, &h.ep_label),
                i18n::ago(h.when)
            );
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
        ui.label(RichText::new(tr("Settings", "ตั้งค่า")).font(bold(28.0)));
        ui.add_space(14.0);
        let mut changed = false;

        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(tr("General", "ทั่วไป")).font(bold(18.0)));
            ui.add_space(6.0);
            egui::Grid::new("general_settings")
                .num_columns(2)
                .spacing([28.0, 12.0])
                .show(ui, |ui| {
                    ui.label(format!("{}  {}", icon::TRANSLATE, tr("Language", "ภาษา")));
                    let cur = i18n::lang();
                    let mut pick = cur;
                    egui::ComboBox::from_id_salt("language")
                        .selected_text(cur.native())
                        .show_ui(ui, |ui| {
                            for l in Lang::ALL {
                                ui.selectable_value(&mut pick, l, l.native());
                            }
                        });
                    if pick != cur {
                        self.set_language(pick);
                    }
                    ui.end_row();

                    ui.label(tr("UI scale", "ขนาด UI"));
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
        ui.add_space(14.0);

        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(tr("Playback", "การเล่น")).font(bold(18.0)));
            ui.add_space(6.0);
            egui::Grid::new("play_settings").num_columns(2).spacing([28.0, 12.0]).show(ui, |ui| {
                ui.label(tr("Player", "Player"));
                ui.horizontal(|ui| {
                    let r = ui.add(egui::TextEdit::singleline(&mut self.settings.player_cmd).desired_width(320.0).background_color(PANEL).hint_text("mpv  /  C:\\mpv\\mpv.exe"));
                    if r.lost_focus() {
                        changed = true;
                        self.check_player();
                    }
                    match self.player_ok {
                        None => {
                            ui.add(egui::Spinner::new().size(14.0));
                        }
                        Some(true) => {
                            ui.label(RichText::new(format!("{} {}", icon::CHECK_CIRCLE, tr("Ready", "พร้อมใช้"))).color(GOOD));
                        }
                        Some(false) => {
                            ui.label(
                                RichText::new(format!("{} {}", icon::WARNING, tr("Not found — put mpv.exe in an mpv\\ folder next to the app", "ไม่พบ — วาง mpv.exe ไว้ในโฟลเดอร์ mpv\\ ข้างโปรแกรม")))
                                    .color(BAD),
                            );
                        }
                    }
                });
                ui.end_row();

                ui.label(tr("Quality", "คุณภาพ"));
                egui::ComboBox::from_id_salt("quality").selected_text(self.settings.quality.clone()).show_ui(ui, |ui| {
                    for q in ["best", "1080p", "720p", "480p", "360p", "worst"] {
                        if ui.selectable_value(&mut self.settings.quality, q.to_string(), q).changed() {
                            changed = true;
                        }
                    }
                });
                ui.end_row();

                ui.label(tr("Auto switch source", "สลับแหล่งอัตโนมัติ"));
                if ui
                    .checkbox(
                        &mut self.settings.auto_failover,
                        tr("If every server of a source fails, play the same episode from another source", "ถ้าทุกเซิร์ฟเวอร์ของแหล่งนี้เปิดไม่ได้ ให้หาตอนเดียวกันจากแหล่งอื่นแล้วเล่นต่อ"),
                    )
                    .changed()
                {
                    changed = true;
                }
                ui.end_row();
            });
        });
        if changed {
            self.settings.save();
        }

        ui.add_space(14.0);
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr("Sources", "แหล่งอนิเมะ")).font(bold(18.0)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::ghost_button(ui, &format!("{}  {}", icon::ARROWS_CLOCKWISE, tr("Reload sources", "โหลดแหล่งใหม่"))).clicked() {
                        self.reload_sources();
                    }
                    if widgets::ghost_button(ui, &format!("{}  {}", icon::FOLDER_OPEN, tr("Open sources.toml", "เปิด sources.toml"))).clicked() {
                        open_path(&waku_core::profile::config_dir());
                    }
                    if widgets::ghost_button(ui, &format!("{}  {}", icon::BROADCAST, tr("Check status", "ตรวจสถานะ"))).clicked() {
                        self.refresh_health();
                    }
                });
            });
            ui.label(
                RichText::new(tr(
                    "Failover order = order of this list (use the arrows). Domains and selectors are edited in sources.toml.",
                    "ลำดับการสลับแหล่ง = ลำดับในรายการนี้ (กดลูกศรเพื่อเลื่อน) แก้โดเมน / selector ได้ในไฟล์ sources.toml",
                ))
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
                Frame::new().fill(PANEL).corner_radius(CornerRadius::same(6)).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{}", i + 1)).color(DIM).monospace());
                        ui.label(RichText::new("●").color(self.health_of(id)));
                        ui.label(RichText::new(src.name()).font(bold(15.0)));
                        ui.label(RichText::new(src.base()).size(12.5).color(MUTED));
                        let st = match self.health.get(id.as_str()) {
                            None if self.health_loading => tr("checking…", "กำลังตรวจ…").to_string(),
                            None => "—".to_string(),
                            Some(h) if h.ok => format!("{} ms", h.latency_ms),
                            Some(h) => truncate(&crate::app::first_line(h.error.as_deref().unwrap_or("error")), 60),
                        };
                        ui.label(RichText::new(st).size(12.5).color(MUTED));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .small_button(format!("{}  {}", icon::FLASK, tr("Test", "ทดสอบ")))
                                .on_hover_text(tr("latest → title page → player → stream", "ล่าสุด → หน้าเรื่อง → player → สตรีม"))
                                .clicked()
                            {
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
                            None => widgets::loading(ui, tr("Testing…", "กำลังทดสอบ…")),
                            Some(steps) => {
                                for s in steps {
                                    let (g, c) = if s.ok { (icon::CHECK_CIRCLE, GOOD) } else { (icon::X_CIRCLE, BAD) };
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(RichText::new(g).color(c));
                                        ui.label(RichText::new(i18n::step(&s.name)).strong());
                                        ui.label(RichText::new(&s.detail).color(MUTED));
                                    });
                                    if !s.trace.is_empty() {
                                        egui::CollapsingHeader::new(RichText::new(tr("details", "รายละเอียด")).size(12.0).color(DIM)).id_salt((id, &s.name)).show(ui, |ui| {
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
                let n = &p.name;
                ui.label(RichText::new(format!("{}  {}", icon::CIRCLE, trf!("{n} — disabled (enabled = false)", "{n} — ปิดอยู่ (enabled = false)"))).size(12.5).color(DIM));
            }
        });

        ui.add_space(16.0);
        let dir = waku_core::profile::config_dir().display().to_string();
        let (h, l) = (self.lib.history.len(), self.lib.my_list.len());
        ui.label(
            RichText::new(trf!(
                "Data folder: {dir}  ·  history {h}  ·  my list {l}",
                "ข้อมูลเก็บที่ {dir}  ·  ประวัติ {h} เรื่อง  ·  รายการของฉัน {l} เรื่อง"
            ))
            .size(12.0)
            .color(DIM),
        );
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
