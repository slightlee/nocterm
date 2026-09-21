# 贡献指南

感谢参与 Nocterm。提交代码前请先阅读[技术架构](docs/technical-architecture.md)和[开发规范](docs/development.md)；涉及真实终端、SSH、SFTP 或 AI Provider 时执行[测试与验收](docs/testing.md)；涉及发布时遵循[发布流程](docs/release-process.md)。

## 报告问题

- 可复现缺陷使用 GitHub Bug 模板，提供平台、版本、最小复现步骤和脱敏证据；
- 非平凡功能先创建 Feature Request，说明用户问题、范围、非目标和验收标准；
- 安全漏洞不要提交公开 Issue，按 [SECURITY.md](SECURITY.md) 报告。

日志、截图和测试数据中不得包含密码、私钥、Token、完整环境变量或真实生产主机信息。

## 开发流程

1. 从最新 `main` 创建短期任务分支，例如 `fix/windows-keychain` 或 `feat/sftp-resume`；
2. 先阅读入口、调用链、测试和相关文档，再做最小完整变更；
3. 保持前端 `app -> features -> shared` 和 Rust `desktop -> application -> domain` 的依赖方向；
4. 新增行为补充测试，Bug 修复原则上补充回归测试；
5. 涉及用户行为、架构、协议或验收状态时同步对应文档；
6. 运行与风险相称的自动检查和真实环境测试；
7. 通过 Pull Request 合并，仓库使用 Squash merge。

## 本地验证

安装依赖后执行完整门禁：

```bash
corepack pnpm install
corepack pnpm check
corepack pnpm cargo:check
```

涉及 UI、终端、凭据、SSH、SFTP 或 AI Provider 的变更，还必须执行 `docs/testing.md` 中对应的真实运行流程。CI 或浏览器预览不能替代 macOS / Windows Tauri 验收。

本地需要点开验证 Release 构建产物的可运行性时，在 macOS 执行：

```bash
corepack pnpm build:app
```

它构建并保留可直接双击运行的 `target/release/bundle/macos/Nocterm.app`，不产出 DMG。注意 `release:build:macos` 与 CI 的 DMG 构建只产出可分发的安装包，会在结束后自动清理中间产物 `.app`（Tauri 设计行为）；`target/release/artifacts/` 是唯一稳定的发布产物目录。

## 提交与 Pull Request

提交信息和 Pull Request 标题使用英文 Conventional Commits，格式为：

```text
<type>: <subject>
```

仓库不使用 scope。示例：`fix: release transfer task after cancellation`。

Pull Request 应说明：

- 问题与变更范围；
- 关键实现和安全边界；
- 已执行的静态检查、自动测试、构建和真实运行；
- 未验证的平台、环境或场景；
- 关联 Issue，未完成全部验收时使用 `Refs #123`，完成时才使用 `Closes #123`。

更完整的分支、提交、语言与合并规则只在[开发规范](docs/development.md)中维护。
