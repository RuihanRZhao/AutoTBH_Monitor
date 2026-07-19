//! AutoTBH_Monitor — Tauri shell. Boots the embedded axum backend on 127.0.0.1:5260,
//! then opens the window on it (the Nuxt SPA is served by axum, same origin as /api).

mod currency;
mod news;
mod pricing;
mod save;
mod server;
mod steam;

use std::path::PathBuf;
use std::sync::atomic::AtomicU32;
use std::sync::Arc;
use tauri::Manager;

fn resolve_dirs(app: &tauri::App) -> (PathBuf, PathBuf) {
    // Prod: bundled under the resource dir. Dev: relative to the crate.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let res = app.path().resource_dir().ok();

    let data_dir = res
        .as_ref()
        .map(|r| r.join("data"))
        .filter(|p| p.exists())
        .unwrap_or_else(|| manifest.join("../data"));

    let frontend_dir = res
        .as_ref()
        .map(|r| r.join("frontend"))
        .filter(|p| p.exists())
        .unwrap_or_else(|| manifest.join("../frontend/.output/public"));

    (data_dir, frontend_dir)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let (data_dir, frontend_dir) = resolve_dirs(app);
            save::set_data_dir(data_dir.clone());

            let initial_currency: u32 = std::env::var("TSM_CURRENCY").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
            let state = server::AppState {
                data_dir,
                frontend_dir,
                currency: Arc::new(AtomicU32::new(initial_currency)),
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
