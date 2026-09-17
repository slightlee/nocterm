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
  -> Nocterm MCP（任务 token）
  -> Tool Gateway
  -> Application Services
  -> 当前本地 PTY 或当前已认证 SSH exec channel
```

这个方案借鉴了同类终端产品的共同做法：复用成熟 Agent，把终端应用拥有的会话、审批和远程连接作为受控工具提供给 Agent；不把 SSH 凭据交给 Provider，也不让 Provider 自己选择另一台主机。与单纯的 MCP Server 相比，Provider Adapter 还负责 CLI 发现、启动、流式输出、持续会话和取消；与自研 Agent 相比，避免重复实现模型和 Agent 生态。

## 3. 组件职责

| 组件                           | 负责                                              | 不负责                           |
| ------------------------------ | ------------------------------------------------- | -------------------------------- |
| `AiPanel`                      | Provider 选择、会话 UI、流式消息、审批卡片、停止  | 模型调用、SSH 凭据、命令安全判断 |
| `ProviderAdapter`              | CLI 发现、参数和配置注入、厂商协议、Provider 输出 | SSH 连接、权限策略、业务工具实现 |
| `PersistentSessionRegistry`    | 持续会话缓存、启动互斥、取消兜底、迟到退出隔离    | 厂商 JSON-RPC/ACP 消息解析       |
| `ManagedChild`                 | 子进程退出探测、幂等终止和回收                    | Provider 配置和对话语义          |
| `AiGateway`/`ToolGateway`      | token 校验、目标绑定、类型化执行、调用限流和路由  | Provider 私有协议和 React 状态   |
| `ai_tools`                     | 封闭 Schema、参数校验、固定命令计划、结构化诊断   | 任意 Shell 解析和凭据管理        |
| `ai_policy`/`ai_tool_approval` | 权限决策、审批状态、超时、取消、审计前置          | Agent 推理和自然语言回答         |
| Application/Infrastructure     | PTY、SSH、退出状态、凭据和资源生命周期            | Provider 文案和模型上下文        |

Rust 业务仍遵守 `desktop -> application -> domain` 与 `infrastructure -> domain` 边界。Tauri Command 只负责 IPC 校验、状态访问和 DTO 转换；Domain 不依赖 Tauri、SQLite 或操作系统 API。

## 4. 目标绑定与数据流

发送请求时，后端重新确认目标状态并创建一次任务绑定：

1. AI 面板只服务当前终端；UI 只传 `connectionId` 或 `targetSessionId`，不能同时为空或同时存在。目标必须在发送时处于 connected 状态，未连接时前端保留草稿并阻止发送，后端再次校验并拒绝无目标启动，不能静默退化到 Provider 宿主环境。
2. 后端生成 256 位随机 token，将 token 绑定到 Provider、任务 session 和目标。
3. Provider 只能通过当前任务注入的 Nocterm MCP 调用统一 Tool Gateway；调用方不能覆盖目标。
4. Provider 退出、turn 结束、停止、目标变化或会话重置时，token、审批和临时资源一起撤销。
5. 迟到的输出、退出事件和审批事件必须匹配当前 session，不能覆盖新会话。

### 本地终端

本地命令写入用户当前可见的 PTY，继承当前 Shell 的目录和环境。Infrastructure 按 POSIX Shell、Fish、PowerShell 或 CMD 生成随机完成探针；只有完整标记和真实数字退出码都收到后才认为执行结束。探针、退出码和 Shell 重绘片段在输出发送到 UI 前过滤；同一 PTY 的 AI 命令串行执行。

### SSH 终端

SSH 工具复用 Nocterm 当前已认证连接的独立 exec channel，不写入用户可见 PTY，也不从凭据库重新读取密码或私钥建立隐藏连接。独立 exec 不继承可见 Shell 的临时 `cd`、`export` 或虚拟环境；Agent 必须显式建立所需上下文。执行结果使用 SSH 协议提供的真实 `exit-status`，没有退出状态时按未知失败处理。

## 5. Provider 适配

三个 Provider 具有独立适配实现。Codex 和 Grok 共享 `PersistentSessionRegistry` 与 `ManagedChild`：公共层只管理对话级缓存、启动槽位、generation、取消兜底和进程回收；Provider Adapter 仍各自负责握手、消息解析、完成条件和协议中断。这样新增持续 Provider 时可以复用生命周期骨架，又不会把不兼容的厂商协议塞进条件分支。

- **Codex**：使用 `codex app-server --stdio`。同一 UI 对话复用临时内存 thread 和 app-server；每个 turn 重新激活工具授权。初始化后通过带当前 cwd 的 `config/read` 探测全局与项目生效配置，thread 覆盖关闭既有 MCP、内置 Shell 能力门、Code Mode、浏览器、插件、应用及其他执行旁路，跳过宿主 Skill 与项目指令发现，注入唯一且必需的 Nocterm MCP，并固定为只读沙箱；创建后通过 thread 响应和 `experimentalFeature/list` 验证最终策略，旧版本不支持安全启动链时明确失败。`unified_exec` 只是 Shell 实现选择器，`shell_tool=false` 时 Codex 不注册 legacy 或 unified 执行工具。
- **Claude Code**：使用 headless `-p` 和 `stream-json`。Prompt 通过 stdin 传递；使用严格 MCP 配置、空内置工具列表和无会话持久化，避免自带 Bash 或用户配置旁路 Nocterm。绑定终端时进程在任务级私有空目录中运行，输出只有在本轮真实调用 Nocterm Gateway 后才发布；未调用工具的模型猜测按失败丢弃。
- **Grok**：最低支持版本为 `1.0.34`，使用 `grok agent stdio` 的官方 ACP 持续会话和 MCP-over-ACP。初始化响应必须声明 `_meta["x.ai/mcp/sdk"] = true`；版本过低、版本无法识别或能力缺失时，会话启动失败并在 AI 面板提示升级，不回退到第二套 MCP 传输。Nocterm 在 `session/new` 的 `_meta["x.ai/mcp/servers"]` 中注册唯一的进程内 MCP，并处理 Grok 发回的 MCP SDK 反向调用；该扩展在官方源码中的逻辑方法名为 `x.ai/mcp/sdk_call`，ACP stdio 线路编码为 `_x.ai/mcp/sdk_call`。MCP JSON-RPC 在 Tauri 进程内直接进入公共 Tool Gateway，不启动额外 Bridge 子进程。ACP 负责会话、流式事件和取消，不启用 ACP Terminal、客户端文件能力或 Grok 自带 Shell。同一 UI 对话复用进程和 session，每个 turn 单独激活授权并通过 `session/cancel` 停止。Grok 的模型调用协议要求先用 `search_tool` 取得 Schema，再用 `use_tool` 调用，因此一次精确能力发现是正常路径；Adapter 向模型提供当前目标的固定工具目录，要求单轮合并搜索所需工具并禁止重复探索。对话级私有 `GROK_HOME`、`HOME`、`USERPROFILE` 和工作目录隔离用户 MCP、Skills、Hook、memory、subagent 与 Web；只链接已有 `auth.json`，并仅投影当前模型配置。协议依据为 [xAI 官方 grok-build](https://github.com/xai-org/grok-build) 中的 ACP session 与 MCP reverse transport 实现。

### 统一入口约束

- 三个 Provider 的终端请求都必须进入同一个 Nocterm MCP 工具目录和 Tool Gateway；不得为单个 Provider 建立第二套终端执行协议。
- Provider Adapter 只处理 CLI 启动、配置隔离、MCP 注入、厂商会话协议、流式输出和取消，不实现 SSH、PTY、审批、审计或命令策略。
- MCP 工具定义、结果契约、目标绑定和权限策略只有一个来源。新增 Provider 只增加 Adapter，不复制工具实现或修改其他 Provider 的业务逻辑。
- Provider 自身协议造成的调用步骤允许不同：Codex、Claude Code 可直接调用注入工具；Grok 需要一次 `search_tool` 后通过 `use_tool` 调用。该差异只能留在 Grok Adapter 的提示与协议测试中，不能扩散到公共执行核心。
- MCP 传输由 Provider 协议决定，但只能复用公共分发器：Codex、Claude Code 使用 stdio MCP 到回环 Gateway；Grok 仅使用官方 MCP-over-ACP。所有传输必须产生相同的 MCP 结果、审批事件、审计记录、限流和取消语义。

Grok 的 MCP-over-ACP 采用官方 half-duplex v1 约束：只接受带 JSON-RPC `id` 的客户端到服务器请求及其响应，不依赖 MCP notification、sampling、roots 或服务器主动请求。Nocterm 当前工具目录只包含同步的 `initialize`、`tools/list` 和 `tools/call`，满足该约束；未来工具需要双向 MCP 能力时，必须先升级协议并补充兼容性测试，不能静默丢失消息。

Provider 输出由统一有界读取器处理：单行最大 1 MiB，进程事件队列容量为 128，stdout/stderr 合计的单轮输出最大 4 MiB；非法 UTF-8、读取错误、单条或累计超限都显式失败并终止对应任务，不能被当作正常 EOF 或继续无界占用内存。

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

AI 面板折叠和路由切换只改变可见性，不卸载运行中的 Provider、事件监听或审批状态。事件监听注册失败时禁止启动新任务。Provider 输出、退出、审批关闭和错误都按 session 过滤；停止按钮、重试、切换 Provider、新建/删除会话都会经过统一清理路径。Adapter 将厂商事件统一为回答、工具动作和错误；Provider 的原始 reasoning/thinking 属于私有推理，不进入消息、历史或执行过程。执行过程只展示可审计的 Nocterm 工具动作与必要的进度叙述。

## 10. 当前状态与限制

已实现：Provider 独立适配、Codex 持续 thread、Grok 持续 ACP session、Claude Code headless、统一 MCP Tool Gateway、目标绑定、本地 PTY、SSH exec、结构化服务器/Docker 诊断、四级权限、审批过期、取消、限流、脱敏审计和有界输出。Codex、Claude Code 通过 stdio MCP 接入；Grok `1.0.34` 及以上通过官方 MCP-over-ACP 接入，传输层不拥有工具业务逻辑。

仍需真实验收：每个 Provider 的实际登录和模型请求、macOS/Windows Tauri UI、真实 SSH 认证矩阵、不同 Linux 发行版的 `systemd`/Docker/`ss` 能力差异。静态检查和自动测试不能替代这些验收，执行步骤见 [测试文档](testing/ai-provider-smoke.md)。

## 11. 开发约束

新增 Provider 时实现独立 Adapter 和对应契约测试，不把厂商参数散落到 UI 或终端领域，也不得绕开统一 Nocterm MCP 新增 Provider 专属执行通道。新增服务器能力时优先添加结构化工具、封闭 Schema、固定命令计划、错误映射和审计类型；不要通过扩大只读白名单或允许 Provider 自带 Shell 来“修复”兼容性。任何涉及凭据、SSH、PTY、审批和资源清理的修改，都必须补充失败路径和取消路径测试。
