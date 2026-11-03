mod commands;
mod nodes;
mod state;
mod tray;

use clash_rs_desktop_core::{AppPaths, BundledCore, Engine};
use tauri::{AppHandle, Manager, WindowEvent, path::BaseDirectory};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_window_state::{AppHandleExt, StateFlags};

use crate::state::AppState;

/// Window size and position survive restarts. Visibility is ours to decide
/// (start in the tray), and the window never has native decorations.
const WINDOW_STATE: StateFlags = StateFlags::SIZE
    .union(StateFlags::POSITION)
    .union(StateFlags::MAXIMIZED);

/// Passed by the autostart entry so a login launch stays in the tray.
const AUTOSTART_FLAG: &str = "--autostart";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Must be first: a second launch (or a clicked deep link) is routed
        // to the running instance.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            tray::show_main_window(app);
            commands::handle_deep_links(app, args);
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![AUTOSTART_FLAG]),
        ))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(WINDOW_STATE)
                .build(),
        )
        .setup(|app| {
            let data_dir = app.path().app_local_data_dir()?;
            let mut engine = Engine::load(AppPaths::from_program_data(data_dir))?;
            // Installers ship a core so the first connection works offline.
            let bundled = app
                .path()
                .resolve("core", BaseDirectory::Resource)
                .ok()
                .and_then(|dir| BundledCore::from_dir(&dir));
            engine.set_bundled_core(bundled);
            clash_rs_desktop_core::i18n::set_language(engine.settings().settings().language);
            let auto_connect = engine.settings().settings().auto_connect;
            app.manage(AppState::new(engine));

            tray::create(app.handle())?;
            commands::spawn_health_watch(app.handle().clone());
            commands::spawn_subscription_updater(app.handle().clone());
            commands::spawn_leak_watch(app.handle().clone());
            commands::spawn_health_check(app.handle().clone());

            // Installed builds get `choclite://` from the installer; register
            // at runtime too so portable copies work. `clash://` (what most
            // providers' "import" buttons use) is only claimed when no other
            // client owns it — never steal it from another app.
            #[cfg(all(not(debug_assertions), any(windows, target_os = "linux")))]
            {
                let _ = app.deep_link().register("choclite");
                if !clash_scheme_taken() {
                    let _ = app.deep_link().register("clash");
                }
            }
            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                commands::handle_deep_links(&handle, event.urls().iter().map(|u| u.to_string()));
            });
            // Cold start from a link: the URL is in our own argv.
            commands::handle_deep_links(app.handle(), std::env::args().skip(1));

            let from_autostart = std::env::args().any(|arg| arg == AUTOSTART_FLAG);
            if !from_autostart && let Some(window) = app.get_webview_window("main") {
                window.show()?;
            }

            let has_profile = !app.state::<AppState>().view().profiles.is_empty();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // A TUN core the service kept alive counts as connected.
                commands::adopt_existing(&handle).await;
                let connected = handle.state::<AppState>().view().phase == state::Phase::Connected;
                if auto_connect && has_profile && !connected {
                    let _ = commands::connect_flow(&handle).await;
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window keeps the connection alive in the tray.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.app_handle().save_window_state(WINDOW_STATE);
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_view,
            commands::toggle_connection,
            commands::connect,
            commands::dismiss_error,
            commands::set_connect_mode,
            commands::install_tun_support,
            commands::add_subscription,
            commands::refresh_subscription,
            commands::select_profile,
            commands::remove_profile,
            commands::set_auto_update_subscriptions,
            commands::set_routing_mode,
            commands::set_auto_connect,
            commands::add_custom_rule,
            commands::remove_custom_rule,
            commands::check_core_update,
            commands::update_core,
            commands::get_nodes,
            commands::select_node,
            commands::test_delays,
            commands::core_logs,
            commands::read_override,
            commands::write_override,
            commands::take_deep_link,
            commands::import_file,
            commands::set_merged,
            commands::set_bypass,
            commands::ignore_leak,
            commands::open_logs_dir,
            commands::run_diagnostics,
            commands::apply_fix,
            commands::set_tun_ipv6,
            commands::set_language,
            quit_app,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                restore_network(app);
            }
        });
}

/// Whether another app already handles `clash://` links.
#[cfg(all(not(debug_assertions), windows))]
fn clash_scheme_taken() -> bool {
    let output = std::process::Command::new("reg")
        .args(["query", r"HKCR\clash\shell\open\command", "/ve"])
        .output();
    match output {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
            let me = std::env::current_exe()
                .map(|p| p.display().to_string().to_ascii_lowercase())
                .unwrap_or_default();
            !me.is_empty() && !text.contains(&me)
        }
        _ => false,
    }
}

#[cfg(all(not(debug_assertions), target_os = "linux"))]
fn clash_scheme_taken() -> bool {
    std::process::Command::new("xdg-mime")
        .args(["query", "default", "x-scheme-handler/clash"])
        .output()
        .map(|out| {
            let handler = String::from_utf8_lossy(&out.stdout);
            !handler.trim().is_empty() && !handler.to_ascii_lowercase().contains("choclite")
        })
        .unwrap_or(false)
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    quit(&app);
}

/// Disconnect (restoring the OS proxy, stopping TUN) and exit.
pub fn quit(app: &AppHandle) {
    let _ = app.save_window_state(WINDOW_STATE);
    restore_network(app);
    app.exit(0);
}

fn restore_network(app: &AppHandle) {
    let state = app.state::<AppState>();
    // Wait briefly for an in-flight command. If it doesn't finish, exiting is
    // still safe: the job object kills a local core and the proxy snapshot
    // left on disk is restored on the next launch.
    tauri::async_runtime::block_on(async {
        if let Ok(mut engine) =
            tokio::time::timeout(std::time::Duration::from_secs(3), state.engine.lock()).await
        {
            let _ =
                tokio::time::timeout(std::time::Duration::from_secs(5), engine.disconnect()).await;
        }
    });
}
