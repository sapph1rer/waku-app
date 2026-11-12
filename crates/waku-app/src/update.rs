//! Updates from GitHub.
//!
//! * The app: the newest Release of the repository (Settings › Updates; default `sapph1rer/waku-app`). Its
//!   zip (`waku-windows-x64.zip`: waku-app.exe, waku-browser.exe, libmpv-2.dll …) is downloaded, checked against
//!   the `.sha256` published beside it, unpacked, and copied over the program's folder. Windows lets a running
//!   .exe / .dll be renamed but not overwritten, so each old file becomes `<name>.old-<n>` first (removed at the
//!   next start); then the new waku-app.exe is started and this one closes.
//! * The sources' rules: `crates/waku-core/sources.default.toml` on the main branch, taken when its `revision`
//!   is higher (`waku_core::profile::check_remote_sources`).
//!
//! `scripts/make-release.ps1` builds the zip + checksum and publishes the Release.

use crate::app::{Msg, WakuApp};
use crate::i18n::tr;
use anyhow::{Context, Result, anyhow, bail};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug)]
pub struct Release {
    pub tag: String,
    pub version: (u32, u32, u32),
    pub notes: String,
    pub page: String,
    pub asset_url: Option<String>,
    pub asset_name: String,
    pub asset_size: u64,
    pub sha_url: Option<String>,
}

#[derive(Default)]
pub struct UpdateState {
    pub checking: bool,
    pub checked: bool,
    pub latest: Option<Release>,
    pub error: Option<String>,
    /// downloading / installing: (bytes done, total)
    pub progress: Option<Arc<Mutex<(u64, u64)>>>,
    pub installed: Option<PathBuf>,
    /// newer sources rules waiting because sources.toml was edited by hand: (text, revision)
    pub sources_pending: Option<(String, u32)>,
    pub sources_note: Option<String>,
}

pub fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let core = s.split(['-', '+']).next()?;
    let mut it = core.split('.').map(|x| x.parse::<u32>().ok());
    let a = it.next()??;
    let b = it.next().flatten().unwrap_or(0);
    let c = it.next().flatten().unwrap_or(0);
    Some((a, b, c))
}

pub fn is_newer(release: (u32, u32, u32)) -> bool {
    parse_version(VERSION).is_some_and(|me| release > me)
}

/// Is this a build run from the source tree (cargo's target folder)? Those are not replaced.
pub fn dev_build() -> bool {
    std::env::current_exe().map(|p| p.components().any(|c| c.as_os_str() == "target")).unwrap_or(false)
}

pub fn parse_release(v: &serde_json::Value) -> Result<Release> {
    let tag = v["tag_name"].as_str().ok_or_else(|| anyhow!("no releases yet"))?.to_string();
    let version = parse_version(&tag).ok_or_else(|| anyhow!("release tag {tag} is not a version"))?;
    let assets: Vec<&serde_json::Value> = v["assets"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
    let name_of = |a: &serde_json::Value| a["name"].as_str().unwrap_or("").to_string();
    // the Windows zip: prefer one saying windows / x64
    let zip = assets
        .iter()
        .filter(|a| name_of(a).to_lowercase().ends_with(".zip"))
        .max_by_key(|a| {
            let n = name_of(a).to_lowercase();
            (n.contains("windows") || n.contains("win")) as u8 * 2 + (n.contains("x64") || n.contains("x86_64")) as u8
        });
    let (asset_url, asset_name, asset_size) = match zip {
        Some(a) => (a["browser_download_url"].as_str().map(String::from), name_of(a), a["size"].as_u64().unwrap_or(0)),
        None => (None, String::new(), 0),
    };
    let sha_url = assets
        .iter()
        .find(|a| !asset_name.is_empty() && name_of(a).eq_ignore_ascii_case(&format!("{asset_name}.sha256")))
        .and_then(|a| a["browser_download_url"].as_str().map(String::from));
    Ok(Release {
        tag,
        version,
        notes: v["body"].as_str().unwrap_or("").trim().to_string(),
        page: v["html_url"].as_str().unwrap_or("").to_string(),
        asset_url,
        asset_name,
        asset_size,
        sha_url,
    })
}

pub fn latest_release(http: &waku_core::http::Http, repo: &str) -> Result<Release> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", repo.trim().trim_matches('/'));
    let v = http
        .get_json(&url, &[("Accept", "application/vnd.github+json"), ("X-GitHub-Api-Version", "2022-11-28")])
        .with_context(|| format!("GitHub {repo}"))?;
    if let Some(m) = v["message"].as_str()
        && v.get("tag_name").is_none()
    {
        bail!("GitHub: {m}");
    }
    parse_release(&v)
}

fn client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(120))
        .user_agent(concat!("waku/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("http client")
}

fn sha256_hex(path: &Path) -> Result<String> {
    use sha2::Digest;
    let mut f = std::fs::File::open(path)?;
    let mut h = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Unpack `zip` into `dir`; a single top folder inside the zip is skipped. Returns the files (relative paths).
pub fn unpack(zip: &Path, dir: &Path) -> Result<Vec<PathBuf>> {
    let mut a = zip::ZipArchive::new(std::fs::File::open(zip)?).context("the download is not a zip")?;
    let names: Vec<PathBuf> = (0..a.len()).filter_map(|i| a.by_index(i).ok().and_then(|f| f.enclosed_name())).collect();
    let top: Option<PathBuf> = {
        let firsts: std::collections::HashSet<_> = names.iter().filter_map(|p| p.components().next().map(|c| c.as_os_str().to_owned())).collect();
        (firsts.len() == 1 && names.iter().all(|p| p.components().count() > 1)).then(|| PathBuf::from(firsts.into_iter().next().unwrap()))
    };
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir)?;
    let mut out = Vec::new();
    for i in 0..a.len() {
        let mut f = a.by_index(i)?;
        let Some(p) = f.enclosed_name() else { continue };
        let rel = match &top {
            Some(t) => p.strip_prefix(t).map(Path::to_path_buf).unwrap_or(p),
            None => p,
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        let dest = dir.join(&rel);
        if f.is_dir() {
            std::fs::create_dir_all(&dest)?;
            continue;
        }
        if let Some(d) = dest.parent() {
            std::fs::create_dir_all(d)?;
        }
        let mut w = std::fs::File::create(&dest)?;
        std::io::copy(&mut f, &mut w)?;
        out.push(rel);
    }
    Ok(out)
}

/// Copy the unpacked files over `target`, moving files in use aside (`.old-N`) first.
pub fn swap_in(src: &Path, files: &[PathBuf], target: &Path) -> Result<()> {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    for rel in files {
        let from = src.join(rel);
        let to = target.join(rel);
        if let Some(d) = to.parent() {
            std::fs::create_dir_all(d)?;
        }
        if to.exists() {
            let aside = to.with_file_name(format!("{}.old-{stamp}", to.file_name().unwrap_or_default().to_string_lossy()));
            std::fs::rename(&to, &aside).with_context(|| format!("move aside {}", to.display()))?;
        }
        std::fs::copy(&from, &to).with_context(|| format!("copy {}", to.display()))?;
    }
    Ok(())
}

/// Files left by an earlier update (`*.old-N`): removed at start (they are no longer in use).
pub fn clean_old() {
    let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) else { return };
    let mut stack = vec![dir];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n != "Downloads") {
                    stack.push(p);
                }
            } else if p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.contains(".old-")) {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
}

/// Download, check and put the release in place. Returns the new program to start.
pub fn install(r: &Release, progress: &Mutex<(u64, u64)>) -> Result<PathBuf> {
    let url = r.asset_url.clone().ok_or_else(|| anyhow!("the release {} has no Windows zip", r.tag))?;
    let exe = std::env::current_exe()?;
    let target = exe.parent().ok_or_else(|| anyhow!("program folder"))?.to_path_buf();
    let work = waku_core::profile::cache_dir().join("update");
    std::fs::create_dir_all(&work)?;
    let zip = work.join(&r.asset_name);
    let c = client();
    let mut resp = c.get(&url).send()?.error_for_status()?;
    let total = resp.content_length().unwrap_or(r.asset_size);
    *progress.lock().unwrap() = (0, total);
    let mut f = std::fs::File::create(&zip)?;
    let mut buf = vec![0u8; 1 << 16];
    let mut done = 0u64;
    loop {
        let n = resp.read(&mut buf)?;
        if n == 0 {
            break;
        }
        f.write_all(&buf[..n])?;
        done += n as u64;
        *progress.lock().unwrap() = (done, total);
    }
    drop(f);
    if total > 0 && done != total {
        bail!("the download stopped early ({done} of {total} bytes)");
    }
    // checksum published beside the zip
    match &r.sha_url {
        Some(u) => {
            let want = c.get(u).send()?.error_for_status()?.text()?;
            let want = want.split_whitespace().next().unwrap_or("").to_lowercase();
            let got = sha256_hex(&zip)?;
            if want.len() != 64 || want != got {
                let _ = std::fs::remove_file(&zip);
                bail!("the download does not match its checksum — not installed");
            }
        }
        None => bail!("the release has no {}.sha256 — not installed (unverified)", r.asset_name),
    }
    let dir = work.join(&r.tag);
    let files = unpack(&zip, &dir)?;
    let exe_name = exe.file_name().unwrap_or_default().to_owned();
    if !files.iter().any(|p| p.file_name() == Some(exe_name.as_os_str()) || p.file_name().is_some_and(|n| n == "waku-app.exe" || n == "waku-app")) {
        bail!("the zip has no waku-app.exe");
    }
    swap_in(&dir, &files, &target)?;
    let _ = std::fs::remove_file(&zip);
    let new_exe = target.join(files.iter().find(|p| p.file_name() == Some(exe_name.as_os_str())).cloned().unwrap_or_else(|| PathBuf::from(&exe_name)));
    Ok(new_exe)
}

impl WakuApp {
    pub fn check_app_update(&mut self, manual: bool) {
        if self.upd.checking {
            return;
        }
        self.upd.checking = true;
        self.upd.error = None;
        let repo = self.settings.update_repo.clone();
        self.spawn(move |m| Msg::Update { manual, res: latest_release(m.http(), &repo).map_err(|e| format!("{e:#}")) });
    }

    pub fn on_update(&mut self, manual: bool, res: Result<Release, String>) {
        self.upd.checking = false;
        self.upd.checked = true;
        match res {
            Ok(r) => {
                let newer = is_newer(r.version);
                if newer && !manual {
                    let tag = r.tag.clone();
                    self.notify(crate::trf!("Waku {tag} is out — Settings › Updates", "Waku {tag} ออกแล้ว — ไปที่ ตั้งค่า › อัปเดต"));
                }
                self.upd.latest = Some(r);
            }
            Err(e) => {
                if manual {
                    self.upd.error = Some(crate::app::first_line(&e).to_string());
                }
            }
        }
    }

    pub fn start_install(&mut self) {
        let Some(r) = self.upd.latest.clone() else { return };
        let prog = Arc::new(Mutex::new((0u64, 0u64)));
        self.upd.progress = Some(prog.clone());
        self.upd.error = None;
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let res = install(&r, &prog).map_err(|e| format!("{e:#}"));
            let _ = tx.send(Msg::Installed(res));
            ctx.request_repaint();
        });
    }

    pub fn on_installed(&mut self, res: Result<PathBuf, String>) {
        self.upd.progress = None;
        match res {
            Ok(p) => self.upd.installed = Some(p),
            Err(e) => self.upd.error = Some(crate::app::first_line(&e).to_string()),
        }
    }

    /// Start the new version and close this one.
    pub fn restart_into(&mut self, exe: &Path) {
        self.settings.save();
        match std::process::Command::new(exe).spawn() {
            Ok(_) => self.ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Err(e) => self.upd.error = Some(format!("{}: {e}", tr("Could not start the new version", "เปิดเวอร์ชันใหม่ไม่ได้"))),
        }
    }

    pub fn check_sources_update(&mut self, manual: bool) {
        let repo = self.settings.update_repo.clone();
        self.spawn(move |m| Msg::SourcesUpdate { manual, res: waku_core::profile::check_remote_sources(m.http(), &repo).map_err(|e| format!("{e:#}")) });
    }

    pub fn on_sources_update(&mut self, manual: bool, res: Result<Option<(String, u32)>, String>) {
        match res {
            Ok(Some((text, rev))) => {
                let (_, edited) = waku_core::profile::local_sources_state();
                if edited || !self.settings.auto_update_sources {
                    self.upd.sources_pending = Some((text, rev));
                    self.upd.sources_note = Some(crate::trf!(
                        "New sources rules (revision {rev}) — your sources.toml was edited, so they wait in Settings › Updates",
                        "มีกฎดึงข้อมูลใหม่ (รุ่น {rev}) — คุณแก้ sources.toml เองไว้ จึงรอให้กดใช้ใน ตั้งค่า › อัปเดต"
                    ));
                    if let Some(n) = self.upd.sources_note.clone() {
                        self.notify(n);
                    }
                } else {
                    self.apply_sources(&text, rev, false);
                }
            }
            Ok(None) => {
                if manual {
                    let (rev, _) = waku_core::profile::local_sources_state();
                    self.upd.sources_note = Some(crate::trf!("Sources rules are up to date (revision {rev})", "กฎดึงข้อมูลเป็นรุ่นล่าสุดแล้ว (รุ่น {rev})"));
                }
            }
            Err(e) => {
                if manual {
                    self.upd.sources_note = Some(crate::app::first_line(&e).to_string());
                }
            }
        }
    }

    pub fn apply_sources(&mut self, text: &str, rev: u32, backup: bool) {
        match waku_core::profile::apply_sources(text, backup) {
            Ok(saved) => {
                self.upd.sources_pending = None;
                self.reload_sources();
                let note = match saved {
                    Some(b) => {
                        let b = b.display().to_string();
                        crate::trf!("Sources rules updated to revision {rev} (your old file: {b})", "อัปเดตกฎดึงข้อมูลเป็นรุ่น {rev} แล้ว (ไฟล์เดิมสำรองไว้ที่ {b})")
                    }
                    None => crate::trf!("Sources rules updated to revision {rev}", "อัปเดตกฎดึงข้อมูลเป็นรุ่น {rev} แล้ว"),
                };
                self.upd.sources_note = Some(note.clone());
                self.notify(note);
            }
            Err(e) => self.upd.sources_note = Some(format!("{e:#}")),
        }
    }

    /// Settings › Updates.
    pub fn updates_section(&mut self, ui: &mut egui::Ui) {
        use crate::theme::*;
        use crate::widgets;
        use egui::RichText;
        use egui_phosphor::regular as icon;
        ui.label(RichText::new(tr("Updates", "อัปเดต")).font(bold(18.0)));
        ui.add_space(6.0);
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Waku {VERSION}")).font(bold(14.0)));
            ui.label(RichText::new(tr("· from GitHub", "· จาก GitHub")).color(MUTED));
            changed |= ui.add(egui::TextEdit::singleline(&mut self.settings.update_repo).hint_text("owner/repo").desired_width(200.0)).changed();
        });
        ui.horizontal(|ui| {
            changed |= ui.checkbox(&mut self.settings.check_updates, tr("Look for a new version on start", "ตรวจหาเวอร์ชันใหม่ตอนเปิดแอป")).changed();
            changed |= ui.checkbox(&mut self.settings.auto_update_sources, tr("Take new sources rules by themselves", "อัปเดตกฎดึงข้อมูลของแต่ละเว็บให้เอง")).changed();
        });
        if changed {
            self.settings.save();
        }
        ui.add_space(8.0);
        // the app
        ui.horizontal(|ui| {
            if self.upd.checking {
                ui.add(egui::Spinner::new().size(16.0).color(ACCENT));
                ui.label(RichText::new(tr("Checking…", "กำลังตรวจ…")).color(MUTED));
            } else if widgets::ghost_button(ui, &format!("{}  {}", icon::ARROWS_CLOCKWISE, tr("Check now", "ตรวจตอนนี้"))).clicked() {
                self.check_app_update(true);
                self.check_sources_update(true);
            }
            match &self.upd.latest {
                Some(r) if is_newer(r.version) => {
                    let tag = &r.tag;
                    ui.label(RichText::new(crate::trf!("{tag} is available", "มีเวอร์ชัน {tag} แล้ว")).color(ACCENT_HI).strong());
                }
                Some(r) => {
                    let tag = &r.tag;
                    ui.label(RichText::new(crate::trf!("Up to date (newest: {tag})", "เป็นเวอร์ชันล่าสุดแล้ว (ล่าสุด: {tag})")).color(GOOD));
                }
                None if self.upd.checked => {}
                None => {}
            }
        });
        if let Some(e) = &self.upd.error {
            ui.label(RichText::new(e).color(BAD));
        }
        if let Some(r) = self.upd.latest.clone().filter(|r| is_newer(r.version)) {
            if !r.notes.is_empty() {
                egui::CollapsingHeader::new(RichText::new(tr("What's new", "มีอะไรใหม่")).color(MUTED)).id_salt("relnotes").show(ui, |ui| {
                    ui.label(RichText::new(&r.notes).size(12.5));
                });
            }
            if let Some(p) = &self.upd.progress {
                let (d, t) = *p.lock().unwrap();
                let f = if t > 0 { d as f32 / t as f32 } else { 0.0 };
                ui.add(egui::ProgressBar::new(f).text(format!("{:.1} / {:.1} MB", d as f64 / 1e6, t as f64 / 1e6)).fill(ACCENT));
                ui.ctx().request_repaint_after(Duration::from_millis(200));
            } else if let Some(exe) = self.upd.installed.clone() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr("Installed — restart to use it", "ติดตั้งแล้ว — เปิดใหม่เพื่อใช้")).color(GOOD));
                    if widgets::play_button(ui, tr("Restart now", "เปิดใหม่ตอนนี้")).clicked() {
                        self.restart_into(&exe);
                    }
                });
            } else if dev_build() {
                ui.label(RichText::new(tr("This copy runs from the source folder (cargo build) — update it with git, or download the release:", "ตัวนี้รันจากโฟลเดอร์ซอร์ส (cargo build) — อัปเดตด้วย git หรือดาวน์โหลดจากหน้า Release:")).color(MUTED));
                ui.hyperlink(&r.page);
            } else if r.asset_url.is_some() {
                let tag = &r.tag;
                if widgets::play_button(ui, &crate::trf!("Download and install {tag}", "ดาวน์โหลดและติดตั้ง {tag}")).clicked() {
                    self.start_install();
                }
            } else {
                ui.label(RichText::new(tr("This release has no Windows zip — see its page:", "Release นี้ไม่มีไฟล์ zip สำหรับ Windows — ดูที่หน้า:")).color(MUTED));
                ui.hyperlink(&r.page);
            }
        }
        // the sources' rules
        ui.add_space(8.0);
        let (rev, edited) = waku_core::profile::local_sources_state();
        ui.horizontal(|ui| {
            let state = if edited { tr("edited by you", "คุณแก้เอง") } else { tr("as published", "ตามที่เผยแพร่") };
            ui.label(RichText::new(crate::trf!("Sources rules: revision {rev} ({state})", "กฎดึงข้อมูล: รุ่น {rev} ({state})")).color(MUTED));
        });
        if let Some((text, r)) = self.upd.sources_pending.clone() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(crate::trf!("Revision {r} is available", "มีรุ่น {r} แล้ว")).color(ACCENT_HI));
                if widgets::outline_button(ui, tr("Use it (keep a copy of mine)", "ใช้รุ่นใหม่ (สำรองไฟล์เดิมไว้)")).clicked() {
                    self.apply_sources(&text, r, true);
                }
            });
        }
        if let Some(n) = &self.upd.sources_note {
            ui.label(RichText::new(n).size(12.0).color(DIM));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("0.4"), Some((0, 4, 0)));
        assert_eq!(parse_version("v2.0.0-beta.1"), Some((2, 0, 0)));
        assert_eq!(parse_version("latest"), None);
        assert!((0, 2, 0) > (0, 1, 9));
    }

    #[test]
    fn release_json() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"tag_name":"v0.2.0","body":"notes","html_url":"https://github.com/o/r/releases/tag/v0.2.0","assets":[
            {"name":"waku-src.zip","browser_download_url":"https://x/src.zip","size":5},
            {"name":"waku-windows-x64.zip","browser_download_url":"https://x/win.zip","size":1234},
            {"name":"waku-windows-x64.zip.sha256","browser_download_url":"https://x/win.zip.sha256","size":64}]}"#,
        )
        .unwrap();
        let r = parse_release(&v).unwrap();
        assert_eq!(r.version, (0, 2, 0));
        assert_eq!(r.asset_name, "waku-windows-x64.zip");
        assert_eq!(r.sha_url.as_deref(), Some("https://x/win.zip.sha256"));
        assert!(parse_release(&serde_json::json!({"message": "Not Found"})).is_err());
    }

    /// zip with a top folder → unpacked flat → swapped over a folder whose files are "in use" (moved aside)
    #[test]
    fn unpack_and_swap() {
        let base = std::env::temp_dir().join(format!("waku-upd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("app")).unwrap();
        std::fs::write(base.join("app/waku-app.exe"), b"old").unwrap();
        let zp = base.join("u.zip");
        {
            let mut z = zip::ZipWriter::new(std::fs::File::create(&zp).unwrap());
            let o = zip::write::SimpleFileOptions::default();
            z.start_file("waku-windows-x64/waku-app.exe", o).unwrap();
            z.write_all(b"new").unwrap();
            z.start_file("waku-windows-x64/libmpv-2.dll", o).unwrap();
            z.write_all(b"dll").unwrap();
            z.finish().unwrap();
        }
        let files = unpack(&zp, &base.join("x")).unwrap();
        assert!(files.contains(&PathBuf::from("waku-app.exe")));
        swap_in(&base.join("x"), &files, &base.join("app")).unwrap();
        assert_eq!(std::fs::read(base.join("app/waku-app.exe")).unwrap(), b"new");
        assert_eq!(std::fs::read(base.join("app/libmpv-2.dll")).unwrap(), b"dll");
        let old: Vec<_> = std::fs::read_dir(base.join("app")).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().contains(".old-")).collect();
        assert_eq!(old.len(), 1);
        assert_eq!(std::fs::read(old[0].path()).unwrap(), b"old");
        let _ = std::fs::remove_dir_all(&base);
    }
}
