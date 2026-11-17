//! Media Lite — a desktop Emby client.
//!
//! Built from a static study of the original program (see MECHANICS.md): the same
//! Emby REST calls, the same `Emby Client="…"` sign-in scheme, the same layout of a
//! sidebar, home rows, library grids, a detail page and a player. Unlike the
//! original there is no paid tier, so servers can be added without limit.
//!
//! The window speaks English and Chinese; `--lang zh` starts in Chinese, and the
//! choice is remembered in the settings file.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod api;
mod app;
mod i18n;
mod model;
mod mpv;
mod net;
mod platform;
mod preferences;
mod sample;
mod theme;

use app::AppLite;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let mut screenshot: Option<PathBuf> = None;
    let mut demo: Option<String> = None;
    let mut autoplay = false;
    let mut view: Option<String> = None;
    let mut lang: Option<i18n::Lang> = None;
    let mut size: Option<[f32; 2]> = None;
    let mut i = 1;
    while i < args.len() {
        let a = &args[i];
        if a == "--screenshot" {
            if i + 1 < args.len() {
                screenshot = Some(PathBuf::from(&args[i + 1]));
                i += 1;
            }
        } else if let Some(rest) = a.strip_prefix("--screenshot=") {
            screenshot = Some(PathBuf::from(rest));
        } else if a == "--view" {
            if i + 1 < args.len() {
                view = Some(args[i + 1].clone());
                i += 1;
            }
        } else if a == "--autoplay" {
            autoplay = true;
        } else if a == "--demo" {
            if i + 1 < args.len() {
                demo = Some(args[i + 1].clone());
                i += 1;
            }
        } else if let Some(rest) = a.strip_prefix("--demo=") {
            demo = Some(rest.to_string());
        } else if a == "--lang" {
            if i + 1 < args.len() {
                lang = Some(i18n::from_code(&args[i + 1]));
                i += 1;
            }
        } else if let Some(rest) = a.strip_prefix("--lang=") {
            lang = Some(i18n::from_code(rest));
        } else if a == "--width" {
            if i + 1 < args.len() {
                if let Ok(w) = args[i + 1].parse::<f32>() {
                    size.get_or_insert([1280.0, 800.0])[0] = w;
                }
                i += 1;
            }
        } else if a == "--height" {
            if i + 1 < args.len() {
                if let Ok(h) = args[i + 1].parse::<f32>() {
                    size.get_or_insert([1280.0, 800.0])[1] = h;
                }
                i += 1;
            }
        }
        i += 1;
    }

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size(size.unwrap_or([1280.0, 800.0]))
        .with_min_inner_size([900.0, 600.0])
        .with_decorations(false)
        .with_title("Media Lite")
        .with_app_id("media-lite");
    if screenshot.is_some() {
        // A check must never steal focus, and on Windows it also stays behind
        // other windows (a covered window still draws there).
        viewport = viewport.with_active(false);
        #[cfg(target_os = "windows")]
        {
            viewport = viewport.with_window_level(eframe::egui::WindowLevel::AlwaysOnBottom);
        }
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    let shot_path = screenshot.clone();
    let demo_name = demo.clone();
    eframe::run_native(
        "Media Lite",
        options,
        Box::new(move |cc| {
            Ok(Box::new(AppLite::new(
                cc,
                shot_path.clone(),
                demo_name.clone(),
                autoplay,
                view.clone(),
                lang,
            )))
        }),
    )?;
    Ok(())
}
