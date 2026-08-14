# ADR-001：采用模块化单体与三层 Rust Workspace

- 状态：Accepted
- 日期：2026-08-14
- 决策人：Nocterm maintainers

## 背景

Nocterm 需要在一个桌面应用内承载连接、本地终端、SSH、SFTP 和本地数据，并保持 UI、IPC、系统进程与持久化之间的清晰边界。当前没有独立部署多个服务的需求。

## 决策

采用模块化单体：React 按 Feature 组织；Rust 固定为 `nocterm-domain`、`nocterm-application`、`nocterm-infrastructure` 三个核心 crate，Tauri Host 作为组合根。

Domain 定义稳定模型和平台 Port；Application 编排用例；Infrastructure 实现 SQLite、PTY、凭据、SSH/SFTP 和平台能力；Tauri Host 不写业务逻辑。

## 备选方案

- 单个 Rust crate：起步简单，但无法通过编译边界阻止平台和数据库依赖进入领域。
- 每个功能一个 crate：隔离更强，但早期 crate 数量和跨模块 DTO 成本过高。
- 独立本地服务：增加进程、端口、部署和安全成本，没有当前收益。

## 后果

三个 crate 带来少量装配代码，但依赖边界可由 Cargo 直接约束。只有形成稳定独立模型时才允许增加 crate，避免为了分层继续拆分。

## 验证

- `cargo check --workspace` 能独立编译所有 crate；
- `nocterm-domain/Cargo.toml` 不出现 Tauri、rusqlite 和平台依赖；
- `health_check` 通过 Infrastructure Port → Application → Tauri DTO 完成端到端调用。
