// Download the latest stable clash-rs into src-tauri/core/ so installers can
// ship it: the first connection then works without reaching GitHub.
//
// Picks the same asset the app's updater would (static-CRT exe on Windows,
// static musl build on Linux) and verifies it against GitHub's SHA-256.
// Writes core/clash-rs[.exe] and core/release.json ({ tag, asset }).
// Set GITHUB_TOKEN to avoid the anonymous API rate limit.

import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";

const RELEASE_API = "https://api.github.com/repos/Watfaq/clash-rs/releases/latest";
const windows = process.platform === "win32";
const target = windows ? "x86_64-pc-windows-msvc" : "x86_64-unknown-linux-musl";
const ext = windows ? ".exe" : "";
const wanted = [`clash-rs-${target}-static-crt${ext}`, `clash-rs-${target}${ext}`];

const dir = path.join(import.meta.dirname, "..", "src-tauri", "core");
const binary = path.join(dir, `clash-rs${ext}`);
const meta = path.join(dir, "release.json");

const headers = { "user-agent": "choclite-build", accept: "application/vnd.github+json" };
if (process.env.GITHUB_TOKEN) headers.authorization = `Bearer ${process.env.GITHUB_TOKEN}`;

const res = await fetch(RELEASE_API, { headers });
if (!res.ok) throw new Error(`GitHub API: ${res.status} ${await res.text()}`);
const release = await res.json();
const asset = wanted
  .map((name) => release.assets.find((a) => a.name === name))
  .find(Boolean);
if (!asset) throw new Error(`release ${release.tag_name} has none of: ${wanted.join(", ")}`);

// Already fetched this exact release: nothing to do.
if (fs.existsSync(binary) && fs.existsSync(meta)) {
  const current = JSON.parse(fs.readFileSync(meta, "utf8"));
  if (current.tag === release.tag_name && current.asset === asset.name) {
    console.log(`clash-rs ${release.tag_name} already in ${dir}`);
    process.exit(0);
  }
}

console.log(`Downloading ${asset.name} (${release.tag_name}, ${(asset.size / 1e6).toFixed(1)} MB)`);
const download = await fetch(asset.browser_download_url, { headers: { "user-agent": headers["user-agent"] } });
if (!download.ok) throw new Error(`download failed: ${download.status}`);
const bytes = Buffer.from(await download.arrayBuffer());

const sha256 = createHash("sha256").update(bytes).digest("hex");
const expected = asset.digest?.replace(/^sha256:/, "");
if (!expected) throw new Error(`GitHub reports no digest for ${asset.name}; refusing to bundle it unverified`);
if (sha256 !== expected) throw new Error(`SHA-256 mismatch for ${asset.name}: got ${sha256}, expected ${expected}`);

fs.mkdirSync(dir, { recursive: true });
fs.writeFileSync(binary, bytes, { mode: 0o755 });
fs.writeFileSync(meta, JSON.stringify({ tag: release.tag_name, asset: asset.name }, null, 2) + "\n");
console.log(`clash-rs ${release.tag_name} → ${binary} (sha256 ${sha256})`);
