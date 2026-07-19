//! AutoTBH_Monitor — Tauri shell. Boots the embedded axum backend on 127.0.0.1:5260,
//! then opens the window on it (the Nuxt SPA is served by axum, same origin as /api).

mod currency;
mod farm;
mod insights;
mod memory;
mod meter;
mod news;
mod pricing;
mod save;
mod server;
mod steam;
mod wiki;

use std::path::PathBuf;
use std::sync::atomic::AtomicU32;
use std::sync::Arc;
use tauri::Manager;

fn resolve_dirs(app: &tauri::App) -> (PathBuf, PathBuf) {
    // Release: the bundled resource dir is authoritative.
    // Debug: prefer the live source tree — Tauri's resource copy under target/debug goes stale
    // as soon as you edit anything in data/, which silently serves outdated files.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let res = app.path().resource_dir().ok();

    let src_data = manifest.join("../data");
    let src_frontend = manifest.join("../frontend/.output/public");
    let res_data = res.as_ref().map(|r| r.join("data"));
    let res_frontend = res.as_ref().map(|r| r.join("frontend"));

    let pick = |src: PathBuf, packaged: Option<PathBuf>| -> PathBuf {
        let packaged = packaged.filter(|p| p.exists());
        if cfg!(debug_assertions) && src.exists() {
            src
        } else {
            packaged.unwrap_or(src)
        }
    };

    (pick(src_data, res_data), pick(src_frontend, res_frontend))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let (data_dir, frontend_dir) = resolve_dirs(app);
            save::set_data_dir(data_dir.clone());
            steam::set_data_dir(data_dir.clone());

            // Validate against the table — an out-of-range code used to panic every handler
            // that looked it up.
            let initial_currency: u32 = std::env::var("TSM_CURRENCY")
                .ok()
                .and_then(|v| v.parse::<u32>().ok())
                .filter(|c| currency::get(*c).is_some())
                .unwrap_or(1);

            // Built-in live meter (DPS / gold / EXP / run tracker). Opt-in: off until enabled.
            let meter = meter::Meter::new(data_dir.clone());
            meter.spawn_sampler();
            if std::env::var("TSM_METER").as_deref() == Ok("1") {
                meter.set_enabled(true);
            }

            let state = server::AppState {
                data_dir,
                frontend_dir,
                currency: Arc::new(AtomicU32::new(initial_currency)),
                meter,
                scan: Arc::new(std::sync::Mutex::new(Default::default())),
                items_progress: Arc::new(std::sync::Mutex::new(Default::default())),
            };

            // Bind the listener first (so the window can load immediately), then serve in the background.
            tauri::async_runtime::spawn(async move {
                if let Err(e) = server::serve(state).await {
                    eprintln!("[server] fatal: {e}");
                }
            });

            // Give the listener a moment to bind, then open the window on the local server.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                for _ in 0..50 {
                    if reqwest::get(format!("http://127.0.0.1:{}/__tsm-ping", server::port())).await.is_ok() {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                let url = tauri::Url::parse(&format!("http://localhost:{}", server::port())).unwrap();
                let _ = tauri::WebviewWindowBuilder::new(&handle, "main", tauri::WebviewUrl::External(url))
                    .title("AutoTBH_Monitor")
                    .inner_size(1280.0, 840.0)
                    .min_inner_size(900.0, 600.0)
                    .build();
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running AutoTBH_Monitor");
}
