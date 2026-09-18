# Nocterm 开发规范

本文件是 Nocterm 架构边界、开发验证、分支、Pull Request 和 Git 提交的详细规则来源。

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

## 5. Issue、分支与 Pull Request

`main` 是受保护的集成分支，不等同于已发布版本。禁止直接推送；功能、修复、文档和 CI 变更都必须从最新 `main` 创建短期任务分支，通过 Pull Request 合并。当前不维护长期 `develop` 或 `release/*` 分支；引入新分支模型必须有并行版本或发布列车等实际需求，并在技术架构的“关键技术决策”中同步自动化与合并策略。

### 5.1 Issue 边界

- 可复现的 Bug 原则上先创建 Issue，记录影响平台、复现步骤、预期行为、实际行为和脱敏后的证据；涉及凭据泄露、远程代码执行或数据损坏的安全问题必须使用 GitHub Private Security Advisory，不得创建公开 Issue；
- 非平凡功能先通过 Issue 明确用户问题、范围、非目标和验收标准；拼写、纯格式等无需独立跟踪的小改动可以不创建 Issue，但 Pull Request 必须说明原因；
- Issue 描述问题和验收边界，不预先锁死实现；一个 Issue 可以经过讨论后拆分为多个独立交付的 Pull Request。
- 发布候选版本使用“发布验收”Issue 记录 Tag、Commit、CI Run、产物哈希、目标平台实测和最终发布决定；该 Issue 是验收记录，不替代 Release Please Pull Request、Git Tag 或发布授权。

### 5.2 分支与 Pull Request

- 一个分支和 Pull Request 表达一个逻辑完整的变更，不与单个 Git 提交一一对应；
- 同一任务可以在原分支上多次提交和推送，已打开的 Pull Request 会自动更新，不得为每次修改重复创建 Pull Request；
- 同一轮验收发现的高度相关问题，优先收敛到一个临时稳定化分支和 Draft Pull Request；无关变更、需要独立回滚或不能互相等待的修复才拆分；
- 当前 Pull Request 的 CI、评审或验收尚未完成时，先在原分支修正同范围问题；不得用新 Pull Request 规避原 Pull Request 的失败门禁；
- 任务开发中可以提前打开 Draft Pull Request 供协作者拉取和评审；只有范围收敛、相关检查通过后才转为可合并状态；
- Release Please Pull Request 只负责版本、变更日志和发布准备，业务修复必须通过普通任务分支合入 `main`；发布范围重新稳定后，由维护者手动运行 Release Please 刷新原发布 Pull Request。

分支名使用与 Conventional Commits 一致的类型前缀；有关联 Issue 时建议包含编号，例如 `feat/123-sftp-upload`、`fix/456-windows-keychain`、`docs/123-release-process` 或 `ci/123-release-validation`。分支合并后立即删除，不将已完成的任务分支演变为长期集成分支。

Pull Request 标题必须符合第 6 节的 Conventional Commits 规范，并作为 Squash merge 后进入 `main` 的提交信息。不应为了关联 Issue 把编号塞入提交标题。关联语义按以下规则选择：

- 默认使用 `Refs #123`，表示变更与 Issue 相关，但合并时不自动关闭；
- 只有 Pull Request 合并本身即可满足 Issue 全部验收标准时，才使用 `Closes #123` 或 `Fixes #123`；
- 如果仍需合并后的目标平台、最终安装包或人工验收，必须使用 `Refs #123`，补齐证据后再手动关闭 Issue；
- 一个 Issue 拆分为多个 Pull Request 时，前置 Pull Request 使用 `Refs`，仅最终完成全部验收标准的 Pull Request 可以使用 `Closes` 或 `Fixes`。

仓库只允许 Squash merge，禁止 Merge Commit 和 Rebase merge，以保持一个 Pull Request 对应 `main` 上一个可回滚的逻辑提交。合并前应在 Pull Request 中分别记录已检查、已自动测试、已构建、已真实运行和未验证范围，不得用 CI 通过代替真实平台验收。

GitHub 必须通过 Ruleset 保护 `main`：所有变更必须经过 Pull Request，Required Checks 至少包含前端检查以及 macOS、Windows Rust 检查，并禁止强制推送和删除分支。单人维护阶段不强制批准人数，但必须解决 Review Conversation；增加协作者后再要求至少一名非作者批准。仓库设置应启用合并后自动删除分支。

### 5.3 协作语言

- Issue 标题和正文允许使用中文或英文，不因贡献者语言阻止问题提交；同一 Issue 内应尽量保持一种主要语言；
- Pull Request 正文、Review 和讨论允许使用中文或英文，原则上跟随关联 Issue 或贡献者使用的语言；
- Pull Request 标题和 Git Commit 必须使用英文 Conventional Commits。仓库使用 Squash merge，因此 PR 标题就是进入 `main` 的永久提交信息；
- 分支名、代码标识、API、稳定错误码、GitHub Label、CI Job、Tag 和安装包名称使用英文；
- 当前 README、开发文档、代码注释和用户界面以中文为主，第三方技术名词保留原文；未来需要国际化时通过独立英文文档或 i18n 资源实现，不在同一句文案中逐句混排。

Commitlint 的本地 `commit-msg` Hook 和远程 CI 共同校验 Commit 与 PR 标题的 Conventional Commits 结构及英文 ASCII 字符。Issue 与 PR 正文不执行语言检测，避免限制外部贡献者表达；维护者在合并前只需要规范化 PR 标题。

## 6. Git 提交规范

提交信息采用 Conventional Commits：

```text
<type>: <subject>
```

仓库不使用 scope，`feat(terminal): ...` 等带括号形式会被 Commitlint 拒绝，以保持提交历史简洁一致。

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

主题必须使用英文 ASCII 字符，保持简洁、明确且不超过 100 个字符。例如：

```text
feat: add local session lifecycle
fix: release transfer task after cancellation
chore: initialize Nocterm project scaffold
```

一次提交只表达一个完整意图。提交前 Husky 会通过 lint-staged 格式化并检查暂存文件，`commit-msg` Hook 会执行 Commitlint；不得使用 `--no-verify` 绕过失败，除非已明确说明 Hook 自身故障并获得确认。完整质量门禁仍使用 `corepack pnpm check` 和 `corepack pnpm cargo:check`。

本地 `pre-commit` Hook 禁止直接在 `main` 提交，远程 Ruleset 禁止绕过 Pull Request 推送，并把 CI 配置为 Pull Request 的 Required Check。本地 Hook 可被主动绕过，不能替代远程合并门禁。

普通贡献者只提交符合上述规范的功能、修复和文档变更，不手工递增产品版本。仓库维护者在发布范围稳定后手动运行 Release Please，由其根据 `main` 上的 Conventional Commits 维护发布 Pull Request；产品版本、发布通道切换、Tag 和最终发布仍按 `docs/release-process.md` 执行。

## 7. 事实、范围与安全操作

修改前先确认工作区状态，阅读入口、调用方、实现、测试和关联文档。设计文档、类型名称或测试替身只能证明意图，不能证明运行时实际经过该路径。

- 选择满足需求的最小完整变更，不混入无关重构和未来占位；
- 功能先定义输入、输出、错误、取消和清理行为；
- Bug 修复原则上增加修复前会失败的回归测试；
- 不允许空 `catch`、吞错、伪成功、硬编码演示结果或静默安全降级；
- TODO 必须有关联阻塞条件或跟踪项，不能替代当前需求；
- 架构、协议、用户行为或验收方式变化时同步技术架构或测试文档。

删除或覆盖大量文件、数据库变更、生产接口调用、Git 历史重写、系统权限和安全策略修改，以及新增或升级生产依赖，必须在 Issue 或 Pull Request 中说明目标、影响、恢复方式和验证计划，并在合并前完成维护者审查。AI 与自动化工具执行本地高风险操作时，还必须遵循 `AGENTS.md` 的事前确认要求。

## 8. 依赖与供应链

新增或升级生产依赖前必须确认：

1. 标准库或现有依赖不能合理解决；
2. 项目仍维护且许可证与 MIT 兼容；
3. 支持 macOS 和 Windows；
4. 是否进入安装包、接触凭据、网络或执行外部代码；
5. 锁文件、CI 和失败路径如何覆盖。

未使用依赖应删除。不得通过关闭 TLS、主机密钥、权限或输入校验换取兼容性。

## 9. Code Review

审查先寻找正确性、安全、数据和生命周期问题，格式问题交给自动化工具。问题级别：

- **P0 阻断**：凭据泄露、任意命令执行、不可恢复数据损坏、默认关闭安全校验；
- **P1 严重**：主流程错误、资源泄漏、竞态、死锁、错误目标执行或跨平台核心能力不可用；
- **P2 一般**：边界破坏、错误语义不稳定、测试缺失或明显维护风险；
- **P3 建议**：不影响当前正确性的可读性或局部简化。

每条 Finding 必须包含文件位置、触发条件、影响和可执行修复方向。重点检查：

- session / task ID 是否绑定正确目标，迟到事件是否可能覆盖新状态；
- 失败、取消、超时、窗口关闭和异常退出是否释放进程、连接、临时文件与监听器；
- SSH 主机密钥、认证、远程路径和 SFTP 覆盖/取消是否安全；
- SQLite Migration 是否保留原数据，凭据是否可能进入日志、IPC 或快照；
- Rust DTO 与 TypeScript 类型是否同步，UI 是否只依赖稳定错误码；
- AI Provider 是否绕过统一 Gateway、审批或目标绑定；
- 测试与文档是否把静态通过误写为真实平台验收。

审查结论必须说明阻断问题、自动检查、真实运行和未验证范围。没有 Finding 时仍需指出剩余风险。

## 10. 验证证据

验证结果按以下等级描述，不得混用：

| 等级       | 可声称内容                                 |
| ---------- | ------------------------------------------ |
| 已检查     | 阅读源码、配置、diff 或文档                |
| 已静态验证 | 格式、Lint、类型检查或 Clippy 通过         |
| 已自动测试 | 明确列出的单元或集成测试通过               |
| 已构建     | Web、Rust 或 Tauri 构建成功                |
| 已运行     | 应用或服务实际启动并保持稳定               |
| 已功能验收 | 在真实目标环境完成规定交互并观察到预期结果 |

最低验证要求：

| 变更                          | 最低要求                                             |
| ----------------------------- | ---------------------------------------------------- |
| 文档                          | Prettier、本地链接、命令人工核对、`git diff --check` |
| TypeScript 纯逻辑             | Lint、类型检查、相关 Vitest                          |
| React UI                      | 前端门禁、真实浏览器或 Tauri 视觉与交互检查          |
| Rust Domain / Application     | rustfmt、Clippy、相关单元测试                        |
| IPC / DTO / Event             | 前后端契约测试、真实 Tauri 链路                      |
| SQLite Migration              | 新建库、逐版本升级、失败与数据保留                   |
| 终端 / SSH / SFTP / 凭据 / AI | 自动测试及 `docs/testing.md` 中对应真实流程          |

环境无法满足真实验证时，可以交付实现，但必须明确标为未完成验收并给出准确步骤。最终报告应区分已检查、已静态验证、已自动测试、已构建、已运行和已功能验收。
