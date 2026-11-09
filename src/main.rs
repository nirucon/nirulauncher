//! NIRULAUNCHER — native application and desktop-action launcher.
//!
//! Keyboard-driven, one process per invocation. No indexing daemon, no network
//! and no shell: applications are started from argv arrays parsed from their
//! desktop entries.

mod app;
mod calc;
mod compose;
mod config;
mod desktop;
mod emoji_font;
mod input;
mod intent;
mod modes;
mod paste;
mod providers;
mod rank;
mod render;
mod saved;
mod ui;
mod usage;
mod websearch;

// The crate prelude: every module starts with `use crate::*;` and so sees the
// shared types, the views and these imports without long paths.
use app::*;
use input::*;
use modes::{
    actions::*, apps::*, audio::*, clip::*, colors::*, emoji::*, health::*, keys::*, manual,
    settings::*, web::*, *,
};
use providers::system::*;
use providers::{eventsound, sound, tailscale, wifi};
use rank::*;
use ui::*;

use config::Config;
use render::Renderer;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};
use x11rb::{
    connection::Connection,
    protocol::{randr::ConnectionExt as _, xproto::*, Event},
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = args.first().map(String::as_str).unwrap_or("--apps");
    let result = match arg {
        "--version" => {
            println!("NIRULAUNCHER {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "--check-config" => match Config::load() {
            Ok(cfg) => {
                println!("{}", cfg.describe());
                Ok(())
            }
            Err(e) => Err(e),
        },
        "--list" => {
            let apps = desktop::discover();
            for app in apps {
                println!("{}\t{}", app.name, app.id);
            }
            Ok(())
        }
        "--list-actions" => match Config::load() {
            Ok(cfg) => {
                for item in action_items(&cfg) {
                    println!(
                        "{}\t{}\t{}\t{}",
                        item.group, item.name, item.detail, item.key
                    );
                }
                Ok(())
            }
            Err(e) => Err(e),
        },
        "--search" | "--search=" => {
            // `--search PHRASE` opens the address bar with the text already in
            // it, which is the useful thing for a keyboard shortcut that
            // passes the selection. With no phrase it starts empty.
            let phrase = args
                .get(1)
                .cloned()
                .or_else(|| arg.strip_prefix("--search=").map(str::to_string))
                .unwrap_or_default();
            match Ui::new("search") {
                Ok(mut ui) => {
                    ui.query = phrase;
                    ui.refilter();
                    run_ui(ui)
                }
                Err(e) => Err(e),
            }
        }
        "--list-search" => match Config::load() {
            Ok(cfg) => {
                for source in websearch::sources(&cfg.search_sources) {
                    println!(
                        "{}\t{}\t{}\t{}",
                        source.label,
                        source.id,
                        source.url,
                        source.host()
                    );
                }
                Ok(())
            }
            Err(e) => Err(e),
        },
        "--wifi-status" | "--wifi-scan" => {
            let snapshot = wifi::snapshot(arg == "--wifi-scan");
            serde_json::to_string_pretty(&snapshot)
                .map(|text| println!("{text}"))
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)
                .map(|_| ())
        }
        "--list-files" => match Config::load() {
            Ok(cfg) => {
                let query = args[1..].join(" ");
                modes::files::print_search(&cfg.files, &query);
                Ok(())
            }
            Err(e) => Err(e),
        },
        "--wired-status" => {
            let snapshot = wifi::wired_snapshot();
            serde_json::to_string_pretty(&snapshot)
                .map(|text| println!("{text}"))
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)
                .map(|_| ())
        }
        "--apps" | "--actions" | "--system-actions" | "--windows" | "--overview" | "--quick"
        | "--clipboard" | "--microphone" | "--bluetooth" | "--tailscale" | "--displays"
        | "--power" | "--keys" | "--settings" | "--themes" | "--timing" | "--input" | "--wifi"
        | "--wired" | "--sound" | "--clip" | "--health" | "--agent" | "--share"
        | "--share-pair" | "--emoji" | "--colors" | "--files" | "--manual" => {
            let mode = match arg {
                "--actions" => "actions",
                "--system-actions" => "system-actions",
                "--windows" | "--overview" => "windows",
                "--quick" => "quick",
                "--agent" => "agent",
                "--share" => "share",
                "--share-pair" => "share-pair",
                "--clipboard" => "clipboard",
                "--microphone" => "microphone",
                "--bluetooth" => "bluetooth",
                "--tailscale" => "tailscale",
                "--displays" => "displays",
                "--power" => "power",
                "--keys" => "keys",
                "--settings" => "settings",
                "--themes" => "themes",
                "--timing" => "timing",
                "--input" => "input",
                "--wifi" => "wifi",
                "--wired" => "wired",
                "--sound" => "sound",
                "--clip" => "clip",
                "--health" => "health",
                "--emoji" => "emoji",
                "--colors" => "colors",
                "--files" => "files",
                "--manual" => "manual",
                _ => "apps",
            };
            match Ui::new(mode) {
                Ok(ui) => run_ui(ui),
                Err(e) => Err(e),
            }
        }
        "--help" => {
            println!(
                "Usage: nirulauncher [--apps|--search [PHRASE]|--actions|--system-actions|--windows|--overview|--quick|--agent|--share|--share-pair|--emoji|--colors|--files|--manual|--clipboard|--microphone|--bluetooth|--tailscale|--displays|--power|--keys|--settings|--themes|--timing|--input|--wifi|--wired|--sound|--clip|--health|--wifi-status|--wifi-scan|--wired-status|--check-config|--list|--list-actions|--list-search|--list-files [QUERY]|--version]"
            );
            Ok(())
        }
        _ => {
            eprintln!(
                "Usage: nirulauncher [--apps|--search [PHRASE]|--actions|--system-actions|--windows|--overview|--quick|--agent|--share|--share-pair|--emoji|--colors|--files|--manual|--clipboard|--microphone|--bluetooth|--tailscale|--displays|--power|--sound|--clip|--health|--check-config|--list|--list-actions|--list-search|--list-files [QUERY]|--version]"
            );
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("nirulauncher: {e}");
        std::process::exit(1);
    }
}

/// Run the window; an emoji chosen with Enter is pasted once it has closed.
fn run_ui(mut ui: Ui) -> Result<()> {
    ui.run()?;
    let Some(text) = ui.paste_after.take() else {
        return Ok(());
    };
    let launcher = ui.win;
    drop(ui);
    let mut paster = paste::Paster::new()?;
    if let Err(e) = paster.paste(&text, launcher) {
        eprintln!("nirulauncher: {e}; the text is on the clipboard instead");
    }
    paster.linger();
    Ok(())
}
