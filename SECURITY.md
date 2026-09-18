# 安全策略

## 支持范围

安全修复优先应用到默认分支，并随下一个可分发版本发布。维护者只主动支持 [GitHub Releases](https://github.com/slightlee/nocterm/releases) 中最新的公开版本；预发布版本只在对应 Release Notes 声明的测试范围内支持。旧版本、未发布提交和第三方 Provider CLI 不承诺独立维护。

## 私下报告漏洞

请使用 GitHub 的 [Private Security Advisory](https://github.com/slightlee/nocterm/security/advisories/new) 报告以下问题，不要创建公开 Issue：

- 密码、私钥、Token 或连接信息泄露；
- SSH 主机密钥校验绕过、认证绕过或权限提升；
- AI Provider 绕过 Nocterm 目标绑定、审批或工具网关；
- 任意命令执行、路径穿越、文件覆盖或数据损坏；
- Provider、终端、SSH 或 SFTP 进程和资源无法回收，造成安全影响；
- 安装包、更新或依赖供应链风险。

报告中请包含受影响版本或 Commit、平台、复现步骤、影响范围和最小化的脱敏证据。不要上传真实凭据、生产数据库、完整环境变量、用户目录或未脱敏日志。

维护者确认问题后会在私有 Advisory 中沟通复现、影响、修复和披露计划。修复发布前请不要公开细节。

## 安全边界

- 密码由 macOS Keychain 或 Windows Credential Manager 保存；SQLite 只保存凭据引用；
- 私钥内容不进入 SQLite 或系统凭据库，连接资料只保存本地文件路径；
- SSH 主机密钥采用首次信任、变更拒绝的 TOFU 策略；
- AI Provider 只可通过任务绑定的 Nocterm 工具网关访问当前终端，不能获得 SSH 凭据或覆盖目标；
- AI 完全访问只在当前会话有效，仍受目标绑定、限流、超时、取消和审计约束；
- Provider 与模型服务属于第三方信任边界，其数据处理规则由用户所选 Provider 决定。

不要通过关闭 TLS、主机密钥检查、权限校验或审批机制来规避故障。发现疑似漏洞时，先停止在生产目标复现并使用专用测试环境收集证据。
