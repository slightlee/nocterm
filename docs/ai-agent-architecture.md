# Nocterm AI 面板技术架构

- 状态：已实现，真实 Provider/SSH 矩阵持续验收
- 适用范围：Nocterm 桌面端中的本机 AI Provider、当前本地终端和当前 SSH 终端
- 关联测试：[AI Provider 与 Nocterm Bridge 验收流程](testing/ai-provider-smoke.md)

## 1. 目标与边界

Nocterm 不重新实现模型、Agent loop、Skills 或通用 MCP 客户端。用户已经安装的 Codex、Claude Code、Grok 等 CLI 继续负责模型调用、推理、上下文编排和登录状态。Nocterm 负责把这些 Provider 安全地接入当前终端产品：

- 在 AI 面板中启动和管理 Provider；
- 将本轮任务绑定到发送时选中的本地终端或 SSH 连接；
- 提供结构化的服务器诊断能力和受策略控制的通用命令能力；
- 在本地 PTY 或已认证 SSH 连接中执行，并把真实结果回传；
- 处理权限、审批、取消、超时、审计和资源清理。

首版不包含 SFTP AI 工具、MCP 市场、自研 Agent、Provider GUI 集成或远程服务器端 Agent。浏览器预览只能展示 UI，Provider 和终端执行必须在 Tauri 桌面端运行。

## 2. 方案选型

采用 **Provider Adapter + 会话级能力代理 + 结构化工具/通用命令双路径**：

```text
AI 面板
  -> Provider Adapter
  -> Codex / Claude Code / Grok CLI
  -> Nocterm MCP/CLI Bridge（任务 token）
  -> Tool Gateway
  -> Application Services
  -> 当前本地 PTY 或当前已认证 SSH exec channel
```

这个方案借鉴了同类终端产品的共同做法：复用成熟 Agent，把终端应用拥有的会话、审批和远程连接作为受控工具提供给 Agent；不把 SSH 凭据交给 Provider，也不让 Provider 自己选择另一台主机。与单纯的 MCP Server 相比，Provider Adapter 还负责 CLI 发现、启动、流式输出、持续会话和取消；与自研 Agent 相比，避免重复实现模型和 Agent 生态。

## 3. 组件职责

| 组件                           | 负责                                                  | 不负责                           |
| ------------------------------ | ----------------------------------------------------- | -------------------------------- |
| `AiPanel`                      | Provider 选择、会话 UI、流式消息、审批卡片、停止      | 模型调用、SSH 凭据、命令安全判断 |
| `ProviderAdapter`              | CLI 发现、参数和配置注入、进程生命周期、Provider 输出 | SSH 连接、权限策略、业务工具实现 |
| `AiGateway`/`ToolGateway`      | token 校验、目标绑定、工具清单、调用限流和路由        | Provider 私有协议和 React 状态   |
| `ai_tools`                     | 封闭 Schema、参数校验、固定命令计划、结构化诊断       | 任意 Shell 解析和凭据管理        |
| `ai_policy`/`ai_tool_approval` | 权限决策、审批状态、超时、取消、审计前置              | Agent 推理和自然语言回答         |
| Application/Infrastructure     | PTY、SSH、退出状态、凭据和资源生命周期                | Provider 文案和模型上下文        |

Rust 业务仍遵守 `desktop -> application -> domain` 与 `infrastructure -> domain` 边界。Tauri Command 只负责 IPC 校验、状态访问和 DTO 转换；Domain 不依赖 Tauri、SQLite 或操作系统 API。

## 4. 目标绑定与数据流

发送请求时，后端重新确认目标状态并创建一次任务绑定：

1. UI 只传 `connectionId` 或 `targetSessionId`，不能同时传两者；目标必须在发送时处于 connected 状态。
2. 后端生成 256 位随机 token，将 token 绑定到 Provider、任务 session 和目标。
3. Provider 只能通过当前任务注入的 Bridge 调用 Nocterm 工具；工具调用不接受调用方覆盖目标。
4. Provider 退出、turn 结束、停止、目标变化或会话重置时，token、审批和临时资源一起撤销。
5. 迟到的输出、退出事件和审批事件必须匹配当前 session，不能覆盖新会话。

### 本地终端

本地命令写入用户当前可见的 PTY，继承当前 Shell 的目录和环境。Infrastructure 按 POSIX Shell、Fish、PowerShell 或 CMD 生成随机完成探针；只有完整标记和真实数字退出码都收到后才认为执行结束。探针、退出码和 Shell 重绘片段在输出发送到 UI 前过滤；同一 PTY 的 AI 命令串行执行。

### SSH 终端

SSH 工具复用 Nocterm 当前已认证连接的独立 exec channel，不写入用户可见 PTY，也不从凭据库重新读取密码或私钥建立隐藏连接。独立 exec 不继承可见 Shell 的临时 `cd`、`export` 或虚拟环境；Agent 必须显式建立所需上下文。执行结果使用 SSH 协议提供的真实 `exit-status`，没有退出状态时按未知失败处理。

## 5. Provider 适配

三个 Provider 具有独立适配实现，公共接口只描述 Nocterm 需要的生命周期计划：

- **Codex**：使用 `codex app-server --stdio`。同一 UI 对话复用临时内存 thread 和 app-server；每个 turn 重新激活工具授权。目标绑定时关闭内置 Shell/unified exec，并将 Nocterm MCP 设为必需能力。
- **Claude Code**：使用 headless `-p` 和 `stream-json`。Prompt 通过 stdin 传递；使用严格 MCP 配置、空内置工具列表和无会话持久化，避免自带 Bash 或用户配置旁路 Nocterm。
- **Grok**：使用 headless `streaming-json`。由于缺少同等严格的 MCP 配置开关，为每个任务创建私有 `GROK_HOME` 和工作目录，关闭兼容 MCP、Hook、memory、subagent 和 Web，只链接已有 `auth.json`，任务结束后删除目录。

Provider 输出由统一有界读取器处理：单行最大 1 MiB，非法 UTF-8、读取错误和超长输出都显式失败，不能被当作正常 EOF。

## 6. 工具模型

### 结构化服务器工具

SSH 目标提供固定工具和封闭 JSON Schema，后端根据参数生成命令，不接受完整 Shell：

- 系统信息、进程、监听端口；
- systemd 服务状态和受限日志；
- 磁盘、内存；
- Docker 信息、镜像加速器、容器列表、容器状态和受限日志。

工具参数拒绝未知字段、越界数值、选项注入和非法服务/容器标识。Docker 诊断不返回环境变量、密码或 Token。远端没有 Docker、systemd、`ss` 或权限不足时，返回真实错误和退出状态，不编造结果。

### 通用命令工具

结构化工具无法完成任务时，才提供一个目标绑定的通用命令入口。命令本身不能决定“只读”，因为同一命令名的参数可能产生副作用。因此默认策略只自动放行明确的无参数安全命令：`pwd`、`ls`、`id`、`whoami`、`uname`、`hostname`、`date`、`uptime`。复杂参数、Shell 组合符号、未知或写入命令进入审批路径。

## 7. 权限与审批

权限按当前会话单调递增：

| 策略       | 行为                                                       |
| ---------- | ---------------------------------------------------------- |
| 仅分析     | 不执行终端操作，只基于已有上下文回答                       |
| 每次确认   | 结构化工具和通用命令都逐次确认                             |
| 变更前确认 | 自动执行结构化只读工具和固定无参数安全命令，其他操作确认   |
| 完全访问   | 当前会话内跳过确认，但仍受目标、限流、超时、取消和审计约束 |

完全访问不持久化，新会话恢复为“变更前确认”。审批请求绑定 `approvalId + sessionId`，后端拒绝迟到、重复、旧会话或并行覆盖。审批最多等待 60 秒；停止任务会撤销审批，过期卡片不能执行命令。执行前写入 pending 审计，完成后写入结果；审计失败时宁可阻止执行或返回结果未知，也不伪装成功。

## 8. 超时、取消和审计

- 审批预算：60 秒；实际执行预算：60 秒；完整工具调用：120 秒；Bridge 响应：135 秒。
- 每个任务 token 每分钟最多调用 30 次。
- SSH 输出限制为 128 KiB；本地 PTY 超限、标记缺失或退出状态非法均按失败处理。
- Bridge 请求发送后断线时返回“结果未知”，禁止自动重放可能产生副作用的命令。
- SQLite v6 审计只保存时间、Provider、任务、目标类型、连接 ID、固定工具名、审批状态、结果、耗时和稳定错误码；不保存命令、参数、输出、Token、主机、账号或凭据。
- 审计按 30 天和最多 10,000 条双重上限清理。

## 9. UI 生命周期

AI 面板折叠和路由切换只改变可见性，不卸载运行中的 Provider、事件监听或审批状态。事件监听注册失败时禁止启动新任务。Provider 输出、退出、审批关闭和错误都按 session 过滤；停止按钮、重试、切换 Provider、新建/删除会话都会经过统一清理路径。

## 10. 当前状态与限制

已实现：Provider 独立适配、Codex 持续 thread、Claude/Grok headless、MCP Bridge、目标绑定、本地 PTY、SSH exec、结构化服务器/Docker 诊断、四级权限、审批过期、取消、限流、脱敏审计和有界输出。

仍需真实验收：每个 Provider 的实际登录和模型请求、macOS/Windows Tauri UI、真实 SSH 认证矩阵、不同 Linux 发行版的 `systemd`/Docker/`ss` 能力差异。静态检查和自动测试不能替代这些验收，执行步骤见 [测试文档](testing/ai-provider-smoke.md)。

## 11. 开发约束

新增 Provider 时实现独立 Adapter 和对应契约测试，不把厂商参数散落到 UI 或终端领域。新增服务器能力时优先添加结构化工具、封闭 Schema、固定命令计划、错误映射和审计类型；不要通过扩大只读白名单或允许 Provider 自带 Shell 来“修复”兼容性。任何涉及凭据、SSH、PTY、审批和资源清理的修改，都必须补充失败路径和取消路径测试。
