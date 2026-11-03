use std::{
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use crate::paths::AppPaths;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpdateChannel {
    Stable,
    Preview,
}

impl UpdateChannel {
    pub fn release_api_url(self) -> &'static str {
        match self {
            Self::Stable => "https://api.github.com/repos/Watfaq/clash-rs/releases/latest",
            Self::Preview => "https://api.github.com/repos/Watfaq/clash-rs/releases/tags/latest",
        }
    }

    pub fn release_api_path(self) -> &'static str {
        match self {
            Self::Stable => "/repos/Watfaq/clash-rs/releases/latest",
            Self::Preview => "/repos/Watfaq/clash-rs/releases/tags/latest",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WindowsArch {
    X64,
    Arm64,
    X86,
}

impl WindowsArch {
    /// Target triple used in release asset names. (The type predates Linux
    /// support; on Linux it picks the fully static musl builds.)
    pub fn release_name(&self) -> &'static str {
        match (self, cfg!(windows)) {
            (Self::X64, true) => "x86_64-pc-windows-msvc",
            (Self::Arm64, true) => "aarch64-pc-windows-msvc",
            (Self::X86, true) => "i686-pc-windows-msvc",
            (Self::X64, false) => "x86_64-unknown-linux-musl",
            (Self::Arm64, false) => "aarch64-unknown-linux-musl",
            (Self::X86, false) => "i686-unknown-linux-musl",
        }
    }

    pub fn detect() -> Self {
        if cfg!(target_arch = "aarch64") {
            Self::Arm64
        } else if cfg!(target_arch = "x86") {
            Self::X86
        } else {
            Self::X64
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreUpdateSettings {
    pub channel: UpdateChannel,
    pub architecture: WindowsArch,
    pub auto_check: bool,
    pub auto_download: bool,
}

impl CoreUpdateSettings {
    pub fn stable_x64() -> Self {
        Self {
            channel: UpdateChannel::Stable,
            architecture: WindowsArch::detect(),
            auto_check: true,
            auto_download: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveCore {
    pub channel: UpdateChannel,
    pub release_tag: String,
    pub version: String,
    pub commit: Option<String>,
    pub asset_name: String,
    pub binary_path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_sha256: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

impl ActiveCore {
    pub fn load(path: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let path = path.into();
        let contents = fs::read_to_string(&path)?;
        Ok(serde_json::from_str(
            contents.trim_start_matches('\u{feff}'),
        )?)
    }

    pub fn save(&self, path: impl Into<PathBuf>) -> anyhow::Result<()> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let contents = serde_json::to_string_pretty(self)?;
        fs::write(path, contents)?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ManagedCoreFiles {
    pub current_binary: PathBuf,
    pub rollback_binary: PathBuf,
    pub wintun_dll: PathBuf,
    pub active_metadata: PathBuf,
}

impl ManagedCoreFiles {
    pub fn from_paths(paths: &AppPaths) -> Self {
        Self {
            current_binary: paths
                .core_dir
                .join(format!("clash-rs{}", std::env::consts::EXE_SUFFIX)),
            rollback_binary: paths
                .core_dir
                .join(format!("clash-rs.rollback{}", std::env::consts::EXE_SUFFIX)),
            wintun_dll: paths.core_dir.join("wintun.dll"),
            active_metadata: paths.active_core_metadata_path(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct GitHubRelease {
    pub tag_name: String,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub assets: Vec<GitHubReleaseAsset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GitHubReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseCandidate {
    pub channel: UpdateChannel,
    pub release_tag: String,
    pub asset_name: String,
    pub download_url: String,
    pub size: u64,
}

impl ReleaseCandidate {
    pub fn is_zip(&self) -> bool {
        self.asset_name.to_ascii_lowercase().ends_with(".zip")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpdateDecision {
    UpToDate,
    UpdateAvailable { current: String, latest: String },
    NotInstalled { latest: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateCheckResult {
    pub decision: UpdateDecision,
    pub candidate: ReleaseCandidate,
}

#[derive(Debug, Clone)]
pub struct CoreUpdateManager {
    client: reqwest::Client,
    user_agent: String,
}

impl Default for CoreUpdateManager {
    fn default() -> Self {
        Self::new()
    }
}

impl CoreUpdateManager {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            user_agent: "clash-rs-desktop/0.1".to_owned(),
        }
    }

    pub async fn check_latest(
        &self,
        settings: &CoreUpdateSettings,
        current: Option<&ActiveCore>,
    ) -> anyhow::Result<UpdateCheckResult> {
        let release = self.fetch_release(settings.channel).await?;
        let candidate =
            select_release_candidate(settings.channel, &settings.architecture, &release)
                .with_context(|| {
                    format!(
                        "release {} does not contain a Windows asset for {}",
                        release.tag_name,
                        settings.architecture.release_name()
                    )
                })?;

        let decision = decide_update(settings.channel, current, &candidate);

        Ok(UpdateCheckResult {
            decision,
            candidate,
        })
    }

    pub async fn download_and_activate(
        &self,
        candidate: &ReleaseCandidate,
        files: &ManagedCoreFiles,
    ) -> anyhow::Result<ActiveCore> {
        fs::create_dir_all(
            files
                .current_binary
                .parent()
                .ok_or_else(|| anyhow::anyhow!("core binary path has no parent"))?,
        )?;

        let archive = self.download_asset(candidate).await?;
        let binary = if candidate.is_zip() {
            extract_clash_binary_from_zip(&archive)?
        } else {
            archive
        };
        let sha256 = sha256_hex(&binary);

        if files.current_binary.exists() {
            if files.rollback_binary.exists() {
                fs::remove_file(&files.rollback_binary)?;
            }
            fs::rename(&files.current_binary, &files.rollback_binary)?;
        }

        fs::write(&files.current_binary, binary)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&files.current_binary, fs::Permissions::from_mode(0o755))?;
        }

        let active = ActiveCore {
            channel: candidate.channel,
            release_tag: candidate.release_tag.clone(),
            version: version_from_tag(&candidate.release_tag),
            commit: None,
            asset_name: candidate.asset_name.clone(),
            binary_path: files.current_binary.clone(),
            binary_sha256: Some(sha256),
            updated_at: OffsetDateTime::now_utc(),
        };
        active.save(&files.active_metadata)?;
        Ok(active)
    }

    async fn fetch_release(&self, channel: UpdateChannel) -> anyhow::Result<GitHubRelease> {
        Ok(self
            .client
            .get(channel.release_api_url())
            .header(reqwest::header::USER_AGENT, &self.user_agent)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    async fn download_asset(&self, candidate: &ReleaseCandidate) -> anyhow::Result<Vec<u8>> {
        Ok(self
            .client
            .get(&candidate.download_url)
            .header(reqwest::header::USER_AGENT, &self.user_agent)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?
            .to_vec())
    }
}

pub fn select_release_candidate(
    channel: UpdateChannel,
    arch: &WindowsArch,
    release: &GitHubRelease,
) -> Option<ReleaseCandidate> {
    let arch_name = arch.release_name();
    let ext = std::env::consts::EXE_SUFFIX;

    // Prefer the static-CRT build: it runs without the VC++ runtime installed.
    let direct_exe = [
        format!("clash-rs-{arch_name}-static-crt{ext}"),
        format!("clash-rs-{arch_name}{ext}"),
    ]
    .iter()
    .find_map(|wanted| {
        release
            .assets
            .iter()
            .find(|asset| asset.name.eq_ignore_ascii_case(wanted))
    });

    let zipped = release
        .assets
        .iter()
        .find(|asset| {
            asset.name.contains(arch_name)
                && asset.name.ends_with(".zip")
                && asset.name.to_ascii_lowercase().contains("windows")
        })
        .or_else(|| {
            release
                .assets
                .iter()
                .find(|asset| asset.name.contains(arch_name) && asset.name.ends_with(".zip"))
        });

    direct_exe.or(zipped).map(|asset| ReleaseCandidate {
        channel,
        release_tag: release.tag_name.clone(),
        asset_name: asset.name.clone(),
        download_url: asset.browser_download_url.clone(),
        size: asset.size,
    })
}

pub fn decide_update(
    channel: UpdateChannel,
    current: Option<&ActiveCore>,
    candidate: &ReleaseCandidate,
) -> UpdateDecision {
    let Some(current) = current else {
        return UpdateDecision::NotInstalled {
            latest: candidate.release_tag.clone(),
        };
    };

    if current.asset_name == candidate.asset_name {
        return UpdateDecision::UpToDate;
    }

    match channel {
        UpdateChannel::Stable => {
            if current.release_tag == candidate.release_tag {
                UpdateDecision::UpToDate
            } else {
                UpdateDecision::UpdateAvailable {
                    current: current.release_tag.clone(),
                    latest: candidate.release_tag.clone(),
                }
            }
        }
        UpdateChannel::Preview => {
            // Preview uses a stable tag identifier. Detect updates by actual asset/version.
            let latest = version_from_tag(&candidate.release_tag);
            if current.version == latest {
                UpdateDecision::UpToDate
            } else {
                UpdateDecision::UpdateAvailable {
                    current: current.version.clone(),
                    latest,
                }
            }
        }
    }
}

fn extract_clash_binary_from_zip(archive: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut zip = zip::ZipArchive::new(Cursor::new(archive))?;
    for index in 0..zip.len() {
        let mut file = zip.by_index(index)?;
        // An entry name the parser can't agree on is a malformed (or
        // malicious) archive: refuse it instead of guessing.
        let entry = file.name()?.into_owned();
        let Some(name) = Path::new(&entry)
            .file_name()
            .and_then(|name| name.to_str())
        else {
            continue;
        };

        if name.eq_ignore_ascii_case("clash-rs.exe") {
            let mut binary = Vec::new();
            file.read_to_end(&mut binary)?;
            return Ok(binary);
        }
    }

    anyhow::bail!("downloaded archive does not contain clash-rs.exe")
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn version_from_tag(tag: &str) -> String {
    tag.strip_prefix('v').unwrap_or(tag).to_owned()
}

#[cfg(test)]
mod tests {
    use super::{
        ActiveCore, OffsetDateTime, ReleaseCandidate, UpdateChannel, UpdateDecision, decide_update,
        version_from_tag,
    };
    #[cfg(windows)]
    use super::{GitHubRelease, GitHubReleaseAsset, WindowsArch, select_release_candidate};

    #[cfg(windows)]
    #[test]
    fn selects_matching_windows_asset() {
        let release = GitHubRelease {
            tag_name: "v0.10.6".to_owned(),
            prerelease: false,
            assets: vec![
                GitHubReleaseAsset {
                    name: "clash-rs-aarch64-pc-windows-msvc.zip".to_owned(),
                    browser_download_url: "https://example.invalid/arm.zip".to_owned(),
                    size: 1,
                },
                GitHubReleaseAsset {
                    name: "clash-rs-x86_64-pc-windows-msvc.exe".to_owned(),
                    browser_download_url: "https://example.invalid/x64.zip".to_owned(),
                    size: 2,
                },
            ],
        };

        let candidate =
            select_release_candidate(UpdateChannel::Stable, &WindowsArch::X64, &release)
                .expect("x64 asset should be selected");

        assert_eq!(candidate.release_tag, "v0.10.6");
        assert_eq!(candidate.asset_name, "clash-rs-x86_64-pc-windows-msvc.exe");
        assert_eq!(candidate.size, 2);
    }

    #[test]
    fn preview_decision_not_based_only_on_tag() {
        let current = ActiveCore {
            channel: UpdateChannel::Preview,
            release_tag: "latest".to_owned(),
            version: "0.10.5".to_owned(),
            commit: None,
            asset_name: "clash-rs-x86_64-pc-windows-msvc.exe".to_owned(),
            binary_path: std::path::PathBuf::from("clash-rs.exe"),
            binary_sha256: None,
            updated_at: OffsetDateTime::now_utc(),
        };

        let candidate = ReleaseCandidate {
            channel: UpdateChannel::Preview,
            release_tag: "latest".to_owned(),
            asset_name: "clash-rs-x86_64-pc-windows-msvc-static-crt.exe".to_owned(),
            download_url: "https://example.invalid/latest.exe".to_owned(),
            size: 1,
        };

        assert!(matches!(
            decide_update(UpdateChannel::Preview, Some(&current), &candidate),
            UpdateDecision::UpdateAvailable { .. }
        ));
    }

    #[test]
    fn version_removes_v_prefix() {
        assert_eq!(version_from_tag("v0.10.6"), "0.10.6");
        assert_eq!(version_from_tag("latest"), "latest");
    }
}
