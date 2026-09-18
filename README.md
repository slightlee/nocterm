# Nocterm

[![GitHub Release](https://img.shields.io/github/v/release/slightlee/nocterm?include_prereleases&sort=semver)](https://github.com/slightlee/nocterm/releases)

Nocterm 是一个本地优先的 macOS / Windows 桌面终端客户端，统一管理本地终端、SSH、SFTP 和 AI 辅助操作。连接资料保存在本机 SQLite；选择保存的密码进入系统凭据库；AI Provider 复用用户本机已经安装并登录的 CLI。

最新版本、发布通道和发布说明以 [GitHub Releases](https://github.com/slightlee/nocterm/releases) 为准；源码版本由根目录 `package.json` 唯一管理，README 不维护静态版本号。

## 当前能力

- 本地终端、多标签、窗口尺寸同步和会话重连；
- SSH 连接资料、分组、搜索、排序和 OpenSSH Config 导入；
- 密码、私钥文件和 SSH Agent 认证，主机密钥采用 TOFU 策略；
- SFTP 双栏浏览、文件操作、上传、下载、进度和取消；
- 跟随系统、浅色、深色主题及终端字体、配色设置；
- AI 面板接入 Codex、Claude Code 和 Grok，绑定当前本地或 SSH 终端；
- AI 结构化服务器诊断、通用命令审批、四级会话权限和脱敏审计。

AI Provider 不直接获得 SSH 凭据，也不能自行切换目标。三个 Provider 通过同一套 Nocterm 工具网关访问当前终端；详细边界见[技术架构](docs/technical-architecture.md)。

## 下载安装

从 [GitHub Releases](https://github.com/slightlee/nocterm/releases) 下载最新版本，并根据设备选择安装包：

- Apple Silicon Mac：`Nocterm_<version>_macos_aarch64.dmg`；
- Intel Mac：`Nocterm_<version>_macos_x86_64.dmg`；
- Windows x64：`Nocterm_<version>_windows_x86_64-setup.exe`。

安装前请阅读对应 Release Notes，确认发布通道、签名状态和已知限制，并使用同名 `.sha256` 文件核对安装包。预发布版本只用于其发布说明声明的测试范围。

## 平台与前置条件

Nocterm 目标平台为 macOS 14+ 和 Windows。源码开发需要：

- Node.js 22；
- pnpm 11（仓库通过 Corepack 固定版本）；
- Rust 1.94；
- 对应平台的 [Tauri 2 系统依赖](https://v2.tauri.app/start/prerequisites/)；
- macOS 的 Xcode Command Line Tools，或 Windows 的 Microsoft C++ Build Tools 与 WebView2。

AI 面板按需使用本机的 `codex`、`claude` 或 `grok` CLI。CLI 必须能从启动 Nocterm 的环境中找到并已完成登录；Grok 最低支持版本为 `1.0.34`。Nocterm 当前不提供 Provider 安装、登录或可执行文件路径配置。

## 本地开发

```bash
corepack pnpm install
corepack pnpm tauri dev
```

浏览器预览只用于检查不依赖 Tauri IPC 的页面布局：

```bash
corepack pnpm dev
```

完整质量门禁：

```bash
corepack pnpm check
corepack pnpm cargo:check
```

## 安全与隐私

- 核心终端、SSH、SFTP 和配置能力在本机运行，不依赖 Nocterm 云服务；
- SQLite 不保存密码和私钥内容；选择保存的密码进入系统凭据库，未保存密码只在有活跃会话时短暂驻留内存，私钥只保存本地文件路径；
- Provider 仍会按照其自身配置把提示和工具结果发送给对应模型服务；不要把敏感信息交给不信任的 Provider；
- AI 审计只保存工具、审批和结果元数据，不保存命令、参数、输出、Token、主机或账号；
- 安全问题请按 [SECURITY.md](SECURITY.md) 私下报告，不要创建公开 Issue。

## 项目结构

```text
apps/desktop/                  React UI 与 Tauri Host
crates/nocterm-domain/         领域模型与 Port
crates/nocterm-application/    用例编排
crates/nocterm-infrastructure/ SQLite、PTY、SSH、SFTP 与平台 Adapter
docs/                          架构、开发、测试与发布规范
```

## 文档

- [技术架构](docs/technical-architecture.md)
- [开发规范](docs/development.md)
- [测试与验收](docs/testing.md)
- [发布流程](docs/release-process.md)
- [贡献指南](CONTRIBUTING.md)
- [安全策略](SECURITY.md)

## 许可证

[MIT License](LICENSE)
