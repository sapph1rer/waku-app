//! Home, Browse, Search, My List, History and Settings.

use crate::app::{Page, WakuApp};
use crate::i18n::{self, Lang, tr};
use crate::theme::*;
use crate::trf;
use crate::widgets::{self, backdrop, card_grid, card_row, chip, section_title};
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
            self.draw_card(ui, c, &sub, &mut open);
        });
        if let Some(a) = open {
            self.apply_card_act(a);
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
            self.draw_card(ui, c, &sub, &mut open);
        });
        if let Some(a) = open {
            self.apply_card_act(a);
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
        // featured: the home source's newest titles; when that site is down, the next source that answers
        let mut featured: Vec<AnimeCard> = self
            .listings
            .get(&home)
            .map(|l| l.cards.iter().filter(|c| c.poster.is_some()).take(6).cloned().collect())
            .unwrap_or_default();
        if featured.is_empty() && self.listings.get(&home).is_some_and(|l| !l.loading) {
            let order: Vec<String> = self.settings.source_order.clone();
            for id in order.iter().filter(|id| **id != home) {
                // (an 18+ site never stands in for the featured strip)
                if self.mgr.get(id).is_none() || self.mgr.is_adult(id) {
                    continue;
                }
                self.ensure_latest(id);
                if let Some(l) = self.listings.get(id)
                    && !l.cards.is_empty()
                {
                    featured = l.cards.iter().filter(|c| c.poster.is_some()).take(6).cloned().collect();
                    break;
                }
            }
        }

        // ---------------------------------------------------------- hero
        let w = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(w, 470.0), Sense::hover());
        if featured.is_empty() {
            widgets::shimmer(ui, rect, 0);
            let err = self.listings.get(&home).filter(|l| !l.loading).and_then(|l| l.error.clone());
            if let Some(e) = err {
                let mut c = ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_center_size(rect.center(), vec2(520.0, 160.0))));
                widgets::problem_card(&mut c, &crate::app::friendly_error(&e), &e);
            }
        } else {
            // carousel state: advances every 9 s, waits while the pointer is over it, cross-fades between slides
            const SLIDE_SECS: f64 = 9.0;
            let (t, dt) = ui.input(|i| (i.time, i.stable_dt as f64));
            let n = featured.len();
            if self.hero_idx >= n {
                (self.hero_idx, self.hero_prev) = (0, 0);
            }
            if self.hero_t0 <= 0.0 || self.hero_t0 > t {
                self.hero_t0 = t;
                self.hero_shown = t;
            }
            let hovered = ui.rect_contains_pointer(rect);
            if hovered {
                self.hero_t0 += dt.min(0.25); // the countdown stands still while the user is reading / aiming
            } else if t - self.hero_t0 >= SLIDE_SECS && n > 1 {
                self.hero_prev = self.hero_idx;
                self.hero_idx = (self.hero_idx + 1) % n;
                self.hero_t0 = t;
                self.hero_shown = t;
            }
            if self.hero_shown > t {
                self.hero_shown = 0.0;
            }
            let idx = self.hero_idx;
            let slide_t = (t - self.hero_t0).max(0.0) as f32;
            // the cross-fade has its own clock: the countdown above stands still under the pointer, the fade
            // must not (an arrow click would otherwise leave the new slide half-drawn until the mouse moved away)
            let k = ((t - self.hero_shown).max(0.0) as f32 / 0.6).clamp(0.0, 1.0);
            let fade_in = 1.0 - (1.0 - k).powi(3);
            if fade_in < 1.0 {
                ui.ctx().request_repaint();
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(if hovered { 100 } else { 250 }));
            // pictures of the slides either side are fetched ahead, so a click on an arrow shows the next one at once
            for j in [(idx + 1) % n, (idx + n - 1) % n] {
                let nf = featured[j].clone();
                let nref = self.referer(&nf.source);
                let nview = self.card_view(&nf);
                if let Some(u) = self.meta_of(&nf.url, &nf.title).and_then(|m| m.banner_url.clone()).or(nview.poster.clone()) {
                    let _ = self.images.get_blur(&u, &nref);
                    let _ = self.images.get(&u, &nref);
                }
            }
            let f = featured[idx].clone();
            let referer = self.referer(&f.source);
            let view = self.card_view(&f);
            let fm = self.meta_of(&f.url, &f.title).cloned();
            let art = fm
                .as_ref()
                .and_then(|m| m.banner_url.clone())
                .or(view.poster.clone());
            if fade_in < 1.0 && self.hero_prev != idx && self.hero_prev < n {
                // the slide before stays underneath while the new one fades in over it
                let pf = featured[self.hero_prev].clone();
                let pref = self.referer(&pf.source);
                let pview = self.card_view(&pf);
                let part = self.meta_of(&pf.url, &pf.title).and_then(|m| m.banner_url.clone()).or(pview.poster.clone());
                backdrop(ui, &mut self.images, part.as_deref(), &pref, rect);
                let mut layer = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                layer.multiply_opacity(fade_in);
                backdrop(&layer, &mut self.images, art.as_deref(), &referer, rect);
            } else {
                backdrop(ui, &mut self.images, art.as_deref(), &referer, rect);
            }
            let inner = rect.shrink2(vec2(PAD as f32, 0.0));
            let mut act_open = false;
            let mut act_list = false;
            // the text slides in a little and fades with the picture
            let slide_x = 18.0 * (1.0 - fade_in);
            // the text block sits low in the banner (laid out from the bottom: its height is known from last frame)
            let top = (rect.max.y - 58.0 - self.hero_text_h).max(rect.min.y + 70.0);
            let mut hero = ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_min_max(
                pos2(inner.min.x + slide_x, top),
                pos2(inner.min.x + slide_x + (w * 0.5).max(420.0), inner.max.y),
            )));
            hero.multiply_opacity(fade_in);
            let sn = self.source_name(&f.source).to_uppercase();
            hero.label(
                RichText::new(trf!("FEATURED ON {sn}", "แนะนำจาก {sn}"))
                    .size(12.0)
                    .color(ACCENT_HI)
                    .strong(),
            );
            hero.add_space(4.0);
            widgets::clamp_label(&mut hero, &view.title, bold(36.0), TEXT, 2);
            hero.add_space(8.0);
            hero.horizontal(|ui| {
                if let Some(b) = &f.badge {
                    widgets::badge(ui, b, true);
                }
                if let Some(m) = &fm {
                    let line = i18n::meta_line(m);
                    if !line.is_empty() {
                        widgets::badge(ui, &line, f.badge.is_none());
                    }
                    if let Some(sc) = m.score {
                        widgets::badge(ui, &format!("{} {sc}%", icon::STAR), false);
                    }
                }
            });
            hero.add_space(8.0);
            if let Some(desc) = fm.as_ref().and_then(|m| m.description.clone()) {
                widgets::clamp_label(
                    &mut hero,
                    &desc.replace("\n\n", " "),
                    regular(15.0),
                    egui::Color32::from_gray(220),
                    2,
                );
                hero.add_space(6.0);
            }
            hero.add_space(10.0);
            hero.horizontal(|ui| {
                if widgets::play_button(ui, tr("Start Watching", "เริ่มดู")).clicked() {
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
            let used = hero.min_rect().height();
            if (used - self.hero_text_h).abs() > 0.5 {
                self.hero_text_h = used;
                ui.ctx().request_repaint();
            }
            // carousel dots (click to jump) + previous / next
            let dots_y = rect.max.y - 16.0;
            let mut x = inner.min.x + 4.0;
            let mut jump: Option<usize> = None;
            for i in 0..n {
                let a = ui.ctx().animate_bool_with_time(egui::Id::new(("hero_dot", i)), i == idx, 0.3);
                let wdt = 8.0 + 26.0 * a;
                let r = egui::Rect::from_min_size(pos2(x, dots_y - 3.0), vec2(wdt, 6.0));
                let hit = ui.interact(r.expand2(vec2(4.0, 8.0)), egui::Id::new(("hero_dot_hit", i)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
                let hv = if hit.hovered() { TEXT } else { DIM };
                ui.painter().rect_filled(r, CornerRadius::same(3), widgets::lerp_color(hv, ACCENT.gamma_multiply(0.45), a));
                if a > 0.5 {
                    // how long the current slide has left
                    let pr = egui::Rect::from_min_size(r.min, vec2(wdt * (slide_t / SLIDE_SECS as f32).clamp(0.0, 1.0), 6.0));
                    ui.painter().rect_filled(pr, CornerRadius::same(3), ACCENT_HI);
                }
                if hit.clicked() {
                    jump = Some(i);
                }
                x += wdt + 8.0;
            }
            if n > 1 {
                let show = ui.ctx().animate_bool_with_time(egui::Id::new("hero_arrows"), hovered, 0.2);
                if show > 0.0 {
                    for (j, (glyph, step)) in [(icon::CARET_LEFT, n - 1), (icon::CARET_RIGHT, 1)].into_iter().enumerate() {
                        let c = pos2(rect.max.x - PAD as f32 - 62.0 + j as f32 * 46.0, rect.max.y - 34.0);
                        let r = egui::Rect::from_center_size(c, vec2(38.0, 38.0));
                        let hit = ui.interact(r, egui::Id::new(("hero_arrow", j)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
                        let hv = ui.ctx().animate_bool_with_time(egui::Id::new(("hero_arrow_h", j)), hit.hovered(), 0.12);
                        ui.painter().circle_filled(c, 19.0, widgets::lerp_color(egui::Color32::from_black_alpha(150), ACCENT, hv).gamma_multiply(show));
                        ui.painter().circle_stroke(c, 19.0, egui::Stroke::new(1.0, egui::Color32::from_white_alpha((70.0 * show) as u8)));
                        ui.painter().text(c, egui::Align2::CENTER_CENTER, glyph, regular(18.0), egui::Color32::WHITE.gamma_multiply(show));
                        if hit.clicked() {
                            jump = Some((idx + step) % n);
                        }
                    }
                }
            }
            if let Some(i) = jump.filter(|i| *i != idx) {
                self.hero_prev = idx;
                self.hero_idx = i;
                self.hero_t0 = t;
                self.hero_shown = t;
                ui.ctx().request_repaint(); // start the cross-fade now, not at the next mouse move
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
                let hist: Vec<_> = self.lib.history.iter().filter(|h| self.shown_source(&h.card.source)).take(20).cloned().collect();
                if !hist.is_empty() {
                    section_title(ui, tr("Continue Watching", "ดูต่อ"));
                    let mut open = None;
                    card_row(ui, "continue", &hist, |ui, h| {
                        let sub = format!(
                            "{} · {}",
                            i18n::ep_name_num(h.ep_num, if h.dub { "(Dub)" } else { &h.ep_label }),
                            self.source_name(&h.card.source)
                        );
                        self.draw_history_card(ui, &h.card, &h.ep_url, &sub, &mut open);
                    });
                    if let Some(a) = open {
                        self.apply_card_act(a);
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
                        ui.label(RichText::new(icon::CIRCLE).size(11.0).color(self.health_of(&id)));
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
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 18.0;
                            let n = ((ui.available_width() + 18.0) / (widgets_card_w() + 18.0)).floor().max(1.0) as usize;
                            for _ in 0..n {
                                widgets::card_skeleton(ui);
                            }
                        });
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
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Browse", "เรียกดู")).font(bold(28.0)));
            ui.add_space(18.0);
            if widgets::chip(ui, &format!("{}  {}", icon::BROADCAST, tr("Latest on the sources", "ล่าสุดจากแต่ละแหล่ง")), !self.browse_discover, None).clicked() {
                self.browse_discover = false;
            }
            if widgets::chip(ui, &format!("{}  {}", icon::FUNNEL, tr("Discover with filters", "ค้นหาแบบละเอียด")), self.browse_discover, None).clicked() {
                self.browse_discover = true;
            }
        });
        ui.add_space(12.0);
        if self.browse_discover {
            self.discover_page(ui);
            return;
        }
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
        if loading && cards.is_empty() {
            widgets::card_grid(ui, &[(); 12], |ui, _| widgets::card_skeleton(ui));
        } else if loading {
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
        let focus = std::mem::take(&mut self.search_focus);
        let bar = widgets::search_bar(ui, &mut self.search_box, tr("Search anime on every source…", "ค้นหาอนิเมะจากทุกแหล่ง…"), focus);
        let now = ui.input(|i| i.time);
        if bar.submitted {
            let q = self.search_box.clone();
            self.remember_search(&q);
            self.run_search(&q);
        } else if bar.changed {
            self.search_typed = Some(now);
        }
        // search as you type: half a second after the last key, from 3 letters on
        if let Some(t) = self.search_typed {
            if now - t >= 0.55 {
                self.search_typed = None;
                let q = self.search_box.trim().to_string();
                if q.chars().count() >= 3 {
                    self.run_search(&q);
                } else if q.is_empty() {
                    self.search_q.clear();
                    self.search.clear();
                }
            } else {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
            }
        }
        if bar.image {
            self.scene.open = true;
            ui.memory_mut(|m| m.stop_text_input()); // Ctrl+V goes to the dialog, not the search text
            if !self.scene.busy {
                self.scene.error = None;
            }
        }
        if let Some(j) = &self.scene.jump {
            let ep = j.episode.map(|e| format!("EP {}", waku_core::text::fmt_num(e))).unwrap_or_default();
            let (name, at) = (j.title_romaji.clone(), crate::scene::mmss(j.at));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!(
                        "{}  {}",
                        icon::IMAGE_SQUARE,
                        crate::trf!("Looking for {name} {ep} at {at} — open it below", "กำลังหา {name} {ep} นาที {at} — เปิดเรื่องจากผลด้านล่าง")
                    ))
                    .color(ACCENT_HI),
                );
                if widgets::ghost_button(ui, icon::X).clicked() {
                    self.scene.jump = None;
                }
            });
        }
        ui.add_space(18.0);

        if self.search_q.is_empty() {
            // shortcuts: what was searched before, and what is being watched
            let recent = self.settings.recent_searches.clone();
            let mut pick: Option<String> = None;
            let mut forget: Option<String> = None;
            if !recent.is_empty() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr("Recent searches", "ค้นหาล่าสุด")).font(bold(15.0)).color(MUTED));
                    if widgets::ghost_button(ui, tr("Clear", "ล้าง")).clicked() {
                        forget = Some(String::new());
                    }
                });
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                    for q in &recent {
                        if widgets::shortcut_chip(ui, icon::CLOCK_COUNTER_CLOCKWISE, q).clicked() {
                            pick = Some(q.clone());
                        }
                    }
                });
                ui.add_space(22.0);
            }
            // (the Latin part of a site title searches best on every source)
            let latin = |t: &str| -> String {
                let cut: String = t.chars().take_while(|c| c.is_ascii() || !c.is_alphabetic()).collect();
                let cut = cut.trim().trim_end_matches(['(', '-', ':']).trim().to_string();
                if cut.chars().count() >= 3 { cut } else { t.to_string() }
            };
            let watching: Vec<String> = self.lib.history.iter().take(8).map(|h| latin(&h.card.title)).collect();
            if !watching.is_empty() {
                ui.label(RichText::new(tr("Find another source for…", "หาแหล่งอื่นของเรื่อง…")).font(bold(15.0)).color(MUTED));
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                    for t in &watching {
                        if widgets::shortcut_chip(ui, icon::PLAY, t).clicked() {
                            pick = Some(t.clone());
                        }
                    }
                });
                ui.add_space(22.0);
            }
            if let Some(q) = pick {
                self.search_box = q.clone();
                self.remember_search(&q);
                self.run_search(&q);
            }
            if forget.is_some() {
                self.settings.recent_searches.clear();
                self.settings.save();
            }
            if recent.is_empty() && watching.is_empty() {
                widgets::empty(
                    ui,
                    icon::MAGNIFYING_GLASS,
                    tr("Type a title — English, Romaji or Thai. Results appear as you type.", "พิมพ์ชื่อเรื่อง — อังกฤษ / โรมาจิ / ไทย ผลลัพธ์จะขึ้นระหว่างพิมพ์"),
                );
            }
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
                        ui.label(RichText::new(icon::CIRCLE).size(11.0).color(self.health_of(id)));
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
        let cards: Vec<_> = self.lib.my_list.iter().filter(|c| self.shown_source(&c.source)).cloned().collect();
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
        let hist: Vec<_> = self.lib.history.iter().filter(|h| self.shown_source(&h.card.source)).cloned().collect();
        let mut open = None;
        card_grid(ui, &hist, |ui, h| {
            let sub = format!(
                "{} · {}",
                i18n::ep_name_num(h.ep_num, if h.dub { "(Dub)" } else { &h.ep_label }),
                i18n::ago(h.when)
            );
            self.draw_card(ui, &h.card, &sub, &mut open);
        });
        if let Some(a) = open {
            self.apply_card_act(a);
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

                    ui.label(tr("Titles", "ชื่อเรื่อง"));
                    let cur = self.settings.title_lang.clone();
                    let name = |k: &str| match k {
                        "romaji" => tr("Romaji (AniList)", "โรมาจิ (AniList)"),
                        "site" => tr("As on the site", "ตามเว็บ"),
                        _ => tr("English (AniList)", "อังกฤษ (AniList)"),
                    };
                    egui::ComboBox::from_id_salt("title_lang").selected_text(name(&cur)).show_ui(ui, |ui| {
                        for k in ["english", "romaji", "site"] {
                            if ui.selectable_value(&mut self.settings.title_lang, k.to_string(), name(k)).changed() {
                                changed = true;
                            }
                        }
                    });
                    ui.end_row();

                    ui.label(tr("Metadata", "ข้อมูลเรื่อง"));
                    if ui
                        .checkbox(&mut self.settings.fetch_meta, tr("Fetch covers, banners, synopses and episode titles (AniList · Kitsu · Jikan)", "ดึงปก ภาพพื้นหลัง เรื่องย่อ และชื่อตอน (AniList · Kitsu · Jikan)"))
                        .changed()
                    {
                        changed = true;
                    }
                    ui.end_row();

                    ui.label(format!("{}  Discord", icon::DISCORD_LOGO));
                    ui.vertical(|ui| {
                        if ui.checkbox(&mut self.settings.discord_presence, tr("Show what I'm watching on Discord (Rich Presence)", "แสดงเรื่องที่กำลังดูใน Discord (Rich Presence)")).changed() {
                            changed = true;
                        }
                        if self.settings.discord_presence {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(tr("Application ID", "Application ID")).color(MUTED));
                                if ui.add(egui::TextEdit::singleline(&mut self.settings.discord_app_id).hint_text("123456789012345678").desired_width(200.0)).changed() {
                                    changed = true;
                                }
                            });
                            ui.label(
                                RichText::new(tr(
                                    "discord.com/developers › New Application › name it \"Waku\" (that name shows as \"Watching Waku\") › copy the Application ID here. Discord must be running on this PC.",
                                    "discord.com/developers › New Application › ตั้งชื่อว่า \"Waku\" (ชื่อนี้จะขึ้นว่า \"Watching Waku\") › คัดลอก Application ID มาวางที่นี่ — ต้องเปิด Discord บนเครื่องนี้ไว้",
                                ))
                                .size(11.5)
                                .color(DIM),
                            );
                        }
                    });
                    ui.end_row();

                    ui.label(format!("{}  {}", icon::USERS_THREE, tr("Watch party", "ดูด้วยกัน")));
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(tr("My name", "ชื่อของฉัน")).color(MUTED));
                        if ui.add(egui::TextEdit::singleline(&mut self.settings.party_name).hint_text(tr("Viewer", "ผู้ชม")).desired_width(160.0)).changed() {
                            changed = true;
                        }
                    });
                    ui.end_row();

                    ui.label(format!("{}  {}", icon::HEARTBEAT, tr("Sources", "แหล่ง")));
                    if widgets::ghost_button(ui, &format!("{}  {}", icon::ARROW_RIGHT, tr("Source health page", "หน้าสถานะแหล่ง"))).clicked() {
                        self.go(crate::app::Page::Sources);
                    }
                    ui.end_row();

                    ui.label(tr("On Air", "ตารางฉาย"));
                    if ui.checkbox(&mut self.settings.show_adult, tr("Show 18+ titles", "แสดงเรื่อง 18+")).changed() {
                        changed = true;
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
            self.updates_section(ui);
        });
        ui.add_space(14.0);

        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(tr("Playback", "การเล่น")).font(bold(18.0)));
            ui.label(RichText::new(tr("Videos always play at the sharpest quality the site offers.", "วิดีโอจะเล่นที่ความคมชัดสูงสุดที่เว็บมีให้เสมอ")).size(12.5).color(MUTED));
            ui.add_space(6.0);
            egui::Grid::new("play_settings").num_columns(2).spacing([28.0, 12.0]).show(ui, |ui| {
                ui.label(tr("Play in", "เล่นด้วย"));
                ui.horizontal(|ui| {
                    for (k, en, th) in [("builtin", "Built-in player (rmp)", "ตัวเล่นในแอป (rmp)"), ("external", "External player", "โปรแกรมภายนอก")] {
                        if ui.selectable_label(self.settings.player == k, tr(en, th)).clicked() {
                            self.settings.player = k.to_string();
                            changed = true;
                        }
                    }
                });
                ui.end_row();

                if self.settings.player != "external" && rmp_egui::engine_problem().is_some() {
                    ui.label(tr("Player", "ตัวเล่น"));
                    ui.vertical(|ui| {
                        match rmp_egui::engine_problem() {
                        None => {
                            ui.label(RichText::new(format!("{} {}", icon::CHECK_CIRCLE, tr("libmpv found: GPU decoding, all codecs, styled Thai subtitles, 7.1 / passthrough audio", "พบ libmpv: ถอดรหัสด้วย GPU, รองรับทุก codec, ซับไทยแบบมีสไตล์, เสียง 7.1 / passthrough"))).size(12.5).color(GOOD));
                        }
                        Some(why) => {
                            ui.label(RichText::new(format!("{} {}", icon::WARNING, tr("libmpv is missing — playing with the basic Windows engine (fewer formats, some sources will not play).", "ไม่มี libmpv — ตอนนี้เล่นด้วยเอนจินพื้นฐานของ Windows (รองรับฟอร์แมตน้อยกว่า บางแหล่งจะเล่นไม่ได้)"))).size(12.5).color(WARN));
                            ui.horizontal(|ui| {
                                if self.libmpv_loading {
                                    ui.spinner();
                                    ui.label(RichText::new(tr("Downloading libmpv (about 30 MB)…", "กำลังดาวน์โหลด libmpv (ประมาณ 30 MB)…")).size(12.5).color(MUTED));
                                } else {
                                    if widgets::primary_button(ui, &format!("{}  {}", icon::DOWNLOAD_SIMPLE, tr("Download libmpv", "ดาวน์โหลด libmpv"))).clicked() {
                                        self.download_libmpv();
                                    }
                                    ui.label(RichText::new(tr("or put libmpv-2.dll next to the program yourself", "หรือวาง libmpv-2.dll ไว้ข้างโปรแกรมเอง")).size(12.0).color(DIM)).on_hover_text(why);
                                }
                            });
                        }
                    }
                    });
                    ui.end_row();
                }
                    ui.label(tr("Languages", "ภาษา"));
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(tr("Audio", "เสียง")).color(MUTED));
                            if ui.add(egui::TextEdit::singleline(&mut self.settings.audio_langs).desired_width(220.0).background_color(PANEL).hint_text("Japanese, Thai")).lost_focus() {
                                changed = true;
                            }
                            ui.label(RichText::new(tr("Subtitles", "ซับ")).color(MUTED));
                            if ui.add(egui::TextEdit::singleline(&mut self.settings.sub_langs).desired_width(220.0).background_color(PANEL).hint_text("Thai, English  /  off")).lost_focus() {
                                changed = true;
                            }
                        });
                    });
                    ui.end_row();

                    ui.label(tr("Episodes", "ตอน"));
                    ui.horizontal(|ui| {
                        changed |= ui.checkbox(&mut self.settings.autoplay_next, tr("Play the next episode automatically", "เล่นตอนถัดไปอัตโนมัติ")).changed();
                        changed |= ui.checkbox(&mut self.settings.auto_skip, tr("Skip openings and endings automatically (AniSkip)", "ข้ามเพลงเปิด/ปิดอัตโนมัติ (AniSkip)")).changed();
                        changed |= ui.checkbox(&mut self.settings.resume, tr("Resume where I stopped", "เล่นต่อจากที่ดูค้างไว้")).changed();
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
            ui.add_space(6.0);
            egui::CollapsingHeader::new(RichText::new(tr("Advanced", "ขั้นสูง")).color(MUTED)).id_salt("adv_play").show(ui, |ui| {
                egui::Grid::new("play_settings_adv").num_columns(2).spacing([28.0, 12.0]).show(ui, |ui| {
                    if self.settings.player != "external" {
                    ui.label(tr("Engine", "เอนจิน"));
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            for (k, en, th) in [("auto", "Automatic", "อัตโนมัติ"), ("mpv", "libmpv", "libmpv"), ("native", "Windows (Media Foundation)", "Windows (Media Foundation)")] {
                                if ui.selectable_label(self.settings.engine == k, tr(en, th)).clicked() {
                                    self.settings.engine = k.to_string();
                                    changed = true;
                                }
                            }
                        });
                    });
                    ui.end_row();

                    ui.label(tr("Hardware decoding", "ถอดรหัสด้วยฮาร์ดแวร์"));
                    egui::ComboBox::from_id_salt("hwdec").selected_text(self.settings.hwdec.clone()).show_ui(ui, |ui| {
                        for k in ["auto-safe", "auto", "d3d11va", "d3d11va-copy", "nvdec", "dxva2-copy", "no"] {
                            if ui.selectable_value(&mut self.settings.hwdec, k.to_string(), k).changed() {
                                changed = true;
                            }
                        }
                    });
                    ui.end_row();

                    ui.label(tr("Audio output", "เสียงออก"));
                    ui.vertical(|ui| {
                        let mut stereo = self.settings.audio_channels == "stereo";
                        if ui.checkbox(&mut stereo, tr("Mix surround (5.1 / 7.1) down to stereo — for headphones / TV speakers", "รวมเสียงรอบทิศ (5.1 / 7.1) เป็นสเตอริโอ — สำหรับหูฟัง / ลำโพงทีวี")).changed() {
                            self.settings.audio_channels = if stereo { "stereo".into() } else { "auto".into() };
                            changed = true;
                        }
                        let mut pt = !self.settings.audio_passthrough.is_empty();
                        if ui.checkbox(&mut pt, tr("Send Dolby / DTS / TrueHD untouched to an AV receiver (passthrough)", "ส่ง Dolby / DTS / TrueHD ไปที่ AV receiver โดยตรง (passthrough)")).changed() {
                            self.settings.audio_passthrough = if pt { "ac3,eac3,dts-hd,truehd".into() } else { String::new() };
                            changed = true;
                        }
                    });
                    ui.end_row();

                    }
                ui.label(tr("External player", "โปรแกรมภายนอก"));
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

                });
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
            self.adult_toggle(ui);
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
                        ui.label(RichText::new(icon::CIRCLE).size(11.0).color(self.health_of(id)));
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


fn widgets_card_w() -> f32 {
    crate::widgets::CARD_W
}

impl WakuApp {
    /// Settings › Sources: the 18+ (hentai) sources switch. Turning it on asks for the age first; either way the
    /// sources are loaded again (off = only the main ones connect at start).
    fn adult_toggle(&mut self, ui: &mut Ui) {
        let n = self.mgr.adult_count();
        let mut on = self.settings.adult_sources;
        ui.horizontal(|ui| {
            if widgets::switch(ui, &mut on).changed() {
                if on {
                    on = false;
                    self.adult_confirm = true; // asked below
                } else {
                    self.settings.adult_sources = false;
                    self.settings.save();
                    self.reload_sources();
                }
            }
            ui.vertical(|ui| {
                ui.label(RichText::new(crate::trf!("18+ sources (hentai) · {n}", "แหล่ง 18+ (hentai) · {n} แหล่ง")).strong());
                ui.label(
                    RichText::new(tr(
                        "Off: only the main sources connect when the app opens, and 18+ titles stay out of Continue Watching / My List / History.",
                        "ปิด: เปิดแอปจะเชื่อมต่อแค่แหล่งหลัก และเรื่อง 18+ จะไม่โผล่ใน ดูต่อ / รายการของฉัน / ประวัติ",
                    ))
                    .size(12.0)
                    .color(MUTED),
                );
            });
        });
        if self.adult_confirm {
            let mut close = false;
            let m = egui::Modal::new(egui::Id::new("adult-confirm"))
                .frame(Frame::new().fill(PANEL).stroke(egui::Stroke::new(1.0, LINE)).corner_radius(CornerRadius::same(16)).inner_margin(Margin::same(22)))
                .show(ui.ctx(), |ui| {
                    ui.set_width(420.0);
                    ui.label(RichText::new(format!("{}  {}", icon::WARNING, tr("Adult content", "เนื้อหาสำหรับผู้ใหญ่"))).font(bold(18.0)).color(WARN));
                    ui.add_space(8.0);
                    ui.label(tr(
                        "These sources show sexually explicit anime (hentai). Turn them on only if you are 18 or older and it is legal where you live.",
                        "แหล่งเหล่านี้มีอนิเมะที่มีเนื้อหาทางเพศโจ่งแจ้ง (hentai) เปิดใช้เฉพาะเมื่อคุณอายุ 18 ปีขึ้นไป และถูกกฎหมายในที่ที่คุณอยู่",
                    ));
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        if widgets::play_button(ui, tr("I am 18 or older — turn on", "ฉันอายุ 18 ปีขึ้นไป — เปิดเลย")).clicked() {
                            self.settings.adult_sources = true;
                            self.settings.save();
                            close = true;
                            self.reload_sources();
                        }
                        if widgets::secondary_button(ui, tr("Cancel", "ยกเลิก")).clicked() {
                            close = true;
                        }
                    });
                });
            if close || m.should_close() {
                self.adult_confirm = false;
            }
        }
    }
}
