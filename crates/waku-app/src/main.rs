//! Waku — desktop anime player (Rust + egui) with multi-source failover.
//!
//!   waku-app                 open the GUI
//!   waku-app --probe <id>    test one source in the console (latest → detail → player → stream)
//!   waku-app --probe all     test every enabled source
//!   waku-app --search <q>    search every source in the console

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod detail;
mod images;
mod pages;
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

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = console_mode(&args) {
        std::process::exit(code);
    }

    let options = eframe::NativeOptions {
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

/// Small orange play-button icon drawn in code (no asset files needed).
fn app_icon() -> egui::IconData {
    let n = 64usize;
    let mut rgba = vec![0u8; n * n * 4];
    let c = (n as f32 - 1.0) / 2.0;
    for y in 0..n {
        for x in 0..n {
            let (fx, fy) = (x as f32 - c, y as f32 - c);
            let d = (fx * fx + fy * fy).sqrt();
            let i = (y * n + x) * 4;
            if d <= c {
                // play triangle
                let tri = fx > -9.0 && fx < 14.0 && fy.abs() < (14.0 - fx) * 0.62;
                let (r, g, b) = if tri { (0, 0, 0) } else { (0xf4, 0x75, 0x21) };
                rgba[i..i + 4].copy_from_slice(&[r, g, b, 255]);
            }
        }
    }
    egui::IconData {
        rgba,
        width: n as u32,
        height: n as u32,
    }
}
