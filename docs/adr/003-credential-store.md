# ADR-003：凭据通过平台 Adapter 隔离

- 状态：Accepted
- 日期：2026-08-14
- 决策人：Nocterm maintainers

## 背景

macOS 与 Windows 的安全凭据设施不同，而连接、SSH 和未来 Agent 不应理解平台 API。两种平台实现必须通过统一契约提供对等的安全语义和错误行为。

## 决策

Domain 定义 `CredentialStore` 语义；Infrastructure 分别实现 macOS Keychain 和 Windows Credential Manager Adapter。数据库只保存不可逆推出密钥的逻辑引用。前端允许提交写入、替换和删除请求，常规读取接口不得返回明文。

具体使用系统命令、Rust crate 或平台 FFI，将在实现前通过安全性和兼容性原型决定，不改变上层接口。

## 后果

连接删除、凭据替换和失败回滚必须编排数据库与系统凭据两类资源。实现需要覆盖残留凭据清理、权限拒绝和凭据不存在的幂等行为。

## 验证

- macOS/Windows 分别完成保存、认证、替换和删除；
- SQLite、日志和 IPC 事件中检索不到测试密钥；
- 凭据读取失败返回稳定错误码，不把平台原始信息直接暴露给 UI。
