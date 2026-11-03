# Choclite 桌面版

[English](README.md) · **简体中文**

Choclite 是基于 [clash-rs](https://github.com/Watfaq/clash-rs) 的桌面客户端，核心只有一个开关：添加订阅，打开开关，就能用。分流规则、自定义配置、网络诊断等功能放在下一层，需要的用户随时可以使用。

支持 **Windows** 和 **Linux**。macOS 请使用原生的 Choclite 应用。

<p align="center">
  <img src="docs/images/zh-CN/home-light.png" width="240" alt="主页（已连接）">
  <img src="docs/images/zh-CN/home-dark.png" width="240" alt="主页（深色主题）">
  <img src="docs/images/zh-CN/subscriptions.png" width="240" alt="订阅管理">
  <img src="docs/images/zh-CN/diagnostics.png" width="240" alt="网络诊断">
</p>

## 功能

**默认简单**
- 主页只有一个开关，同时显示实时网速、当前节点和订阅用量。
- 默认使用系统代理，无需管理员权限，不弹任何授权。
- 按订阅记住所选节点，重启或切换模式后依然保留。
- 订阅按服务商建议的间隔自动更新。
- 界面支持简体中文和英文，默认跟随系统语言。

**适应真实的网络环境**
- 局域网地址、`.lan` / `.local` / `home.arpa` 等内网域名，以及网络自身的 DNS 后缀（公司网络、VPN、Tailscale）始终直连，并由系统原本的 DNS 解析。
- 支持自定义"不走代理的地址"。
- 识别不使用系统代理的程序（游戏、启动器、命令行工具），并提示可以开启增强模式。

**增强模式（TUN）**
- 接管所有程序的流量，可选同时接管 IPv6。
- 首次开启时授权一次管理员权限，之后开关不再需要授权。

**订阅**
- 支持 Clash 订阅、base64 链接列表、分享链接（`ss://`、`trojan://`、`vmess://`、`vless://`、`hysteria2://`）以及本地配置文件。
- 支持服务商网站的一键导入：`choclite://` 链接，以及没有其他客户端占用时的 `clash://` 链接。导入前始终需要用户确认。
- 可以合并多个订阅，"自动选择"会在所有订阅的节点中挑选最快的一个。

**进阶功能**
- 网站规则：无需编写 YAML，即可让某个网站、IP 段或程序走代理、直连或被拦截。
- 自定义配置：在订阅之上叠加原始 Clash YAML，订阅更新后依然保留。
- 网络诊断：区分本地网络、节点、路由、DNS、IPv6 和系统代理问题，并提供一键修复。
- 日志自动写入文件，按大小滚动保存。

**安全可靠**
- 断开连接、内核崩溃、退出应用，以及意外关机后的下一次启动，都会恢复原有的系统代理设置。
- 内核无法运行的配置会自动回滚，不会因此断网。

## 安装

### Windows

从 [Releases 页面](https://github.com/Watfaq-legacy/clash-rs-desktop/releases)下载 `Choclite_<版本>_x64-setup.exe` 并运行。

- 仅为当前用户安装，**不需要管理员权限**。
- 依赖 Microsoft Edge WebView2。Windows 10 较新版本和 Windows 11 已经自带；如果缺失，安装程序会自动安装。
- 安装包自带 clash-rs 内核，第一次连接不需要访问 GitHub。内核的新版本会在设置里提示更新。
- 开启增强模式时，经过一次管理员授权后，会安装一个后台服务（`ChocliteService`）。卸载应用时会一并移除。

### Linux

从 [Releases 页面](https://github.com/Watfaq-legacy/clash-rs-desktop/releases)下载 `.deb` 包或 AppImage。

- 系统代理模式通过 `gsettings` 配置兼容 GNOME 的桌面环境（GNOME、Cinnamon、Budgie、Pantheon）。
- 开启增强模式时，经过一次 `pkexec` 密码验证后，会安装一个 systemd 服务（`choclite.service`）。卸载 `.deb` 包时会一并移除。

## 工作原理

Choclite 运行官方的 clash-rs 程序，并按以下顺序分层生成配置，后面的层优先级更高：

1. 订阅（以及合并进来的其他订阅）；
2. 用户的自定义配置；
3. 不走代理的地址和网站规则；
4. 在应用中做的设置（端口、分流模式、增强模式、IPv6）。

应用中的设置始终优先于配置文件。修改会先经过校验，再以热加载的方式生效，不会中断连接。

**系统代理模式**下，clash-rs 作为应用的子进程运行，生命周期与应用绑定，操作系统的代理设置指向它。

**增强模式**下，clash-rs 运行在一个特权服务中，以便创建 TUN 虚拟网卡。这个服务只运行它自己目录中的 clash-rs，该目录只有管理员可以写入；服务收到的配置都会先经过清理：去掉控制器和面板相关设置，把规则和节点集的缓存限制在服务的数据目录内，并拒绝该目录以外的路径。在 Windows 上，[Wintun](https://www.wintun.net/) 会在安装服务时下载，并用固定的 SHA-256 哈希校验。

## 从源码构建

需要 Rust 1.95 或更高版本、Node.js 20 或更高版本，以及 pnpm。在 Linux 上还需要安装 [Tauri 的系统依赖](https://tauri.app/start/prerequisites/#linux)。

```sh
cd app
pnpm install
pnpm tauri dev
```

如果要在开发时测试增强模式，需要同时编译后台服务：

```sh
cargo build -p clash-rs-desktop-service
```

打包（`pnpm bundle:windows` / `pnpm bundle:linux`）时会先把最新版 clash-rs 下载到 `app/src-tauri/core/`。如果想改用本地编译的 clash-rs，可以把环境变量 `CLASH_RS_DESKTOP_CORE` 设置为该程序的路径。

检查：

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
cd app && pnpm check
```

打包：

```sh
cd app
pnpm bundle:windows   # NSIS 安装包，包含后台服务
pnpm bundle:linux     # .deb 和 AppImage（需在 Linux 上运行）
```

## 目录结构

| 路径 | 内容 |
|---|---|
| `crates/clash-rs-desktop-core` | 连接引擎、配置生成、订阅、系统代理、网络诊断、服务通信协议 |
| `crates/clash-rs-desktop-service` | 增强模式使用的特权服务（Windows 服务 / systemd 单元） |
| `app/src-tauri` | Tauri 后端：命令、后台任务、托盘 |
| `app/src` | 用户界面（Svelte 5） |
| `docs` | 实施计划和截图 |

## 许可证

MIT，详见 [LICENSE](LICENSE)。

国旗图标来自 [Twemoji](https://github.com/twitter/twemoji)，采用 CC-BY 4.0 许可。
