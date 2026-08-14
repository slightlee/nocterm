# Nocterm 开发规范

AI 与人工开发都必须遵守根 `AGENTS.md`。AI 的范围、安全、验证和交付规则详见 `docs/ai-development-policy.md`，审查标准详见 `docs/code-review.md`。

## 1. 依赖方向

前端依赖方向为 `app → features → shared`。Feature 之间只允许通过各自 `index.ts` 公开入口协作，禁止跨目录引用内部组件、Store 和 API。`@tauri-apps/api` 只能出现在 Feature 的 `api/` 或专用平台适配目录。

Rust 依赖方向为：

```text
desktop host → application → domain
             ↘ infrastructure → domain
```

Domain 不依赖 Tauri、SQLite、文件系统和操作系统 API。Infrastructure 实现 Domain Port；desktop host 只负责 IPC、DTO、事件和依赖装配。

## 2. 命名与文件

- React 组件使用 PascalCase，Hook 使用 `useXxx`；
- TypeScript 普通文件使用 kebab-case，测试与源码同目录；
- Rust module、function 和文件使用 snake_case，类型使用 PascalCase；
- IPC command 使用 `{domain}_{action}`，事件使用 `nocterm://{domain}-{event}`；
- 单文件达到约 300～400 行时检查职责，不以行数作为机械拆分标准；
- 注释解释边界、协议兼容和非显然决策，不复述代码。

## 3. 错误与日志

- UI 只依赖稳定错误码，不解析 Rust、系统命令或数据库错误文本；
- 原始错误必须在 Adapter 边界转换；
- 密码、私钥、Token、完整命令输出不得进入普通日志；
- 异步任务必须拥有 ID、状态、取消路径和唯一清理责任。

## 4. 完成定义

代码完成不等于功能完成。每项功能必须同时具备：

1. 明确的输入、输出、错误和生命周期行为；
2. 单元测试或契约测试；
3. 对应平台静态检查通过；
4. macOS/Windows 真实运行边界说明；
5. 文档中的完成状态与实际验证一致。

合并前运行：

```bash
corepack pnpm check
corepack pnpm cargo:check
```

## 5. Git 提交规范

提交信息采用 Conventional Commits：

```text
<type>(<optional-scope>): <subject>
```

允许的 `type`：

- `feat`：新增用户可见能力；
- `fix`：修复缺陷；
- `refactor`：不改变行为的结构调整；
- `perf`：性能优化；
- `test`：测试变更；
- `docs`：文档变更；
- `style`：不影响语义的格式调整；
- `build`：构建系统或依赖变更；
- `ci`：持续集成变更；
- `chore`：其他工程维护；
- `revert`：回滚提交。

主题必须简洁、明确，不超过 100 个字符。例如：

```text
feat(terminal): add local session lifecycle
fix(sftp): release transfer task after cancellation
chore: initialize Nocterm project scaffold
```

一次提交只表达一个完整意图。提交前 Husky 会通过 lint-staged 格式化并检查暂存文件，`commit-msg` Hook 会执行 Commitlint；不得使用 `--no-verify` 绕过失败，除非已明确说明 Hook 自身故障并获得确认。完整质量门禁仍使用 `corepack pnpm check` 和 `corepack pnpm cargo:check`。

远程仓库必须保护 `main`，禁止直接推送，并把 CI 配置为 Pull Request 的 Required Check。本地 Hook 可被主动绕过，不能替代远程合并门禁。
