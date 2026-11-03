pub mod binary_update;
pub mod config_layers;
pub mod diagnose;
pub mod elevate;
pub mod engine;
pub mod generate;
pub mod i18n;
pub mod ipc;
pub mod leak;
pub mod links;
pub mod network;
pub mod paths;
pub mod process;
pub mod profile;
pub mod service;
pub mod settings;
pub mod system_proxy;

pub use binary_update::{
    ActiveCore, BundledCore, CoreUpdateManager, CoreUpdateSettings, GitHubRelease,
    GitHubReleaseAsset, ManagedCoreFiles, ReleaseCandidate, UpdateChannel, UpdateCheckResult,
    UpdateDecision, WindowsArch, select_release_candidate,
};
pub use config_layers::{RuntimeConfigBuilder, apply_overlay, merge_yaml};
pub use engine::{Engine, NeedsServiceInstall};
pub use generate::{CustomRule, RuleAction, RuleKind};
pub use i18n::{Language, t};
pub use ipc::{
    ClashIpcClient, ClashIpcClientConfig, ConfigReloadRequest, ConnectionsResponse,
    ControllerConfigPatch, ControllerConfigs, ControllerEndpoint, MemoryResponse,
    ProviderCollection, ProxyCollection, ProxySelectionRequest, RulesResponse, VersionInfo,
};
pub use paths::AppPaths;
pub use process::{ClashProcessManager, CoreLaunchRequest, CoreLog, CoreState};
pub use profile::{
    FetchedSubscription, ProfileId, ProfileManifest, ProfileSource, ProfileStore, SubscriptionInfo,
    UserProfile, default_profile_yaml, fetch_subscription,
};
pub use settings::{AppSettings, ConnectMode, RoutingMode, SettingsStore, SystemProxySettings};
pub use system_proxy::{
    PlatformSystemProxy, SystemProxyBackend, SystemProxySnapshot, SystemProxyTarget,
};
