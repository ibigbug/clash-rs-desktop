# Choclite for Desktop

**English** · [简体中文](README.zh-CN.md)

Choclite is a desktop client for [clash-rs](https://github.com/Watfaq/clash-rs) built around a single switch. Add a subscription, turn it on, and it works. Everything else — routing rules, custom configuration, diagnostics — is one level down for those who need it.

Available for **Windows** and **Linux**. On macOS, use the native Choclite app.

<p align="center">
  <img src="docs/images/en/home-light.png" width="240" alt="Home screen, connected">
  <img src="docs/images/en/home-dark.png" width="240" alt="Home screen, dark theme">
  <img src="docs/images/en/subscriptions.png" width="240" alt="Subscription management">
  <img src="docs/images/en/diagnostics.png" width="240" alt="Network diagnostics">
</p>

## Features

**Simple by default**
- One switch on the home screen, with live speed, the current node and subscription usage.
- Uses the system proxy out of the box. No administrator rights, no prompts.
- The node you pick is remembered per subscription, across restarts and mode changes.
- Subscriptions update automatically on the provider's schedule.
- English and Simplified Chinese interface, following the system language by default.

**Works with real-world networks**
- LAN addresses, `.lan` / `.local` / `home.arpa` names and the network's own DNS suffixes (company networks, VPNs, Tailscale) always connect directly and are resolved by the system's DNS.
- A list of addresses that should never use the proxy.
- Detects programs that ignore the system proxy (games, launchers, command-line tools) and offers Enhanced Mode for them.

**Enhanced Mode (TUN)**
- Captures traffic from every program, optionally including IPv6.
- Installed once with a single administrator prompt; switching it on and off afterwards needs no further prompts.

**Subscriptions**
- Clash subscriptions, base64 link lists, share links (`ss://`, `trojan://`, `vmess://`, `vless://`, `hysteria2://`) and local configuration files.
- One-click import from provider websites via `choclite://` links (and `clash://` when no other client handles them). Imports always ask for confirmation.
- Merge several subscriptions: automatic selection then picks the fastest node across all of them.

**For advanced users**
- Site rules: send a site, IP range or program through the proxy, direct, or block it — without writing YAML.
- Custom configuration: raw Clash YAML layered on top of the subscription, preserved across updates.
- Network diagnostics: tells local network, node, routing, DNS, IPv6 and system proxy problems apart, with one-click fixes.
- Logs written to rotating files.

**Safe by design**
- The original system proxy settings are restored on disconnect, if the core crashes, on exit, and on the next launch after an unexpected shutdown.
- A configuration the core cannot run is rolled back instead of dropping the connection.

## Installation

### Windows

Download `Choclite_<version>_x64-setup.exe` from the [releases page](https://github.com/Watfaq-legacy/clash-rs-desktop/releases) and run it.

- Installs for the current user only and **does not require administrator rights**.
- Requires Microsoft Edge WebView2, which ships with Windows 10 (recent builds) and Windows 11; the installer adds it if missing.
- The installer includes the clash-rs core, so the first connection works without reaching GitHub. Newer cores are offered as updates in Settings.
- Enabling Enhanced Mode installs a small background service (`ChocliteService`) after one administrator prompt. It is removed together with the app.

### Linux

Download the `.deb` package or the AppImage from the [releases page](https://github.com/Watfaq-legacy/clash-rs-desktop/releases).

- System proxy mode configures GNOME-compatible desktops (GNOME, Cinnamon, Budgie, Pantheon) through `gsettings`.
- Enhanced Mode installs a systemd service (`choclite.service`) after one `pkexec` password prompt. The `.deb` package removes it on uninstall.

## How it works

Choclite runs the official clash-rs binary and builds its configuration from layers, in increasing priority:

1. the subscription (plus any merged subscriptions);
2. the user's custom configuration;
3. bypass and site rules;
4. settings made in the app (ports, routing mode, Enhanced Mode, IPv6).

Settings made in the app always take precedence over configuration files. Changes are applied with a hot reload, validated first, so the connection is not interrupted.

**System proxy mode** runs clash-rs as a child process of the app, bound to its lifetime, and points the operating system's proxy at it.

**Enhanced Mode** runs clash-rs inside a privileged service so it can create a TUN device. The service only ever runs its own copy of clash-rs from an administrator-only directory, and sanitises any configuration it receives: controller and dashboard settings are stripped, provider caches are confined to its data directory, and paths outside it are rejected. On Windows, [Wintun](https://www.wintun.net/) is downloaded during service installation and verified against a pinned SHA-256 hash.

## Building from source

Requirements: Rust 1.95 or later, Node.js 20 or later, and pnpm. On Linux, also install the [Tauri system dependencies](https://tauri.app/start/prerequisites/#linux).

```sh
cd app
pnpm install
pnpm tauri dev
```

To test Enhanced Mode during development, build the service as well:

```sh
cargo build -p clash-rs-desktop-service
```

Bundling (`pnpm bundle:windows` / `pnpm bundle:linux`) fetches the latest clash-rs into `app/src-tauri/core/` first. To use a local clash-rs build instead, set `CLASH_RS_DESKTOP_CORE` to the path of the binary.

Checks:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
cd app && pnpm check
```

Packaging:

```sh
cd app
pnpm bundle:windows   # NSIS installer, including the service
pnpm bundle:linux     # .deb and AppImage (run on Linux)
```

## Project layout

| Path | Contents |
|---|---|
| `crates/clash-rs-desktop-core` | Connection engine, configuration generation, subscriptions, system proxy, diagnostics, service protocol |
| `crates/clash-rs-desktop-service` | Privileged service for Enhanced Mode (Windows service / systemd unit) |
| `app/src-tauri` | Tauri backend: commands, background tasks, tray |
| `app/src` | User interface (Svelte 5) |
| `docs` | Implementation plan and screenshots |

## License

MIT — see [LICENSE](LICENSE).

Country flag glyphs are from [Twemoji](https://github.com/twitter/twemoji), licensed under CC-BY 4.0.
