<div align="center">

<img src="apps/desktop/src-tauri/icons/128x128@2x.png" width="88" alt="Nocterm Logo" />

# Nocterm

**A local-first desktop terminal client for macOS & Windows**

Manage local shells, SSH, SFTP and AI assistance in one place — all connection data and credentials stay on your machine.

English | [简体中文](README.zh-CN.md)

[![GitHub Release](https://img.shields.io/github/v/release/slightlee/nocterm?include_prereleases&sort=semver)](https://github.com/slightlee/nocterm/releases)
[![CI](https://github.com/slightlee/nocterm/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/slightlee/nocterm/actions/workflows/ci.yml)
![Platform](https://img.shields.io/badge/platform-macOS%2014%2B%20%7C%20Windows-blue)
[![License](https://img.shields.io/badge/license-MIT-green)](LICENSE)
![Built with](https://img.shields.io/badge/built%20with-Tauri%202%20%C2%B7%20React%2019%20%C2%B7%20Rust-24C8DB)

[Download](#-download) · [Features](#-features) · [Community](#-community) · [Development](#-development) · [Documentation](#-documentation)

</div>

---

## ✨ Features

- 🖥️ **Terminal** — local shells with multi-tab sessions; system / light / dark themes; customizable fonts and color schemes
- 🔗 **SSH** — connection groups, search and OpenSSH config import; password, key file and agent authentication; TOFU host keys; in-process pure Rust implementation, no system `ssh` required
- 📂 **SFTP** — dual-pane browsing, file operations, uploads and downloads with progress and cancel
- 🤖 **AI assistance** — connect your local Codex / Claude Code / Grok CLI to the active terminal: structured server diagnostics, command approval and redacted audit logs. **AI providers never touch SSH credentials and cannot switch targets on their own** — all access goes through the Nocterm tool gateway
- 🔒 **Local-first** — everything runs on your machine, no cloud service required. Connections live in local SQLite; saved passwords go into the OS keychain (macOS Keychain / Windows Credential Manager)

## 📦 Download

Grab the latest build from [GitHub Releases](https://github.com/slightlee/nocterm/releases):

| Platform          | Package                                      |
| ----------------- | -------------------------------------------- |
| Apple Silicon Mac | `Nocterm_<version>_macos_aarch64.dmg`        |
| Intel Mac         | `Nocterm_<version>_macos_x86_64.dmg`         |
| Windows x64       | `Nocterm_<version>_windows_x86_64-setup.exe` |

> [!NOTE]
> Read the release notes before installing to confirm the release channel and signing status, and verify the package with the matching `.sha256` file. Pre-release builds are for testing only.
> The AI panel uses your locally installed and signed-in `codex` / `claude` / `grok` CLI (Grok ≥ 1.0.34). Nocterm does not install or sign in to providers for you.

## 🚀 Development

Requirements: Node.js 22 · pnpm 11 (pinned via Corepack) · Rust 1.94 · [Tauri 2 system dependencies](https://v2.tauri.app/start/prerequisites/).

```bash
corepack pnpm install
corepack pnpm tauri dev      # run the desktop client
corepack pnpm check          # frontend quality gate
corepack pnpm cargo:check    # Rust quality gate
```

## 💬 Community

- [LINUX DO](https://linux.do) — the Chinese-speaking developer community where Nocterm is discussed and promoted. Feedback threads are posted there; feel free to join the discussion
- [GitHub Issues](https://github.com/slightlee/nocterm/issues) — bug reports and feature requests
- [GitHub Discussions](https://github.com/slightlee/nocterm/discussions) — questions and general discussion

## 📚 Documentation

Full details on the security design, the AI tool-gateway boundary, architecture layering, testing and the release process:

- [Technical architecture](docs/technical-architecture.md)
- [Development guide](docs/development.md)
- [Testing & acceptance](docs/testing.md)
- [Release process](docs/release-process.md)
- [Contributing](CONTRIBUTING.md)
- [Security policy](SECURITY.md) — please report vulnerabilities privately, do not open public issues

## 📄 License

[MIT License](LICENSE) © Nocterm contributors
