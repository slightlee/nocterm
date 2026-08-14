# Nocterm

Nocterm 是一个本地优先、面向 macOS 和 Windows 的桌面终端客户端，用于管理本地终端、SSH 连接、SFTP 文件和本地连接配置。

## 技术栈

- Tauri 2 + React + TypeScript + Vite；
- Domain、Application、Infrastructure 三层 Rust workspace；
- pnpm workspace；
- ESLint、Prettier、Stylelint、Vitest、rustfmt、Clippy 和 Cargo Test 门禁。

## 项目结构

```text
apps/desktop/                React UI 与 Tauri Host
crates/nocterm-domain/       领域模型与 Port
crates/nocterm-application/  用例编排
crates/nocterm-infrastructure/  平台与基础设施 Adapter
docs/                        架构、规范、ADR 与测试文档
```

## 开发

环境要求：Node.js 22、pnpm 11、Rust 1.94，以及对应平台的 Tauri 系统依赖。

```bash
corepack pnpm install
corepack pnpm dev
corepack pnpm tauri dev
```

验证：

```bash
corepack pnpm check
corepack pnpm cargo:check
```

## 文档

- [技术架构](docs/technical-architecture.md)：架构边界与技术设计；
- [开发规范](docs/development.md)：目录边界、命令与日常开发约束；
- [AI 开发规范](docs/ai-development-policy.md)：AI 协作、证据和安全要求；
- [Code Review 规范](docs/code-review.md)：审查优先级与通过标准；
- [跨平台测试](docs/testing/cross-platform-smoke.md)：macOS/Windows 验收流程；
- [架构决策记录](docs/adr/)：关键技术取舍及其依据。
