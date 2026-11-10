//! Launching the external video player (mpv by default, like Waku-cli).

use crate::http::UA;
use crate::model::{Stream, Variant};
use crate::text::origin_of;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayOutcome {
    /// Played (or the user closed the player).
    Played,
    /// The player could not open the stream — try the next one.
    LoadFailed,
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(|p| p.to_path_buf())
}

/// Find a bundled tool next to our executable (., ./mpv, ./bin, ./tools)
/// before falling back to PATH. GUI app: `.exe` first so no console pops up.
pub fn find_tool(name: &str) -> PathBuf {
    let exts: &[&str] = if cfg!(windows) {
        &[".exe", ".com"]
    } else {
        &[""]
    };
    if let Some(dir) = exe_dir() {
        for sub in ["", "mpv", "bin", "tools"] {
            for ext in exts {
                let p = dir.join(sub).join(format!("{name}{ext}"));
                if p.is_file() {
                    return p;
                }
            }
        }
    }
    PathBuf::from(name)
}

/// Resolve the configured player command to a path.
pub fn player_path(cmd: &str) -> PathBuf {
    if cmd.contains(['/', '\\']) || cmd.ends_with(".exe") {
        PathBuf::from(cmd)
    } else {
        find_tool(cmd)
    }
}

fn is_mpv(p: &std::path::Path) -> bool {
    p.file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.to_lowercase().contains("mpv"))
}

/// Is the player usable? (runs `<player> --version`)
pub fn player_available(cmd: &str) -> bool {
    let mut c = Command::new(player_path(cmd));
    c.arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    hide_console(&mut c);
    c.status().is_ok()
}

#[cfg(windows)]
fn hide_console(c: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    c.creation_flags(CREATE_NO_WINDOW);
}
#[cfg(not(windows))]
fn hide_console(_: &mut Command) {}

/// Play one variant of a stream, trying the Referer candidates in turn.
/// Blocks until the player exits.
pub fn play(player_cmd: &str, stream: &Stream, v: &Variant, title: &str) -> Result<PlayOutcome> {
    let mut refs: Vec<String> = vec![origin_of(&stream.page)];
    if stream.direct() {
        for extra in [origin_of(&v.url), stream.page.clone()] {
            if !refs.contains(&extra) {
                refs.push(extra);
            }
        }
    }
    for rf in &refs {
        if launch(player_cmd, &v.url, rf, title, stream.direct())? == PlayOutcome::Played {
            return Ok(PlayOutcome::Played);
        }
    }
    Ok(PlayOutcome::LoadFailed)
}

fn launch(
    player_cmd: &str,
    target: &str,
    referer: &str,
    title: &str,
    direct: bool,
) -> Result<PlayOutcome> {
    let player = player_path(player_cmd);
    let mut cmd = Command::new(&player);
    if is_mpv(&player) {
        let ytdlp = find_tool("yt-dlp");
        if ytdlp.is_file() {
            cmd.arg(format!(
                "--script-opts=ytdl_hook-ytdl_path={}",
                ytdlp.display()
            ));
        }
        if direct {
            cmd.arg("--ytdl=no")
                .arg("--demuxer-lavf-o=allowed_extensions=ALL");
            if target
                .to_lowercase()
                .split('?')
                .next()
                .unwrap_or("")
                .ends_with(".txt")
            {
                cmd.arg("--demuxer-lavf-format=hls");
            }
        }
        cmd.arg("--msg-level=ffmpeg/demuxer=error")
            .arg("--force-window=immediate")
            .arg("--keep-open=no")
            .arg(
                "--ytdl-raw-options=extractor-args=generic:impersonate,socket-timeout=60,retries=3",
            )
            .arg(format!("--referrer={referer}"))
            .arg(format!("--user-agent={UA}"))
            .arg(format!("--force-media-title={title}"))
            .arg(format!("--title={title}"));
    } else if player.to_string_lossy().to_lowercase().contains("vlc") {
        cmd.arg(format!("--http-referrer={referer}"))
            .arg(format!("--http-user-agent={UA}"))
            .arg(format!("--meta-title={title}"));
    }
    cmd.arg(target);
    let status = cmd.status().with_context(|| {
        format!(
            "cannot start '{}' — put mpv.exe next to the app (./mpv/mpv.exe) or set its path in Settings",
            player.display()
        )
    })?;
    // mpv: 2 = file could not be loaded; other non-zero (user quit …) is not retried
    Ok(if status.success() || status.code() != Some(2) {
        PlayOutcome::Played
    } else {
        PlayOutcome::LoadFailed
    })
}
