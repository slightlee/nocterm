# Repository Guidelines

## 指令范围与必读文档

本文件约束仓库内所有 AI 与人工开发。开始工作前先读取与任务直接相关的源码、测试和文档，不得仅依据文件名、过期方案或经验推断实现。架构或新功能任务必须阅读 `docs/technical-architecture.md`、`docs/development.md` 和 `docs/ai-development-policy.md`；涉及 SSH/SFTP、凭据或跨平台能力时，还必须阅读对应 ADR、`docs/testing/` 和 `docs/roadmap/capability-matrix.md`。详细审查规则见 `docs/code-review.md`。

## 项目结构

Nocterm 是模块化单体。`apps/desktop/src` 是 React UI，按 `app → features → shared` 单向依赖；`apps/desktop/src-tauri` 仅负责 Tauri 启动、IPC、DTO、事件与依赖装配。Rust 业务位于 `crates/`：`nocterm-domain` 保存领域模型与 Port，`nocterm-application` 编排用例，`nocterm-infrastructure` 实现 SQLite、终端、SSH、SFTP 和平台适配。

## AI 工作流程

1. 先检查当前文件、相关调用链、测试和工作区状态，保留用户已有改动。
2. 复杂、跨模块或存在架构取舍的任务先给出计划；需求明确的小改动直接实施。
3. 只修改完成当前目标必需的内容；禁止顺手重构、猜测性修复和无消费者的未来抽象。
4. 优先修复根因；新增行为必须补充测试，Bug 修复原则上增加回归测试。
5. 修改后检查 diff、运行与风险相称的验证；实现影响架构、协议或验收方式时，同步对应技术方案、ADR、能力矩阵或测试说明。
6. 最终报告区分“已检查”“已构建”“已自动测试”“已真实运行”和“未验证”。不得把静态通过表述为功能已验收。

## 架构与代码边界

- Feature 只能通过各自 `index.ts` 暴露能力，不得跨 Feature 引用内部文件；`shared` 不依赖 Feature。
- 业务 UI 不得直接调用 `@tauri-apps/api`，IPC 调用统一收口到 Feature `api/`。
- Domain 禁止依赖 Tauri、SQLite、文件系统和操作系统 API；Infrastructure 实现 Port。
- Tauri Command 保持轻薄，只做校验、权限检查、用例调用和 DTO 转换。
- `cfg(target_os)` 只允许存在于平台 Adapter 和依赖装配处；公共模型不得泄露平台专用类型。
- UI 依赖稳定错误码，不解析 stderr；禁止空 `catch`、吞错、伪成功返回和无期限 TODO。
- 遵循 KISS、YAGNI、DRY、SOLID；不得为了形式上的“企业架构”制造空接口、万能 `utils` 或循环依赖。

## 注释与文档

新增或修改的非生成业务源码，有效注释行占非空代码行的比例原则上不低于 10%，且具有实质业务逻辑的文件不得完全没有注释。10% 是评审基线，不是单独合格标准；公开 API、Trait/Port、状态机、并发与资源清理、平台差异、安全边界、协议兼容、复杂算法和非显然取舍必须说明设计意图、约束与失败行为。禁止用逐行翻译、重复命名、无信息量块注释和废弃代码凑比例。纯 DTO、类型声明、导出文件、声明式配置、生成代码和简单短测试可合理豁免。注释语言与所在模块保持一致。

## 安全与依赖

- 密码、私钥、Token、完整环境变量和未脱敏诊断信息不得进入源码、SQLite、日志、测试快照和 IPC 事件。
- 不得通过关闭 TLS、主机密钥检查、权限校验或错误处理来换取功能可用。
- 删除、批量移动、数据库结构变更、生产接口调用及其他难以恢复的操作，执行前必须说明范围和风险并获得明确确认。
- 新增或升级生产依赖前必须说明必要性、许可证、维护状态、跨平台影响和替代方案，并获得确认；不得全局安装工具。
- 不复制来源或许可证不明确的代码；生成文件只由对应工具更新，不手工修改。

## 开发、验证与完成定义

使用 Node 22、pnpm 11 和 stable Rust。常用命令：

- `corepack pnpm dev`：启动浏览器预览。
- `corepack pnpm tauri dev`：启动桌面客户端。
- `corepack pnpm check`：前端格式、Lint、样式、测试和构建。
- `corepack pnpm cargo:check`：Rust 格式、Clippy 和测试。

UI 修改必须做浏览器或 Tauri 真实运行检查；终端、凭据、SSH/SFTP 必须执行 `docs/testing/` 中对应流程。只在实际目标系统运行后才能声称 macOS 或 Windows 已验收。任务只有在需求实现、架构边界保持、相关检查通过、必要运行验证完成、文档同步且未验证范围明确时才算完成。

## Code Review 重点

审查时优先寻找安全泄露、数据损坏、资源未释放、并发竞争、终端/传输生命周期错误、跨平台漂移、IPC 契约不一致、错误被吞和缺少失败路径测试。格式问题交给自动化工具；审查意见必须给出文件位置、影响和可执行修复方向。

## Git 约束

未经用户明确要求，不执行 Git 初始化、提交、推送、重置、rebase、分支创建或切换。提交前必须检查已跟踪、已暂存和未跟踪文件，确认准确范围；提交信息必须符合 `docs/development.md` 的 Conventional Commits 规范；禁止使用 `--no-verify` 绕过门禁，禁止覆盖或丢弃用户现有改动。
