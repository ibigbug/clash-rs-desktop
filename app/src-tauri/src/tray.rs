//! System tray: one-click toggle, status line, show/quit.

use tauri::{
    AppHandle, Manager, Wry,
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

use clash_rs_desktop_core::t;

use crate::state::{Phase, View};

const TRAY_ID: &str = "main";

struct TrayItems {
    status: MenuItem<Wry>,
    toggle: MenuItem<Wry>,
    show: MenuItem<Wry>,
    quit: MenuItem<Wry>,
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let status = MenuItem::with_id(
        app,
        "status",
        t("未连接", "Disconnected"),
        false,
        None::<&str>,
    )?;
    let toggle = MenuItem::with_id(app, "toggle", t("连接", "Connect"), true, None::<&str>)?;
    let show = MenuItem::with_id(
        app,
        "show",
        t("打开主界面", "Open Choclite"),
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", t("退出", "Quit"), true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &PredefinedMenuItem::separator(app)?,
            &toggle,
            &show,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip(format!("Choclite · {}", t("未连接", "Disconnected")))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "toggle" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = crate::commands::toggle_flow(&app).await;
                });
            }
            "show" => show_main_window(app),
            "quit" => crate::quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });
    builder.icon(tray_icon(false)).build(app)?;

    app.manage(TrayItems {
        status,
        toggle,
        show,
        quit,
    });
    Ok(())
}

pub fn refresh(app: &AppHandle, view: &View) {
    let Some(items) = app.try_state::<TrayItems>() else {
        return;
    };
    let (status, toggle, toggle_enabled) = match view.phase {
        Phase::Disconnected => (t("未连接", "Disconnected"), t("连接", "Connect"), true),
        Phase::Connecting => (t("正在连接…", "Connecting…"), t("连接", "Connect"), false),
        Phase::Connected => (t("已连接", "Connected"), t("断开连接", "Disconnect"), true),
        Phase::Disconnecting => (
            t("正在断开…", "Disconnecting…"),
            t("断开连接", "Disconnect"),
            false,
        ),
    };
    let _ = items.status.set_text(status);
    let _ = items.toggle.set_text(toggle);
    let _ = items.toggle.set_enabled(toggle_enabled);
    // The language can change at runtime.
    let _ = items.show.set_text(t("打开主界面", "Open Choclite"));
    let _ = items.quit.set_text(t("退出", "Quit"));
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(format!("Choclite · {status}")));
        // Colored when connected, grey otherwise — readable at a glance.
        let _ = tray.set_icon(Some(tray_icon(view.phase == Phase::Connected)));
    }
}

fn tray_icon(connected: bool) -> Image<'static> {
    let bytes: &'static [u8] = if connected {
        include_bytes!("../icons/tray-on.png")
    } else {
        include_bytes!("../icons/tray-off.png")
    };
    Image::from_bytes(bytes).expect("bundled tray icon is a valid PNG")
}

pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}
