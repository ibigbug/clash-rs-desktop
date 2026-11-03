//! Shared app state and the UI-facing snapshot (`View`).
//!
//! The engine sits behind an async mutex and can be held for a long time
//! (first-run core download, subscription fetch). The UI never waits on it:
//! it reads `View`, a cheap snapshot republished after every change.

use std::sync::Mutex;

use clash_rs_desktop_core::{
    ConnectMode, CustomRule, Engine, Language, NeedsServiceInstall, RoutingMode, SubscriptionInfo,
    UserProfile,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::watch;

pub struct AppState {
    pub engine: tokio::sync::Mutex<Engine>,
    view: Mutex<View>,
    /// Stops the live traffic stream when flipped to `true`.
    pub traffic_stop: Mutex<Option<watch::Sender<bool>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    Disconnected,
    Connecting,
    Connected,
    Disconnecting,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileView {
    pub id: String,
    pub name: String,
    pub remote: bool,
    pub active: bool,
    pub subscription: Option<SubscriptionInfo>,
    pub updated_at: Option<i64>,
    /// Merged into the active profile ("合并多个订阅").
    pub merged: bool,
}

/// Shown in Settings → About. Nightly builds set `CHOCLITE_BUILD` (e.g.
/// `0.1.0-nightly.20261103+abc1234`) so users can tell which commit they run.
const APP_VERSION: &str = match option_env!("CHOCLITE_BUILD") {
    Some(build) if !build.is_empty() => build,
    _ => env!("CARGO_PKG_VERSION"),
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub phase: Phase,
    /// Progress text while connecting ("正在下载内核…").
    pub step: Option<String>,
    /// Last user-facing error; cleared by the next action.
    pub error: Option<String>,
    /// Set when TUN needs the one-time privileged install.
    pub needs_install: Option<NeedsServiceInstall>,
    /// Unix millis when the current connection came up.
    pub connected_since: Option<i64>,
    pub port: Option<u16>,
    pub profiles: Vec<ProfileView>,
    pub routing_mode: RoutingMode,
    pub connect_mode: ConnectMode,
    /// Mode of the live connection (settings may have changed since).
    pub connected_mode: Option<ConnectMode>,
    pub auto_connect: bool,
    pub auto_update_subscriptions: bool,
    pub custom_rules: Vec<CustomRule>,
    /// "不走代理的地址" entered by the user.
    pub bypass: Vec<String>,
    /// Programs seen going around the system proxy (best effort).
    pub leaks: Vec<String>,
    /// Most serious problem the background check found, if any.
    pub health_issue: Option<String>,
    pub tun_ipv6: bool,
    pub language: Language,
    /// `language` resolved against the OS locale.
    pub english: bool,
    pub core_version: Option<String>,
    pub app_version: &'static str,
}

impl AppState {
    pub fn new(engine: Engine) -> Self {
        let mut view = View {
            phase: Phase::Disconnected,
            step: None,
            error: None,
            needs_install: None,
            connected_since: None,
            port: None,
            profiles: Vec::new(),
            routing_mode: RoutingMode::default(),
            connect_mode: ConnectMode::default(),
            connected_mode: None,
            auto_connect: false,
            auto_update_subscriptions: true,
            custom_rules: Vec::new(),
            bypass: Vec::new(),
            leaks: Vec::new(),
            health_issue: None,
            tun_ipv6: false,
            language: Language::default(),
            english: false,
            core_version: None,
            app_version: APP_VERSION,
        };
        sync_from_engine(&mut view, &engine);
        Self {
            engine: tokio::sync::Mutex::new(engine),
            view: Mutex::new(view),
            traffic_stop: Mutex::new(None),
        }
    }

    pub fn view(&self) -> View {
        self.view.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn update(&self, change: impl FnOnce(&mut View)) -> View {
        let mut view = self.view.lock().unwrap_or_else(|e| e.into_inner());
        change(&mut view);
        view.clone()
    }
}

/// Copy persisted engine data (profiles, settings) into the view.
pub fn sync_from_engine(view: &mut View, engine: &Engine) {
    let settings = engine.settings().settings();
    view.routing_mode = settings.routing_mode;
    view.connect_mode = settings.connect_mode;
    view.auto_connect = settings.auto_connect;
    view.auto_update_subscriptions = settings.auto_update_subscriptions;
    view.custom_rules = settings.custom_rules.clone();
    view.bypass = settings.bypass.clone();
    view.tun_ipv6 = settings.tun_ipv6;
    view.language = settings.language;
    view.english = clash_rs_desktop_core::i18n::is_english();
    view.connected_mode = engine.connected_mode();
    view.port = engine.mixed_port();
    view.core_version = engine.core_version();
    if !engine.is_connected() {
        view.leaks.clear();
        view.health_issue = None;
    }
    let store = engine.profiles();
    view.profiles = store
        .profiles()
        .iter()
        .map(|profile| ProfileView {
            merged: store.is_merged(&profile.id) && !profile.active,
            ..profile_view(profile)
        })
        .collect();
}

fn profile_view(profile: &UserProfile) -> ProfileView {
    ProfileView {
        id: profile.id.0.clone(),
        name: profile.name.clone(),
        remote: matches!(
            profile.source,
            clash_rs_desktop_core::ProfileSource::RemoteUrl { .. }
        ),
        active: profile.active,
        subscription: profile.subscription.clone(),
        updated_at: profile.updated_at,
        merged: false,
    }
}

/// Apply a change, then push the new view to the window and tray.
pub fn publish(app: &AppHandle, change: impl FnOnce(&mut View)) -> View {
    let view = app.state::<AppState>().update(change);
    let _ = app.emit("view", &view);
    crate::tray::refresh(app, &view);
    view
}
