//! Downloads tab: what is downloading now, then every downloaded series as cards (like Home); a series opens
//! its episodes on disk, which play in the built-in player without the internet.

use crate::app::WakuApp;
use crate::download::{self, SeriesFile, Status};
use crate::i18n::{self, tr};
use crate::trf;
use crate::theme::*;
use crate::widgets;
use egui::{CornerRadius, Frame, Margin, RichText, Ui};
use egui_phosphor::regular as icon;
use std::path::{Path, PathBuf};
use waku_core::{AnimeCard, AnimeDetail, Episode};

/// Source id of episodes played from disk.
pub const LOCAL: &str = "local";

fn mb(b: u64) -> String {
    if b >= 1 << 30 { format!("{:.2} GB", b as f64 / (1u64 << 30) as f64) } else { format!("{:.0} MB", b as f64 / (1u64 << 20) as f64) }
}

impl WakuApp {
    pub fn dl_root(&self) -> PathBuf {
        download::root(&self.settings.download_dir)
    }

    /// Reload the list of downloaded series when a download finished (called every frame; cheap otherwise).
    pub fn dl_refresh(&mut self) {
        let g = self.dl.generation.load(std::sync::atomic::Ordering::Relaxed);
        if g != self.dl_gen {
            self.dl_gen = g;
            self.dl_list = download::load_all(&self.dl_root());
        }
    }

    /// Download state of an episode page for the episode list: None = not downloaded, <0 = queued,
    /// 0..1 = downloading, 1 = on disk.
    pub fn dl_state(&self, ep_url: &str) -> Option<f32> {
        if let Some(j) = self.dl.active_for(ep_url) {
            let s = j.state();
            return Some(if s.status == Status::Downloading && s.total > 0 { (s.done as f32 / s.total as f32).clamp(0.01, 0.99) } else { -1.0 });
        }
        self.dl_list.iter().any(|(_, s)| s.episodes.iter().any(|e| e.source_url == ep_url)).then_some(1.0)
    }

    pub fn download_episodes(&mut self, card: &AnimeCard, eps: Vec<Episode>) {
        let root = self.dl_root();
        let mut n = 0;
        for e in eps {
            if self.dl.enqueue(self.mgr.clone(), root.clone(), card.clone(), e, self.ctx.clone()) {
                n += 1;
            }
        }
        if n > 0 {
            self.notify(trf!("Downloading {n} episode(s) — see the Downloads tab", "กำลังดาวน์โหลด {n} ตอน — ดูได้ที่แท็บดาวน์โหลด"));
        }
    }

    /// Play a downloaded episode (the rest of the series on disk is the queue).
    pub fn play_local(&mut self, dir: &Path, s: &SeriesFile, file: &str) {
        let eps: Vec<Episode> = s
            .episodes
            .iter()
            .map(|e| Episode { season: 1, num: e.num, label: e.label.clone(), url: dir.join(&e.file).to_string_lossy().to_string(), dub: e.dub })
            .collect();
        // the series URL stays the website's, so metadata (episode titles, AniSkip) still applies
        let d = AnimeDetail {
            source: LOCAL.into(),
            url: s.card.url.clone(),
            title: s.card.title.clone(),
            poster: s.card.poster.clone(),
            synopsis: String::new(),
            tags: vec![],
            episodes: eps,
        };
        let path = dir.join(file).to_string_lossy().to_string();
        if let Some(e) = d.episodes.iter().find(|e| e.url == path).cloned() {
            self.play_builtin(&d, &e, None);
        }
    }

    /// The website card + episode a downloaded file came from (progress / Continue Watching go there).
    pub fn local_origin(&self, path: &str) -> Option<(AnimeCard, Episode)> {
        let p = Path::new(path);
        let (dir, file) = (p.parent()?, p.file_name()?.to_string_lossy().to_string());
        let (_, s) = self.dl_list.iter().find(|(d, _)| d == dir)?;
        let e = s.episodes.iter().find(|e| e.file == file)?;
        Some((s.card.clone(), Episode { season: 1, num: e.num, label: e.label.clone(), url: e.source_url.clone(), dub: e.dub }))
    }

    pub fn downloads_page(&mut self, ui: &mut Ui) {
        self.dl_refresh();
        if let Some(dir) = self.dl_open.clone() {
            self.downloaded_series(ui, &dir);
            return;
        }
        let root = self.dl_root();
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Downloads", "ดาวน์โหลด")).font(bold(28.0)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::ghost_button(ui, &format!("{}  {}", icon::FOLDER_OPEN, tr("Open folder", "เปิดโฟลเดอร์"))).on_hover_text(root.display().to_string()).clicked() {
                    let _ = std::fs::create_dir_all(&root);
                    open_in_explorer(&root);
                }
                let jobs = self.dl.jobs.lock().unwrap().clone();
                if jobs.iter().any(|j| !matches!(j.state().status, Status::Queued | Status::Resolving | Status::Downloading))
                    && widgets::ghost_button(ui, &format!("{}  {}", icon::BROOM, tr("Clear finished", "ล้างที่เสร็จแล้ว"))).clicked()
                {
                    self.dl.clear_finished();
                }
            });
        });
        ui.add_space(12.0);

        // in progress
        let jobs = self.dl.jobs.lock().unwrap().clone();
        if !jobs.is_empty() {
            let mut retry = None;
            Frame::new().fill(BAR).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(14)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                for j in &jobs {
                    let s = j.state();
                    ui.horizontal(|ui| {
                        let (glyph, col, text) = match &s.status {
                            Status::Queued => (icon::HOURGLASS, MUTED, tr("waiting", "รอคิว").to_string()),
                            Status::Resolving => (icon::MAGNIFYING_GLASS, MUTED, tr("finding the link…", "กำลังหาลิงก์…").to_string()),
                            Status::Downloading => {
                                let pct = (j.fraction() * 100.0).round();
                                (icon::DOWNLOAD_SIMPLE, ACCENT_HI, format!("{pct:.0}% · {} · {}", mb(s.bytes), s.server))
                            }
                            Status::Done => (icon::CHECK_CIRCLE, GOOD, format!("{} · {}", tr("done", "เสร็จแล้ว"), mb(s.bytes))),
                            Status::Failed(e) => (icon::WARNING_CIRCLE, BAD, format!("{}: {e}", tr("failed", "ไม่สำเร็จ"))),
                            Status::Cancelled => (icon::X_CIRCLE, DIM, tr("cancelled", "ยกเลิกแล้ว").to_string()),
                        };
                        ui.label(RichText::new(glyph).size(18.0).color(col));
                        ui.vertical(|ui| {
                            ui.label(RichText::new(format!("{} — {}", j.card.title, i18n::ep_name(&j.ep))).font(bold(14.0)));
                            ui.label(RichText::new(text).size(12.5).color(MUTED));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            match s.status {
                                Status::Queued | Status::Resolving | Status::Downloading => {
                                    if widgets::ghost_button(ui, &format!("{}  {}", icon::X, tr("Cancel", "ยกเลิก"))).clicked() {
                                        self.dl.cancel(j.id);
                                    }
                                }
                                Status::Failed(_) | Status::Cancelled => {
                                    if widgets::ghost_button(ui, &format!("{}  {}", icon::ARROW_CLOCKWISE, tr("Retry", "ลองใหม่"))).clicked() {
                                        retry = Some((j.card.clone(), j.ep.clone()));
                                    }
                                }
                                Status::Done => {}
                            }
                        });
                    });
                    if s.status == Status::Downloading {
                        ui.add(egui::ProgressBar::new(j.fraction()).desired_height(4.0).fill(ACCENT));
                    }
                    ui.add_space(8.0);
                }
            });
            if let Some((c, e)) = retry {
                self.dl.jobs.lock().unwrap().retain(|j| j.ep.url != e.url);
                self.download_episodes(&c, vec![e]);
            }
            ui.add_space(18.0);
        }

        if self.dl_list.is_empty() {
            widgets::empty(
                ui,
                icon::DOWNLOAD_SIMPLE,
                tr("Nothing downloaded yet — press the download button on an episode", "ยังไม่มีเรื่องที่ดาวน์โหลด — กดปุ่มดาวน์โหลดที่ตอนในหน้าเรื่อง"),
            );
            return;
        }
        let total: u64 = self.dl_list.iter().flat_map(|(_, s)| s.episodes.iter().map(|e| e.bytes)).sum();
        let n_eps: usize = self.dl_list.iter().map(|(_, s)| s.episodes.len()).sum();
        let n_series = self.dl_list.len();
        widgets::section_title(ui, &trf!("{n_series} series · {n_eps} episodes", "{n_series} เรื่อง · {n_eps} ตอน"));
        ui.label(RichText::new(format!("{} · {}", mb(total), root.display())).size(12.0).color(DIM));
        ui.add_space(8.0);
        let list = self.dl_list.clone();
        let mut open = None;
        widgets::card_grid(ui, &list, |ui, (dir, s)| {
            let view = self.card_view(&s.card);
            let referer = self.referer(&s.card.source);
            let n = s.episodes.len();
            let sub = trf!("{n} episodes downloaded", "ดาวน์โหลดแล้ว {n} ตอน");
            let resp = widgets::anime_card(ui, &mut self.images, &view, &referer, &sub);
            if resp.clicked() {
                open = Some(dir.clone());
            }
        });
        if let Some(d) = open {
            self.dl_open = Some(d);
        }
    }

    fn downloaded_series(&mut self, ui: &mut Ui, dir: &Path) {
        let Some((_, s)) = self.dl_list.iter().find(|(d, _)| d == dir).cloned() else {
            self.dl_open = None;
            return;
        };
        ui.horizontal(|ui| {
            if widgets::ghost_button(ui, &format!("{}  {}", icon::ARROW_LEFT, tr("Downloads", "ดาวน์โหลด"))).clicked() {
                self.dl_open = None;
            }
        });
        ui.add_space(10.0);
        let referer = self.referer(&s.card.source);
        let meta = self.meta_of(&s.card.url, &s.card.title).cloned();
        let mut act: Option<(String, bool)> = None; // (file, delete?)
        let mut open_online = false;
        ui.horizontal_top(|ui| {
            let view = self.card_view(&s.card);
            widgets::anime_card(ui, &mut self.images, &view, &referer, "");
            ui.add_space(18.0);
            ui.vertical(|ui| {
                ui.label(RichText::new(&s.card.title).font(bold(26.0)));
                let total: u64 = s.episodes.iter().map(|e| e.bytes).sum();
                let n = s.episodes.len();
                ui.label(RichText::new(trf!("{n} episodes on this PC", "มี {n} ตอนในเครื่อง")).color(MUTED));
                ui.label(RichText::new(format!("{} · {}", mb(total), dir.display())).size(12.0).color(DIM));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if let Some(first) = s.episodes.first()
                        && widgets::play_button(ui, tr("Play", "เล่น")).clicked()
                    {
                        act = Some((first.file.clone(), false));
                    }
                    if widgets::outline_button(ui, &format!("{}  {}", icon::GLOBE, tr("Open the page", "เปิดหน้าเรื่อง"))).clicked() {
                        open_online = true;
                    }
                    if widgets::ghost_button(ui, &format!("{}  {}", icon::FOLDER_OPEN, tr("Folder", "โฟลเดอร์"))).clicked() {
                        open_in_explorer(dir);
                    }
                });
            });
        });
        ui.add_space(18.0);
        ui.spacing_mut().item_spacing.y = 10.0;
        for e in &s.episodes {
            let ep = Episode { season: 1, num: e.num, label: e.label.clone(), url: e.source_url.clone(), dub: e.dub };
            let em = meta.as_ref().filter(|_| e.num.fract() == 0.0).and_then(|m| m.ep(e.num as u32));
            let name = em.map(|x| x.title.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| i18n::ep_name(&ep));
            let title = format!("{}. {name}", waku_core::text::fmt_num(e.num));
            let mut parts = vec![if e.dub { i18n::tag("Dub") } else { i18n::tag("Sub") }, mb(e.bytes), e.file.clone()];
            if let Some(f) = self.lib.fraction(&e.source_url).filter(|f| *f > 0.9) {
                let _ = f;
                parts.push(tr("watched", "ดูแล้ว").into());
            }
            let meta_line = parts.join(" · ");
            let desc = em.and_then(|x| x.summary.clone()).unwrap_or_default();
            let c = widgets::EpCard {
                title: &title,
                meta: &meta_line,
                desc: &desc,
                progress: self.lib.fraction(&e.source_url).filter(|f| *f > 0.01),
                selected: false,
                dl: Some(1.0),
                local: true,
                tip: "",
            };
            let thumb = em.and_then(|x| x.thumbnail.clone()).or(s.card.poster.clone());
            let k = widgets::episode_card(ui, &mut self.images, thumb.as_deref(), &referer, &c, &e.file);
            if k.play {
                act = Some((e.file.clone(), false));
            }
            if k.servers {
                act = Some((e.file.clone(), true));
            }
        }
        match act {
            Some((f, false)) => self.play_local(dir, &s, &f),
            Some((f, true)) => {
                download::delete_episode(dir, &f);
                self.dl_list = download::load_all(&self.dl_root());
                self.notify(tr("Episode deleted from this PC", "ลบตอนออกจากเครื่องแล้ว"));
            }
            None => {}
        }
        if open_online {
            self.open(&s.card);
        }
    }
}

fn open_in_explorer(p: &Path) {
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(p).spawn();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("xdg-open").arg(p).spawn();
}

