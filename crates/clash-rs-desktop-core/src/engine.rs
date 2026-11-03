//! App-level connect/disconnect orchestration.
//!
//! The shell (Tauri) talks to this one type. It owns the core — either as a
//! local child process (system-proxy mode, and TUN on Linux) or through the
//! privileged Windows service (TUN) — plus the generated config and the OS
//! proxy, and guarantees the user's original proxy settings come back: on
//! disconnect, on core crash, and on the next launch if we ourselves crashed.

use std::{
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::Context as _;
use serde::Serialize;
use serde_yaml::Value;

use crate::{
    ActiveCore, AppPaths, BundledCore, ClashIpcClient, ClashIpcClientConfig, ClashProcessManager,
    CoreLaunchRequest, CoreLog, CoreState, CoreUpdateManager, ManagedCoreFiles, ProfileStore,
    SettingsStore, SystemProxyBackend, SystemProxySnapshot,
    elevate::{Elevated, run_elevated},
    generate::{Bypass, ConfigInputs, generate},
    i18n::t,
    leak::LeakDetector,
    network::LocalNetwork,
    service::{Request, Response, SERVICE_VERSION, ServiceClient},
    settings::ConnectMode,
    system_proxy::PlatformSystemProxy,
};

/// Points the engine at a local `clash-rs` build instead of downloading one.
pub const CORE_OVERRIDE_ENV: &str = "CLASH_RS_DESKTOP_CORE";

/// Returned (inside `anyhow::Error`) when TUN needs a one-time privileged
/// install: the Windows service, or Linux capabilities on the core.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NeedsServiceInstall {
    Missing,
    Outdated,
}

impl std::fmt::Display for NeedsServiceInstall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Missing => t(
                "增强模式需要先完成一次授权安装",
                "Enhanced Mode needs a one-time authorized install",
            ),
            Self::Outdated => t(
                "增强模式组件需要更新",
                "The Enhanced Mode component needs an update",
            ),
        })
    }
}

impl std::error::Error for NeedsServiceInstall {}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreUpdateInfo {
    pub current: Option<String>,
    pub latest: String,
    pub available: bool,
}

enum Backend {
    /// `bool`: whether this local core runs TUN (Linux).
    Local(ClashProcessManager, bool),
    /// The core runs inside the Windows service.
    Service,
}

pub struct Engine {
    paths: AppPaths,
    settings: SettingsStore,
    profiles: ProfileStore,
    backend: Option<Backend>,
    controller: Option<ClashIpcClient>,
    mixed_port: Option<u16>,
    service: ServiceClient,
    /// Core output + app events, mirrored to `logs/core.log`.
    log: CoreLog,
    /// The machine's own DNS setup, captured before we change anything.
    local: LocalNetwork,
    /// Watches for apps bypassing the system proxy (system-proxy mode).
    leaks: Option<LeakDetector>,
    /// The config the core is running, for diagnostics.
    running_config: Option<Value>,
    /// Core shipped with the installer, installed on first run.
    bundled_core: Option<BundledCore>,
}

impl Engine {
    pub fn load(paths: AppPaths) -> anyhow::Result<Self> {
        for dir in [
            &paths.program_data,
            &paths.core_dir,
            &paths.runtime_dir,
            &paths.profiles_dir,
        ] {
            fs::create_dir_all(dir)
                .with_context(|| format!("failed to create {}", dir.display()))?;
        }

        let engine = Self {
            settings: SettingsStore::load(paths.settings_path())?,
            profiles: ProfileStore::load(paths.profile_manifest_path())?,
            backend: None,
            controller: None,
            mixed_port: None,
            service: ServiceClient::default(),
            log: CoreLog::with_file(&paths.logs_dir().join("core.log")),
            local: LocalNetwork::detect(),
            leaks: None,
            running_config: None,
            bundled_core: None,
            paths,
        };

        // A snapshot left behind means we crashed while connected: the OS
        // still points at a dead port. Put the user's settings back first.
        engine.restore_system_proxy()?;
        Ok(engine)
    }

    pub fn paths(&self) -> &AppPaths {
        &self.paths
    }

    pub fn settings(&self) -> &SettingsStore {
        &self.settings
    }

    pub fn settings_mut(&mut self) -> &mut SettingsStore {
        &mut self.settings
    }

    pub fn profiles(&self) -> &ProfileStore {
        &self.profiles
    }

    pub fn profiles_mut(&mut self) -> &mut ProfileStore {
        &mut self.profiles
    }

    pub fn mixed_port(&self) -> Option<u16> {
        self.mixed_port
    }

    pub fn is_connected(&self) -> bool {
        self.backend.is_some()
    }

    /// Which mode the live connection uses (may differ from settings until
    /// the next reconnect).
    pub fn connected_mode(&self) -> Option<ConnectMode> {
        match self.backend {
            Some(Backend::Local(_, true)) | Some(Backend::Service) => Some(ConnectMode::Tun),
            Some(Backend::Local(_, false)) => Some(ConnectMode::SystemProxy),
            None => None,
        }
    }

    /// Controller client for the running core, if connected.
    pub fn controller(&self) -> Option<ClashIpcClient> {
        self.controller.clone()
    }

    pub async fn core_log_tail(&self, count: usize) -> Vec<String> {
        match &self.backend {
            Some(Backend::Service) => match self.service.send(&Request::Logs { count }).await {
                Ok(Response::Logs { lines }) => lines,
                _ => self.log.tail(count),
            },
            // Local core and app events share `self.log`; it also keeps the
            // last session's lines after disconnecting.
            _ => self.log.tail(count),
        }
    }

    /// Folder with the log files for the current/last connection mode.
    pub fn logs_dir(&self) -> PathBuf {
        if matches!(self.backend, Some(Backend::Service))
            || (self.backend.is_none() && self.settings.settings().connect_mode == ConnectMode::Tun)
        {
            let service = crate::service::ServicePaths::system().data_dir.join("logs");
            if service.exists() {
                return service;
            }
        }
        self.paths.logs_dir()
    }

    /// Record an app-side event in the log (and its file).
    pub fn note(&self, message: &str) {
        self.log.note(message);
    }

    /// Programs seen going around the system proxy since connecting, minus
    /// ones the user chose to ignore. Empty outside system-proxy mode.
    pub fn leaking_apps(&self) -> Vec<String> {
        let Some(detector) = &self.leaks else {
            return Vec::new();
        };
        let ignored = &self.settings.settings().ignored_leak_apps;
        detector
            .scan()
            .into_iter()
            .filter(|app| !ignored.iter().any(|i| i.eq_ignore_ascii_case(app)))
            .collect()
    }

    // ---- core binary --------------------------------------------------------

    fn installed_core(&self) -> Option<ActiveCore> {
        if let Some(path) = std::env::var_os(CORE_OVERRIDE_ENV).map(PathBuf::from) {
            return path.exists().then(|| local_core(path));
        }
        let files = ManagedCoreFiles::from_paths(&self.paths);
        ActiveCore::load(&files.active_metadata)
            .ok()
            .filter(|active| active.binary_path.exists())
    }

    /// Use the core shipped with the installer on first run.
    pub fn set_bundled_core(&mut self, core: Option<BundledCore>) {
        self.bundled_core = core;
    }

    /// The installed core. On first use: the one shipped with the installer,
    /// else the latest release downloaded from GitHub.
    pub async fn ensure_core(
        &self,
        on_step: &(dyn Fn(&str) + Send + Sync),
    ) -> anyhow::Result<ActiveCore> {
        if let Some(core) = self.installed_core() {
            return Ok(core);
        }
        anyhow::ensure!(
            std::env::var_os(CORE_OVERRIDE_ENV).is_none(),
            "{CORE_OVERRIDE_ENV} points at a missing file"
        );

        let files = ManagedCoreFiles::from_paths(&self.paths);
        if let Some(bundled) = &self.bundled_core {
            match bundled.install(&files) {
                Ok(core) => {
                    self.log
                        .note(&format!("using bundled core {}", core.version));
                    return Ok(core);
                }
                // Fall back to downloading rather than failing the connection.
                Err(err) => self.log.note(&format!("bundled core unusable: {err:#}")),
            }
        }

        on_step(t(
            "首次使用，正在下载内核…",
            "First run: downloading the core…",
        ));
        let manager = CoreUpdateManager::new();
        let check = manager
            .check_latest(&self.settings.settings().updates, None)
            .await
            .context(t(
                "无法获取内核版本信息，请检查网络",
                "Couldn't get core version info. Check your network.",
            ))?;
        manager
            .download_and_activate(&check.candidate, &ManagedCoreFiles::from_paths(&self.paths))
            .await
            .context(t(
                "内核下载失败，请检查网络",
                "Core download failed. Check your network.",
            ))
    }

    pub fn core_version(&self) -> Option<String> {
        self.installed_core().map(|core| core.version)
    }

    pub async fn check_core_update(&self) -> anyhow::Result<CoreUpdateInfo> {
        let current = self.installed_core();
        let check = CoreUpdateManager::new()
            .check_latest(&self.settings.settings().updates, current.as_ref())
            .await
            .context(t(
                "无法获取内核版本信息，请检查网络",
                "Couldn't get core version info. Check your network.",
            ))?;
        let latest = crate::binary_update::version_from_tag(&check.candidate.release_tag);
        Ok(CoreUpdateInfo {
            available: current.as_ref().is_none_or(|core| core.version != latest),
            current: current.map(|core| core.version),
            latest,
        })
    }

    /// Download the latest core; the caller reconnects to start using it.
    pub async fn update_core(&mut self) -> anyhow::Result<String> {
        anyhow::ensure!(
            std::env::var_os(CORE_OVERRIDE_ENV).is_none(),
            "{}",
            crate::tr!(
                "正在使用 {CORE_OVERRIDE_ENV} 指定的内核",
                "Using the core set by {CORE_OVERRIDE_ENV}"
            )
        );
        let settings = self.settings.settings().updates.clone();
        let manager = CoreUpdateManager::new();
        let check = manager.check_latest(&settings, None).await?;
        let active = manager
            .download_and_activate(&check.candidate, &ManagedCoreFiles::from_paths(&self.paths))
            .await
            .context(t(
                "内核下载失败，请检查网络",
                "Core download failed. Check your network.",
            ))?;
        // Keep the service's private copy in step, if it is installed.
        if self.service_version().await.is_some() {
            let _ = self
                .service
                .send(&Request::UpdateCore {
                    channel: settings.channel,
                })
                .await;
        }
        Ok(active.version)
    }

    // ---- config ----------------------------------------------------------------

    fn build_config(
        &self,
        port: u16,
        tun: bool,
        controller_ipc: Option<&str>,
    ) -> anyhow::Result<Value> {
        let profile = self
            .profiles
            .active_profile()
            .ok_or_else(|| anyhow::anyhow!(t("还没有添加订阅", "No subscription added yet")))?;
        let subscription = read_yaml(&profile.source_path)?.ok_or_else(|| {
            anyhow::anyhow!(t(
                "订阅文件丢失，请更新订阅",
                "The subscription file is missing. Update the subscription."
            ))
        })?;

        let mut overrides = Vec::new();
        for path in [
            Some(self.paths.global_override_path()),
            profile.profile_override_path.clone(),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(layer) =
                read_yaml(&path).context(t("自定义配置格式有误", "Custom config is malformed"))?
            {
                overrides.push(layer);
            }
        }

        // Other subscriptions merged in: their nodes join this one's groups.
        let mut extra_subscriptions = Vec::new();
        for merged in self.profiles.merged_profiles() {
            match read_yaml(&merged.source_path) {
                Ok(Some(config)) => extra_subscriptions.push((merged.name.clone(), config)),
                _ => self.log.note(&crate::tr!(
                    "合并订阅「{}」时读取失败，已跳过",
                    "Couldn't read merged subscription \"{}\"; skipped",
                    merged.name
                )),
            }
        }

        let settings = self.settings.settings();
        Ok(generate(ConfigInputs {
            subscription,
            extra_subscriptions,
            overrides,
            custom_rules: &settings.custom_rules,
            bypass: &settings.bypass,
            local: &self.local,
            mixed_port: port,
            mode: settings.routing_mode.as_clash(),
            tun_ipv6: settings.tun_ipv6,
            tun,
            controller_ipc,
            controller_http: None,
        }))
    }

    fn local_launch(&self, core: &ActiveCore, config_path: &Path) -> CoreLaunchRequest {
        CoreLaunchRequest {
            binary_path: core.binary_path.clone(),
            runtime_dir: self.paths.runtime_dir.clone(),
            config_path: config_path.to_owned(),
            core_pipe_path: local_controller_path(&self.paths),
        }
    }

    /// Write `config` to the live config path, validating it first so a bad
    /// config never replaces a good one.
    fn write_local_config(&self, core: &ActiveCore, config: &Value) -> anyhow::Result<PathBuf> {
        let path = self.paths.runtime_dir.join("generated.yaml");
        let staging = self.paths.runtime_dir.join("generated.yaml.new");
        fs::write(&staging, serde_yaml::to_string(config)?)?;
        let check = self.local_launch(core, &staging);
        if let Err(error) = tokio::task::block_in_place(|| check.validate_config()) {
            let _ = fs::remove_file(&staging);
            anyhow::bail!(crate::tr!(
                "配置有误，无法启动：{}",
                "Config error, can't start: {}",
                first_error_line(&error)
            ));
        }
        fs::rename(&staging, &path)?;
        Ok(path)
    }

    // ---- connect ---------------------------------------------------------------

    pub async fn connect(&mut self, on_step: &(dyn Fn(&str) + Send + Sync)) -> anyhow::Result<()> {
        if self.is_connected() {
            return Ok(());
        }
        anyhow::ensure!(
            self.profiles.active_profile().is_some(),
            "{}",
            t("还没有添加订阅", "No subscription added yet")
        );

        let core = self.ensure_core(on_step).await?;
        on_step(t("正在连接…", "Connecting…"));
        let port = pick_port(self.settings.settings().system_proxy.target.port);
        // Capture the machine's DNS setup while it is still untouched.
        self.local = LocalNetwork::detect();

        let mode = self.settings.settings().connect_mode;
        let result = self.connect_with(mode, &core, port).await;
        match &result {
            Ok(()) => {
                self.mixed_port = Some(port);
                self.restore_selections().await;
                self.note(match mode {
                    ConnectMode::SystemProxy => t("已连接（系统代理）", "Connected (system proxy)"),
                    ConnectMode::Tun => t("已连接（增强模式）", "Connected (Enhanced Mode)"),
                });
            }
            Err(error) => self.note(&crate::tr!(
                "连接失败：{error}",
                "Connection failed: {error}"
            )),
        }
        result
    }

    async fn connect_with(
        &mut self,
        mode: ConnectMode,
        core: &ActiveCore,
        port: u16,
    ) -> anyhow::Result<()> {
        match mode {
            ConnectMode::SystemProxy => {
                self.start_local(core, port, false).await?;
                if let Err(error) = self.apply_system_proxy(port) {
                    let _ = self.disconnect().await;
                    return Err(
                        error.context(t("无法设置系统代理", "Couldn't set the system proxy"))
                    );
                }
                let core_pid = match &mut self.backend {
                    Some(Backend::Local(process, _)) => match process.state() {
                        CoreState::Running { pid } => Some(pid),
                        _ => None,
                    },
                    _ => None,
                };
                let ignore = std::iter::once(std::process::id())
                    .chain(core_pid)
                    .collect();
                self.leaks = Some(LeakDetector::start(ignore));
                Ok(())
            }
            // The privileged service owns TUN on every platform: on Linux,
            // capabilities on the core alone aren't enough (clash-rs runs
            // `ip` to set up routes, and caps don't pass to that child).
            ConnectMode::Tun => self.start_service(port).await,
        }
    }

    async fn start_local(&mut self, core: &ActiveCore, port: u16, tun: bool) -> anyhow::Result<()> {
        let ipc = local_controller_path(&self.paths);
        let config = self.build_config(port, tun, Some(&ipc))?;
        self.running_config = Some(config.clone());
        let config_path = self.write_local_config(core, &config)?;

        let mut process = ClashProcessManager::with_log(self.log.clone());
        process.start_direct(&self.local_launch(core, &config_path))?;
        let client = controller_client(&ipc);
        if let Err(error) = wait_ready(&client, Duration::from_secs(15)).await {
            let reason = process
                .log()
                .last_error()
                .unwrap_or_else(|| error.to_string());
            let _ = process.stop();
            anyhow::bail!(crate::tr!(
                "内核启动失败：{reason}",
                "The core failed to start: {reason}"
            ));
        }
        if tun && let Err(reason) = process.log().wait_for_tun(Duration::from_secs(5)).await {
            let _ = process.stop();
            if reason.contains("permission denied") {
                // Capabilities missing or not honoured: ask to authorise again.
                return Err(NeedsServiceInstall::Missing.into());
            }
            anyhow::bail!(crate::tr!(
                "增强模式启动失败：{reason}",
                "Enhanced Mode failed to start: {reason}"
            ));
        }
        self.backend = Some(Backend::Local(process, tun));
        self.controller = Some(client);
        Ok(())
    }

    async fn service_version(&self) -> Option<String> {
        match self.service.send(&Request::Ping).await {
            Ok(Response::Pong { version }) => Some(version),
            _ => None,
        }
    }

    async fn start_service(&mut self, port: u16) -> anyhow::Result<()> {
        match self.service_version().await {
            None => return Err(NeedsServiceInstall::Missing.into()),
            Some(version) if version != SERVICE_VERSION => {
                return Err(NeedsServiceInstall::Outdated.into());
            }
            Some(_) => {}
        }
        let value = self.build_config(port, true, None)?;
        let config = serde_yaml::to_string(&value)?;
        self.running_config = Some(value);
        match self.service.send(&Request::Start { config }).await? {
            Response::Ok => {}
            Response::Error { message } => anyhow::bail!(crate::tr!(
                "增强模式启动失败：{message}",
                "Enhanced Mode failed to start: {message}"
            )),
            other => anyhow::bail!("unexpected service response: {other:?}"),
        }
        self.backend = Some(Backend::Service);
        let client = controller_client(crate::service::CORE_PIPE);
        // With TUN, every lookup goes through the core; its encrypted DNS
        // needs a moment to come up. Don't report "connected" before a
        // page could actually load.
        wait_dns(&client, Duration::from_secs(8)).await;
        self.controller = Some(client);
        Ok(())
    }

    /// The service may still be running a TUN core from before the app
    /// restarted (or was updated); pick it up instead of starting a second
    /// one. Deliberately not version-gated: status/stop work on any service
    /// version, and ignoring a running tunnel would leave the user's traffic
    /// captured with the UI saying "disconnected". Only *starting* requires
    /// the current service version.
    pub async fn adopt_running_service(&mut self) -> bool {
        if self.is_connected() {
            return false;
        }
        match self.service.send(&Request::Status).await {
            Ok(Response::Status { status }) if status.running => {
                self.backend = Some(Backend::Service);
                self.controller = Some(controller_client(crate::service::CORE_PIPE));
                true
            }
            _ => false,
        }
    }

    pub async fn disconnect(&mut self) -> anyhow::Result<()> {
        // Restore the network first: even if stopping the core fails, the
        // user must not be left pointing at a dead proxy.
        let restored = self.restore_system_proxy();
        self.controller = None;
        self.mixed_port = None;
        self.leaks = None;
        if self.backend.is_some() {
            self.note(t("已断开", "Disconnected"));
        }
        match self.backend.take() {
            Some(Backend::Local(mut process, _)) => {
                process.stop()?;
            }
            Some(Backend::Service) => {
                if let Response::Error { message } = self.service.send(&Request::Stop).await? {
                    anyhow::bail!(message);
                }
            }
            None => {}
        }
        restored
    }

    /// Apply config changes to the running core without dropping the
    /// connection. No-op when disconnected.
    pub async fn reload(&mut self) -> anyhow::Result<()> {
        let Some(port) = self.mixed_port else {
            return Ok(());
        };
        match &self.backend {
            Some(Backend::Local(..)) => {
                let core = self
                    .installed_core()
                    .context(t("内核文件丢失", "The core file is missing"))?;
                let tun = self.connected_mode() == Some(ConnectMode::Tun);
                let config =
                    self.build_config(port, tun, Some(&local_controller_path(&self.paths)))?;
                self.running_config = Some(config.clone());
                self.write_local_config(&core, &config)?;
                // `{}` = reload the startup config file we just replaced. A
                // timeout surfaces as an error; the shell then reconnects.
                let client = self
                    .controller
                    .clone()
                    .context(t("尚未连接", "Not connected"))?;
                tokio::time::timeout(
                    Duration::from_secs(10),
                    client.put_json::<_, serde_json::Value>(
                        "/configs?force=true",
                        &serde_json::json!({}),
                    ),
                )
                .await
                .context(t(
                    "内核重新加载配置超时",
                    "The core timed out reloading the config",
                ))??;
            }
            Some(Backend::Service) => {
                let value = self.build_config(port, true, None)?;
                let config = serde_yaml::to_string(&value)?;
                self.running_config = Some(value);
                if let Response::Error { message } =
                    self.service.send(&Request::Reload { config }).await?
                {
                    anyhow::bail!(crate::tr!("配置有误：{message}", "Config error: {message}"));
                }
            }
            None => {}
        }
        self.restore_selections().await;
        Ok(())
    }

    /// Remember a node the user picked in a selector group of the active
    /// profile, so it survives restarts, mode switches and reloads.
    pub fn remember_selection(&mut self, group: &str, node: &str) -> anyhow::Result<()> {
        let Some(profile) = self.profiles.active_profile().map(|p| p.id.0.clone()) else {
            return Ok(());
        };
        let store = &mut self.settings;
        store
            .settings_mut()
            .selections
            .entry(profile)
            .or_default()
            .insert(group.to_owned(), node.to_owned());
        store.save()
    }

    /// Re-apply remembered picks. Groups or nodes that no longer exist
    /// (subscription changed) are skipped silently.
    async fn restore_selections(&self) {
        let (Some(client), Some(profile)) = (&self.controller, self.profiles.active_profile())
        else {
            return;
        };
        let Some(picks) = self.settings.settings().selections.get(&profile.id.0) else {
            return;
        };
        for (group, node) in picks {
            let _ = client.select_proxy(group, node).await;
        }
    }

    /// Poll from the shell. If the core died while we were connected, clean
    /// up and return a user-facing reason.
    pub async fn check_health(&mut self) -> Option<String> {
        let reason = match self.backend.as_mut()? {
            Backend::Local(process, _) => match process.state() {
                CoreState::Running { .. } => return None,
                CoreState::Failed { message } => process.log().last_error().unwrap_or(crate::tr!(
                    "内核已退出（{message}）",
                    "The core exited ({message})"
                )),
                _ => process
                    .log()
                    .last_error()
                    .unwrap_or_else(|| t("内核已退出", "The core exited").to_owned()),
            },
            Backend::Service => match self.service.send(&Request::Status).await {
                Ok(Response::Status { status }) if status.running => return None,
                Ok(Response::Status { status }) => status
                    .last_error
                    .unwrap_or_else(|| t("内核已退出", "The core exited").to_owned()),
                // The service itself is gone (stopped/uninstalled).
                _ => t("增强模式服务已停止", "The Enhanced Mode service stopped").to_owned(),
            },
        };
        let _ = self.disconnect().await;
        Some(crate::tr!(
            "连接意外断开：{reason}",
            "Connection lost: {reason}"
        ))
    }

    // ---- privileged install ----------------------------------------------------

    /// One-time privileged setup for TUN. `service_exe` is the service binary
    /// shipped with the app (Windows only).
    pub async fn install_tun_support(&self, service_exe: &Path) -> anyhow::Result<()> {
        anyhow::ensure!(
            service_exe.exists(),
            "{}",
            crate::tr!(
                "找不到增强模式组件：{}",
                "Enhanced Mode component not found: {}",
                service_exe.display()
            )
        );
        let core = self.ensure_core(&|_| {}).await?;
        let (exe, args) = if cfg!(windows) {
            (
                service_exe.to_owned(),
                format!("--install --core \"{}\"", core.binary_path.display()),
            )
        } else {
            // An AppImage's files live on a per-user FUSE mount root can't
            // read; hand pkexec a copy in our own data dir instead.
            let staged = self.paths.runtime_dir.join("clash-rs-desktop-service");
            fs::copy(service_exe, &staged).context(t(
                "无法准备增强模式组件",
                "Couldn't prepare the Enhanced Mode component",
            ))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                fs::set_permissions(&staged, fs::Permissions::from_mode(0o755))?;
            }
            // pkexec passes argv verbatim (split on whitespace by run_elevated).
            (
                staged,
                format!("--install --core {}", core.binary_path.display()),
            )
        };
        match tokio::task::spawn_blocking(move || run_elevated(&exe, &args)).await?? {
            Elevated::Exited(0) => {}
            Elevated::Cancelled => anyhow::bail!(t("已取消授权", "Authorization cancelled")),
            Elevated::Exited(code) => anyhow::bail!(crate::tr!(
                "安装失败（错误码 {code}）",
                "Install failed (error code {code})"
            )),
        }
        // The service starts asynchronously; give it a moment to listen.
        for _ in 0..50 {
            if self.service_version().await.is_some() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        anyhow::bail!(t(
            "增强模式服务没有启动",
            "The Enhanced Mode service didn't start"
        ))
    }

    // ---- diagnostics -----------------------------------------------------------

    /// Run the checks. Read-only. `thorough` adds delay tests and route
    /// lookups (a few seconds, some traffic); the background watchdog uses
    /// the quick variant.
    pub async fn diagnose(&self, thorough: bool) -> Vec<crate::diagnose::Finding> {
        use crate::diagnose::{Context, assess, collect};

        let mut ctx = Context {
            connected_tun: self.connected_mode().map(|mode| mode == ConnectMode::Tun),
            mixed_port: self.mixed_port,
            direct_mode: self.settings.settings().routing_mode == crate::RoutingMode::Direct,
            ..Default::default()
        };
        if let Some(config) = &self.running_config {
            ctx.proxy_servers = config
                .get("proxies")
                .and_then(Value::as_sequence)
                .into_iter()
                .flatten()
                .filter_map(|p| p.get("server").and_then(Value::as_str))
                .map(str::to_owned)
                .collect();
        }

        if let Some(client) = self.controller.as_ref().filter(|_| thorough) {
            const PROBE: &str = "http://www.gstatic.com/generate_204";
            let delay = |reply: anyhow::Result<serde_json::Value>| {
                reply
                    .ok()
                    .and_then(|v| v.get("delay").and_then(|d| d.as_u64()))
                    .filter(|d| *d > 0)
            };
            let main = self
                .running_config
                .as_ref()
                .and_then(crate::generate::main_select_group);
            let (direct, proxied) =
                tokio::join!(client.test_proxy_delay("DIRECT", PROBE, 5000), async {
                    match &main {
                        Some(group) => Some(client.test_proxy_delay(group, PROBE, 5000).await),
                        None => None,
                    }
                });
            ctx.direct_delay = Some(delay(direct));
            ctx.proxy_delay = proxied.map(delay);

            // A loop shows up as connections that came in through TUN but
            // are headed for a proxy server: the core's own outbound
            // traffic being routed back into itself.
            if ctx.connected_tun == Some(true)
                && let Ok(connections) = client.get_connections().await
            {
                ctx.looping_connections = connections
                    .connections
                    .iter()
                    .filter(|conn| {
                        let meta = &conn["metadata"];
                        let inbound = meta["type"]
                            .as_str()
                            .unwrap_or_default()
                            .to_ascii_lowercase();
                        let host = meta["host"].as_str().unwrap_or_default();
                        let ip = meta["destinationIP"].as_str().unwrap_or_default();
                        inbound.contains("tun")
                            && ctx
                                .proxy_servers
                                .iter()
                                .any(|server| server == host || server == ip)
                    })
                    .count();
            }
        }

        let port = self.mixed_port;
        // Route/adapter lookups only when thorough, or under TUN where routes
        // are the main thing that can break.
        let routes = thorough || ctx.connected_tun == Some(true);
        let sys = tokio::task::spawn_blocking(move || collect(port, routes))
            .await
            .unwrap_or_default();
        let findings = assess(&ctx, &sys);
        if thorough && let Some(worst) = findings.first() {
            self.note(&crate::tr!("网络诊断：{}", "Diagnostics: {}", worst.title));
        }
        findings
    }

    /// Apply a diagnostics remedy that doesn't need a reconnect (the shell
    /// handles `Reconnect` and `EnableTun` itself).
    pub async fn apply_fix(&mut self, fix: crate::diagnose::Fix) -> anyhow::Result<()> {
        use crate::diagnose::Fix;
        match fix {
            Fix::ReapplySystemProxy => {
                let port = self.mixed_port.context(t("尚未连接", "Not connected"))?;
                self.apply_system_proxy(port)
            }
            Fix::RestoreSystemProxy => {
                if self.paths.system_proxy_snapshot_path().exists() {
                    return self.restore_system_proxy();
                }
                // No snapshot of ours: just switch off the dead local proxy,
                // keeping its address so the user's own setup isn't lost.
                let backend = PlatformSystemProxy;
                let mut snapshot = backend.snapshot()?;
                snapshot.enabled = false;
                backend.restore(&snapshot)
            }
            Fix::FlushDns => match self.service.send(&Request::FlushDns).await {
                Ok(Response::Ok) => Ok(()),
                _ => anyhow::bail!(t(
                    "清理 DNS 缓存需要增强模式组件",
                    "Flushing the DNS cache needs the Enhanced Mode component"
                )),
            },
            Fix::Reconnect if !self.is_connected() => {
                // Leftover tunnel while "disconnected": stop it.
                if let Response::Error { message } = self.service.send(&Request::Stop).await? {
                    anyhow::bail!(message);
                }
                Ok(())
            }
            Fix::EnableTunIpv6 => {
                // The shell reconnects so the device gets its IPv6 address.
                let store = &mut self.settings;
                store.settings_mut().tun_ipv6 = true;
                store.save()
            }
            Fix::Reconnect | Fix::EnableTun => Ok(()),
        }
    }

    // ---- system proxy ----------------------------------------------------------

    fn apply_system_proxy(&self, port: u16) -> anyhow::Result<()> {
        let backend = PlatformSystemProxy;
        let snapshot_path = self.paths.system_proxy_snapshot_path();
        // Keep the oldest snapshot: if one exists it holds the user's real
        // settings, while the live values may already be ours.
        if !snapshot_path.exists() {
            let snapshot = backend.snapshot()?;
            fs::write(&snapshot_path, serde_json::to_vec(&snapshot)?)?;
        }

        let mut target = self.settings.settings().system_proxy.target.clone();
        // Local names and the user's "不走代理" list skip the proxy entirely.
        let bypass = Bypass::new(&self.settings.settings().bypass, &self.local);
        for entry in bypass.wininet_entries() {
            if !target.bypass.contains(&entry) {
                target
                    .bypass
                    .insert(target.bypass.len().saturating_sub(1), entry);
            }
        }
        target.port = port;
        backend.apply(&target)
    }

    fn restore_system_proxy(&self) -> anyhow::Result<()> {
        let snapshot_path = self.paths.system_proxy_snapshot_path();
        let Ok(raw) = fs::read(&snapshot_path) else {
            return Ok(());
        };
        let snapshot: SystemProxySnapshot = match serde_json::from_slice(&raw) {
            Ok(snapshot) => snapshot,
            // Unreadable snapshot: turning the proxy off is the safe default.
            Err(_) => SystemProxySnapshot {
                enabled: false,
                server: None,
                bypass: None,
            },
        };
        PlatformSystemProxy.restore(&snapshot)?;
        fs::remove_file(&snapshot_path)?;
        Ok(())
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // Best effort without async: restore the proxy and stop a local core.
        // A service-owned TUN core keeps running and is re-adopted next launch.
        let _ = self.restore_system_proxy();
        if let Some(Backend::Local(mut process, _)) = self.backend.take() {
            let _ = process.stop();
        }
    }
}

fn controller_client(endpoint: &str) -> ClashIpcClient {
    ClashIpcClient::new(ClashIpcClientConfig {
        pipe_path: endpoint.to_owned(),
        auth_secret: None,
    })
}

/// Unique per app run so a stale endpoint from a crashed run can't shadow
/// the new core's controller.
fn local_controller_path(paths: &AppPaths) -> String {
    if cfg!(windows) {
        format!(r"\\.\pipe\choclite-{}", std::process::id())
    } else {
        paths
            .runtime_dir
            .join(format!("controller-{}.sock", std::process::id()))
            .display()
            .to_string()
    }
}

async fn wait_ready(client: &ClashIpcClient, timeout: Duration) -> anyhow::Result<()> {
    let started = std::time::Instant::now();
    loop {
        match client.get_version().await {
            Ok(_) => return Ok(()),
            Err(error) if started.elapsed() > timeout => return Err(error),
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}

/// Best effort: return once the core resolves a well-known name (or after
/// `timeout`; a slow network shouldn't block connecting forever).
async fn wait_dns(client: &ClashIpcClient, timeout: Duration) {
    let started = std::time::Instant::now();
    while started.elapsed() < timeout {
        let answered = client
            .query_dns("www.gstatic.com", "A")
            .await
            .is_ok_and(|reply| {
                reply
                    .get("Answer")
                    .and_then(|a| a.as_array())
                    .is_some_and(|a| !a.is_empty())
            });
        if answered {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn read_yaml(path: &Path) -> anyhow::Result<Option<Value>> {
    let Ok(text) = fs::read_to_string(path) else {
        return Ok(None);
    };
    if text.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(serde_yaml::from_str(&text)?))
}

/// The preferred port if free, otherwise any free loopback port.
fn pick_port(preferred: u16) -> u16 {
    if TcpListener::bind(("127.0.0.1", preferred)).is_ok() {
        return preferred;
    }
    TcpListener::bind(("127.0.0.1", 0))
        .and_then(|listener| listener.local_addr())
        .map(|addr| addr.port())
        .unwrap_or(preferred)
}

/// The useful part of `clash-rs --test-config` output, e.g.
/// `configuration file c.yaml test failed: invalid config: invalid config:
/// unsupported rule type: X` → `unsupported rule type: X`.
fn first_error_line(error: &anyhow::Error) -> String {
    let text = format!("{error:#}");
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line.starts_with("clash-rs config validation")
                && !matches!(*line, "stdout:" | "stderr:")
        })
        .collect();
    let line = lines
        .iter()
        .find(|line| line.contains("test failed:"))
        .or_else(|| lines.last())
        .copied()
        .unwrap_or(t("未知错误", "Unknown error"));
    let detail = line
        .rsplit_once("invalid config: ")
        .or_else(|| line.rsplit_once("test failed: "))
        .map_or(line, |(_, rest)| rest);
    detail.trim_start_matches(": ").trim().to_owned()
}

fn local_core(path: PathBuf) -> ActiveCore {
    ActiveCore {
        channel: crate::UpdateChannel::Stable,
        release_tag: "local".to_owned(),
        version: "local".to_owned(),
        commit: None,
        asset_name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        binary_path: path,
        binary_sha256: None,
        updated_at: time::OffsetDateTime::now_utc(),
    }
}

#[cfg(test)]
mod tests {
    use super::{first_error_line, pick_port};

    #[test]
    fn validation_errors_are_trimmed_to_the_cause() {
        let error = anyhow::anyhow!(
            "clash-rs config validation failed with exit code: 1\nstdout:\n\nstderr:\nconfiguration file C:\\x\\c.yaml test failed: invalid config: invalid config: unsupported rule type: BOGUS-RULE"
        );
        assert_eq!(
            first_error_line(&error),
            "unsupported rule type: BOGUS-RULE"
        );

        let yaml_error = anyhow::anyhow!(
            "clash-rs config validation failed\nstderr:\nrules:\n  - x\n\n: duplicate entry with key \"rules\""
        );
        assert_eq!(
            first_error_line(&yaml_error),
            "duplicate entry with key \"rules\""
        );
    }

    #[test]
    fn pick_port_avoids_a_taken_port() {
        let taken = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = taken.local_addr().unwrap().port();
        assert_ne!(pick_port(port), port);
    }

    #[test]
    fn last_error_ignores_info_and_strips_prefixes() {
        let lines = [
            "26-11-02 15:46:29 ERROR clash_lib::app::inbound: bind failed: address in use",
            "26-11-02 15:46:30 INFO dispatch_stream: matched [TCP] a -> b",
        ]
        .map(str::to_owned);
        assert_eq!(
            crate::process::last_error(&lines).as_deref(),
            Some("bind failed: address in use")
        );
        let info_only = ["26-11-02 15:46:30 INFO all good".to_owned()];
        assert_eq!(crate::process::last_error(&info_only), None);
    }
}
