//! Waku — desktop anime player (Rust + egui) with multi-source failover.
//!
//!   waku-app                 open the GUI
//!   waku-app --probe <id>    test one source in the console (latest → detail → player → stream)
//!   waku-app --probe all     test every enabled source
//!   waku-app --search <q>    search every source in the console

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod detail;
mod dev;
mod dlpage;
mod srcpage;
mod scene;
mod discover;
mod order;
mod discord;
mod party;
mod relay;
mod update;
mod download;
mod i18n;
mod images;
mod metaui;
mod onair;
mod pages;
mod playerview;
mod theme;
mod widgets;

use waku_core::SourceManager;

fn console_mode(args: &[String]) -> Option<i32> {
    #[cfg(windows)]
    {
        // release builds have no console: borrow the parent's for CLI modes
        unsafe extern "system" {
            fn AttachConsole(pid: u32) -> i32;
        }
        unsafe {
            AttachConsole(u32::MAX);
        }
    }
    let mgr = SourceManager::load();
    for e in &mgr.load_errors {
        eprintln!("warning: {e}");
    }
    match args.first().map(String::as_str) {
        Some("--probe") => {
            let which = args.get(1).cloned().unwrap_or_else(|| "all".into());
            let ids: Vec<String> = if which == "all" {
                mgr.sources().iter().map(|s| s.id().to_string()).collect()
            } else {
                vec![which]
            };
            for id in ids {
                println!("\n=== {id} ===");
                for s in mgr.diagnose(&id) {
                    println!(
                        "[{}] {:<14} {}",
                        if s.ok { "ok" } else { "FAIL" },
                        s.name,
                        s.detail
                    );
                    for t in s.trace {
                        println!("      {t}");
                    }
                }
            }
            Some(0)
        }
        Some("--search") => {
            let q = args[1..].join(" ");
            for (id, res) in mgr.search_all(&q) {
                match res {
                    Ok(v) => {
                        println!("\n{id}: {} result(s)", v.len());
                        for c in v.iter().take(8) {
                            println!("  {}  {}", c.title, c.url);
                        }
                    }
                    Err(e) => println!("\n{id}: error: {e}"),
                }
            }
            Some(0)
        }
        _ => None,
    }
}

/// Panics (on any thread) are appended to `<config>/waku/crash.log` with a backtrace, for bug reports.
fn crash_log() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let path = waku_core::profile::config_dir().join("crash.log");
        let bt = std::backtrace::Backtrace::force_capture();
        let thread = std::thread::current().name().unwrap_or("?").to_string();
        let when = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let line = format!("--- waku {} · unix {when} · thread {thread}\n{info}\n{bt}\n", env!("CARGO_PKG_VERSION"));
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        // keep the file small: start over past 1 MB
        let old = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let mut f = std::fs::OpenOptions::new().create(true).append(old < 1 << 20).write(true).truncate(old >= 1 << 20).open(&path);
        if let Ok(f) = &mut f {
            use std::io::Write;
            let _ = f.write_all(line.as_bytes());
        }
        default(info);
    }));
}

fn main() -> eframe::Result {
    crash_log();
    let args: Vec<String> = std::env::args().skip(1).collect();
    // waku-browser.exe next to waku.exe: Cloudflare checks and obfuscated players go through it
    waku_core::browser::install();
    if let Some(code) = console_mode(&args) {
        std::process::exit(code);
    }

    let options = eframe::NativeOptions {
        // glow (OpenGL): the built-in player draws libmpv's picture through it
        renderer: eframe::Renderer::Glow,
        viewport: egui::ViewportBuilder::default()
            .with_title("Waku")
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([900.0, 600.0])
            .with_icon(app_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "Waku",
        options,
        Box::new(|cc| Ok(Box::new(app::WakuApp::new(cc)))),
    )
}

/// The app's logo (assets/icon.png: the picture with rounded corners) for the window and taskbar.
fn app_icon() -> egui::IconData {
    let img = image::load_from_memory(include_bytes!("../assets/icon.png")).map(|i| i.to_rgba8());
    match img {
        Ok(i) => egui::IconData { width: i.width(), height: i.height(), rgba: i.into_raw() },
        Err(_) => egui::IconData::default(),
    }
}
