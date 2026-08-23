# ADR-002：SSH/SFTP 先完成跨平台原型再锁定实现

- 状态：Accepted
- 日期：2026-08-14（2026-08-21 更新为已接受）
- 决策人：Nocterm maintainers

## 背景

Nocterm 需要同时支持 macOS 与 Windows，并为 SSH、SFTP 提供稳定错误、取消、资源清理和主机密钥语义。系统 OpenSSH 能复用用户 SSH 配置，但 Windows 一致性和结构化 SFTP 能力仍需验证；原生 Rust 库则会引入 SSH Agent、跳板机、算法兼容和主机密钥处理成本。

## 候选方案

1. 系统 OpenSSH：macOS `/usr/bin/ssh`，Windows `ssh.exe`；
2. 原生 Rust SSH/SFTP 实现；
3. SSH 使用系统 OpenSSH，SFTP 使用独立原生 Adapter。

## 决策门槛

阶段 1 前制作最小原型，并在 macOS/Windows 验证：密码、私钥、SSH Agent、首次主机密钥、密钥变化、ProxyJump、非 22 端口、Unicode 路径、大文件、取消和断网清理。

只有原型证据齐全后把本文状态改为 Accepted。正式业务代码只能依赖 `SshBackend`/`SftpBackend`，不能依赖候选实现的具体类型。

## 决策（2026-08-21）

选择方案 2（原生 Rust 实现），采用 [`russh`](https://github.com/Eugeny/russh) 0.63 + `russh-sftp` 2.4。原因：

- **Windows 阻断问题**：系统 OpenSSH 在 Windows 上从控制台而非 stdin/PTY 读取密码，无法自动注入已保存口令；且 Windows OpenSSH 不支持 `ControlMaster` 多路复用，SFTP 复用终端连接的方案在 Windows 上不可行。`sshpass` 在 Windows 亦不可用。
- **主流验证**：russh 由 Tabby 作者维护，Tauri 生态（如 r-shell、vscode 相关工具）已用其构建进程内 SSH，属市场主流方案。
- **加密后端**：使用 `ring` 而非默认 `aws-lc-rs`，规避 Windows 上额外的 NASM/CMake 原生构建依赖。

### 落地形态

- 基础设施新增 `nocterm-infrastructure/src/ssh/mod.rs`，其中 `SshTerminalManager` 实现领域 `SshTerminalPort`：共享单一 tokio 运行时，每终端一个 tokio 任务独占 russh 连接句柄与通道，指令经 `tokio::sync::mpsc` 投递，通道输出经 `std::sync::mpsc` 桥接为同步 `Read` 以保持 Port 的同步契约。
- 认证覆盖密码（含 keyboard-interactive 回退）、私钥、SSH Agent（Unix 走 `SSH_AUTH_SOCK`，Windows 走 OpenSSH 命名管道）。
- 主机密钥采用 TOFU：首次记录、变更即拒，绝不关闭校验。
- 旧的“shell 出 `ssh.exe` + 监听 PTY 注入密码”实现（连同 `build_command`/`StrictHostKeyChecking=ask`/临时私钥落盘）已移除。`LocalTerminalManager`（本地 PTY）保持不变。

### 阶段 2（2026-08-22）：SFTP 迁移完成

- 基础设施新增 `nocterm-infrastructure/src/ssh/sftp.rs`，其中 `SftpManager` 以 `connection_id` 为键维护进程内 SFTP 会话池，复用 `ssh/mod.rs` 的 `connect_authenticated_coded` 与主机密钥 TOFU 逻辑：认证成功后打开会话通道、`request_subsystem("sftp")`，在其上建立 `SftpSession`。
- 同步 Port 契约通过“共享多线程 tokio 运行时 + `runtime.spawn` + `std::sync::mpsc` 回传”桥接，避免在工作线程上 `block_on` 引发 panic。
- 远程目录浏览、存在性判断、建目录、重命名、递归删除、上传、下载全部走进程内 SFTP，不再 shell 出 `ssh`/`sftp`/`tar`，也不再依赖 `ControlMaster`；`commands/sftp.rs` 内的 `SecretDirectory`、临时私钥落盘、远端 `tar` 打包与输出解析均已删除。
- 传输语义：上传文件先写 `.{name}.nocterm-{seq}.part` 临时名再原子提交（含备份与回滚），提交前检查取消；下载先落本地暂存路径，由命令层完成原子替换。目录传输递归镜像，进度以 `AtomicU64` 累计字节由命令层每 40 ms 轮询上报，取消为协作式逐块检查。
- 下载的路径类型以远程 `stat` 为准而非前端提示，避免前端状态过期导致的误判。
- 会话生命周期：删除连接、关闭会话、应用退出时先等待在途传输退出，再剔除会话池条目释放底层 russh 连接。

### 阶段 3（2026-08-22）：密码认证改为终端内交互输入

进程内 russh 必须在传输层认证**之前**拿到完整口令，无法像系统 OpenSSH 那样把 `password:` 提示交给远端 PTY。因此“未保存密码就无法连接”一度成为硬前置条件，而连接表单里密码本就是可选项，用户选了密码登录却被直接拒绝。主流进程内客户端都在客户端侧收集口令：PuTTY/WindTerm 在终端里就地提示，Xshell/Termius/Tabby 弹独立对话框。本项目取前者，与终端形态一致。

- 前端新增 `features/terminal/model/password-prompt.ts`：纯状态机，把 xterm 的 `onData` 折叠为口令缓冲。回车提交，Ctrl+C/Ctrl+D 放弃并清空缓冲，退格与 Ctrl+U 可编辑，ESC 引导的 CSI/SS3 整段跳过以免方向键污染口令（bracketed paste 的 `\x1b[200~`/`\x1b[201~` 也由此被剥离），缓冲上限 1024 字符防误粘贴大段文本。抽成纯函数是为了让按键语义可单测覆盖，无需驱动真实 xterm。
- 口令输入**完全不回显**，连长度都不暴露，与 `ssh`、PuTTY 的终端口令提示一致：星号掩码会向旁观者、录屏和截图泄露口令长度，这正是 OpenSSH 不回显、`sudo` 的 `pwfeedback` 默认关闭的原因。掩码方案（GUI 客户端密码框的习惯）曾短暂实现过，因终端形态下惯例优先而回退。代价是粘贴没有任何视觉确认，只能以回车后能否登录判断，因此提示语明确写出"输入不回显"，避免用户误以为按键没被收下。提示行本身采用 `\r\x1b[2K` 整行重绘，只在首次提示和被其他输出（如剪贴板错误）打断后重画。
- 粘贴走 `features/terminal/model/terminal-clipboard.ts` 的 `attachRightClickPaste`：应用壳在捕获阶段统一禁用了 WebView 原生右键菜单，终端里既没有菜单也就没有粘贴项，而右键即粘贴本来就是终端通用习惯（PuTTY/WindTerm/MobaXterm 默认如此）。刻意**不**接管 Ctrl+V / Ctrl+Shift+V / Shift+Insert：这些是 WebView 自带快捷键，xterm 已在 textarea 上监听 `paste` 并转成 `onData`，再自行读一次剪贴板会粘贴两遍（xterm 的自定义按键回调返回 `false` 只跳过自身处理，不会 `preventDefault`）。两条路径都汇入 `terminal.paste()`，因此换行归一化与 bracketed paste 包裹保持一致，口令提示符与已连会话共用同一套逻辑。
- `SshTerminal.tsx` 先按后端已有凭据尝试建连，只有后端明确返回稳定错误码 `SSH_PASSWORD_REQUIRED` 时才提示输入，再带着口令重试。判定权在后端而非前端 `credentialStatus`：连接列表里的凭据状态可能过期，而后端每次都会重新查询会话缓存与系统凭据库。
- `ssh_terminal_open` 增加 `password: Option<String>` 参数，并改为 `#[tauri::command(async)]`——它最长阻塞 30 秒等待建连，留在主线程会冻结窗口。
- 新增 `state/session_password.rs`：会话级口令缓存，`Mutex<HashMap<i64, Entry>>`，仅驻留内存，绝不写入 SQLite、系统凭据库、日志、测试快照或 IPC 事件。口令来源优先级为**本次输入 > 会话缓存 > 系统凭据库**，由纯函数 `choose_password_source` 决策以便单测。现场输入的口令只在认证通过后才写入缓存，避免把明显错误的口令留在内存里。
- 缓存生命周期采用**租约计数**而非"整个应用运行期"：`retain` 在认证通过时登记口令并占用一个租约，`release` 由终端输出线程在会话收尾时归还，归零即丢弃口令。语义上等价于 OpenSSH `ControlMaster` 的连接复用而不是"记住密码"——同一连接的第二个标签和文件页面不再追问，但该连接全部会话关闭后下次连接会重新提示。用户没有勾选保存的密码不应被静默留存，这是 `ssh`/PuTTY 的既有行为。`retain` 把"存口令"和"加租约"合并在同一把锁内，避免另一会话的收尾插在两步之间把条目清掉。归还挂钩点只有输出线程末尾一处：`close` 会中止会话任务并丢弃发送端，主动关标签、远端退出和读中断都会让读取返回 EOF 走到那里。删除连接与认证失败调用 `forget`，不论还有多少租约都立即清除。
- SFTP 复用同一缓存（`commands/sftp.rs` 的 `resolve_password`）：文件页面没有可输入的提示符，因此顺序为“会话缓存 > 系统凭据库”，两者皆空时返回 `SFTP_PASSWORD_REQUIRED`，前端提示用户先在终端输入一次并保持该标签打开，或保存密码。SFTP 侧**不**占用租约：会话建立后自持连接，终端关闭不影响已建立的 SFTP 会话，但此后新建 SFTP 会话会重新要求口令。
- 基础设施层 `build_auth` 的口令缺失分支降级为最后一道防线，提示语不再要求用户“先去保存密码”。

权衡与遗留：

- 命令层无法区分“认证失败”与“网络失败”，因此**任何**建连失败都会丢弃缓存口令，下次连接需重新输入。换取的是不引入编码错误的 `SshTerminalPort` 错误类型改造（会波及领域/用例/基础设施及其全部测试）。
- 缓存中的口令是普通 `String`，未使用 `zeroize`：Rust 的 `String` 在增长时会搬迁堆内存，仅靠 `Drop` 清零无法覆盖所有副本，收益有限而复杂度明显。安全边界仍是“绝不离开内存”。
- 右键粘贴依赖 `navigator.clipboard.readText()`。WebView2 可能按权限策略拒绝读取剪贴板，此时终端会提示改用 Ctrl+V，而不是静默失败。若实测确认该权限在 WebView2 下不可用，再引入 `tauri-plugin-clipboard-manager` 从 Rust 侧读取。
- 租约语义下，“关掉该连接最后一个终端标签后再去文件页面”会重新要求口令（终端里已没有活跃会话）。这是有意为之的取舍：把口令留到应用退出等于替用户做了“记住密码”的决定。若后续需要在文件页面独立登录，再补一个 SFTP 专用的密码对话框（Xshell/Termius 的做法），而不是延长缓存寿命。

### 阶段 4（2026-08-23）：私钥改为按文件路径引用

Windows 上"认证方式=密钥 + 选择 `.pem`"必然保存失败，红字为"保存系统凭据失败"。根因不在业务逻辑：私钥 PEM 内容被当作一条密钥写入系统凭据库，而 `keyring` 3.6.3 的 `windows-native` 后端在 `set_password` 里检查 `password.encode_utf16().count() * 2 > CRED_MAX_CREDENTIAL_BLOB_SIZE`（2560），即 ASCII 明文上限约 1280 字符。AWS 风格的 2048 位 RSA `.pem` 约 1.7 KB，任何 4096 位密钥更是远超；2560 字节是 Windows `CredWrite` 的硬上限，改用 `set_secret`（按原始字节判定）也只是把上限抬到 2560，仍装不下。也就是说"把私钥存进凭据管理器"这条路本身不可行，不是参数没调对。

主流客户端一致的做法是**引用文件路径**：OpenSSH 的 `IdentityFile`、PuTTY 的 Private key file、Xshell/MobaXterm/WindTerm 的密钥文件选择、VS Code Remote-SSH 全都只记路径，没有任何一个把密钥字节复制进 Keychain/凭据管理器。本项目照此改造。

- 领域层 `ConnectionProfile`/`NewConnectionProfile` 新增 `private_key_path: Option<String>`。它是路径这类**元数据**而非密钥内容，因此可以随连接资料落库；"资料不含明文"的边界没有被打破。SQLite 迁移到 `SCHEMA_VERSION = 4`，`ALTER TABLE ... ADD COLUMN private_key_path TEXT`，老库原地升级。
- 认证方式不是私钥时路径一律置空（`build_profile` 与命令层 `validate_private_key_path` 双重保证）：切到密码登录后残留旧路径会让"是否已绑定私钥"的判断长期失真。
- 用例层新增 `bind_private_key_path`，只更新资料；`store_credential` 与 `update_with_credential` 现在**显式拒绝** `kind == "private_key"`，且拒绝发生在触碰平台存储之前。命令层 `credential_store_file` 更名为 `connection_bind_private_key`——旧名字会让人以为密钥被复制进了凭据库。
- 密钥字节只在两处出现：绑定时读一次以确认文件此刻可读（随即丢弃），建连时由 `resolve_private_key` 读一次交给 russh。不落库、不进凭据库、不写临时文件、不回传前端。文件 I/O 留在 Tauri 命令层，用例层保持无 I/O。
- 兼容遗留数据：`resolve_private_key` 在资料没有路径时回落到系统凭据库里的旧 `private_key` 条目，因此改造前"侥幸存进去的短密钥"连接仍可直接使用，无需数据迁移；老条目可读可删，只是不再新增。前端 `isConnectionReady` 同理——有 `privateKeyPath` 即视为就绪，`credentialStatus === 'bound'` 作为回落。
- 平台写入失败不再被折叠：`CREDENTIAL_STORE_FAILED` 现在带上适配器回传的平台诊断文本。长度超限、权限不足和存储不可用的处置方式完全不同，原先统一显示"保存系统凭据失败"让用户和排查者都无从下手。诊断文本不含凭据本身，可安全跨 IPC。
- 备份导入不携带私钥路径：备份可能来自另一台机器，绝对路径在那里未必存在，恢复后由用户重新绑定，好过落一个指向空气的路径。SSH Config 导入相反——`IdentityFile` 本就是本机路径，`~/` 展开后调 `connection_bind_private_key` 补绑，无法解析的相对路径计入"需要手动绑定"的提示计数。

### 阶段 5（2026-08-23）：建连改为分阶段计时，失败提示指名道姓

原实现把"解析 → 拨号 → 握手 → 认证"整段交给 `russh::client::connect`，只在同步 `open()` 外层设一个 30 秒总超时。任何一步卡住都只报**"连接远程主机超时"**：既不说是哪台主机哪个端口，也不说卡在哪一步，用户无从判断该改端口、开安全组还是换凭据。实测一台安全组未放通本机出口 IP 的主机，界面就只有这一句红字。

- `connect_authenticated_coded` 拆成四步并各自计时：解析 15 秒预算内的拨号（`TCP_CONNECT_TIMEOUT`）、握手 20 秒（`HANDSHAKE_TIMEOUT`）、认证 30 秒（`AUTH_TIMEOUT`）。同步 `open()` 的兜底提到 75 秒（`CONNECT_READY_TIMEOUT`）并由单测断言它**大于**三阶段之和——外层若先超时就会用一句笼统的话盖掉阶段级提示，等于白做分阶段计时。
- 拨号自己做而不用 `client::connect`：后者内部的 `TcpStream::connect` 没有超时上限，Windows 默认要等约 21 秒 SYN 重试才报错；15 秒预算比它更早收口，才能确定"超时"这个结论出自我们而非操作系统的模糊 io 错误。拨通后的流交给 `client::connect_stream`，握手（含密钥交换与主机密钥校验，`connect_stream` 内部会等 kex 完成）因此能独立计时。
- 解析单独成步（`tokio::net::lookup_host`）：把"地址写错、DNS 不可用"与"能解析但连不上"分开。不能靠 `io::ErrorKind` 区分——Windows 上 getaddrinfo 的失败并不映射到 `ErrorKind::NotFound`，按 kind 分类会全部落进兜底分支。
- 多地址逐个尝试并共享同一预算：双栈主机常同时解析出 IPv6 与 IPv4，只有 IPv6 不通时若就此放弃，可用的 IPv4 地址就被浪费了。预算用尽时结论以超时为准，某个地址此前的快速失败（如 IPv6 立即被拒）不得掩盖真正的卡点。
- 所有提示都带上 `host:port` 与可执行的排查方向（对齐 OpenSSH 的 `ssh: connect to host X port 22: Connection timed out` 与 PuTTY 的 `Network error`）。**错误码集合保持不变**：`sftp-error.ts` 按码取固定文案，新增码只会退化成通用兜底文案，因此新提示只改 `message`。唯一调整是未归类网络错误的码由 `connectionRefused` 改为 `hostUnreachable`——"拒绝"是个明确结论，不该拿来兜底未知故障。
- 遗留：握手阶段超时时，`connect_stream` 内部已 `spawn` 的会话任务会随 future 被丢弃而失去 join 句柄，套接字由 `keepalive_interval`/`keepalive_max`（30 秒 × 3）兜底回收。这条路径与原先 `task.abort()` 的行为等价，不额外引入泄漏。

### 阶段 6（2026-08-23）：自查修复——读边界、会话隔离与"校验没跑成"的处置

对待提交改动做整体复核后修掉五类问题，都属于"实现细节偏离主流客户端行为"而非架构选择：

- **终端输出按字节直接 `from_utf8_lossy`**。PTY 与 SSH 通道的读边界由内核缓冲和网络分片决定，与字符边界无关：一个 3 字节汉字会被 8 KiB 缓冲切成 2 + 1 两次返回，每次各自 lossy 解码，被切断的半个字符**永久**变成 `�`。新增 `commands/terminal_text.rs` 的 `Utf8Stream` 跨读取保持解码状态（尾部不完整序列结转下一块，确定非法的字节才替换），SSH 与本地终端的读取线程共用。xterm.js 自带解码器与 OpenSSH 客户端都是这么做的。
- **重连计数是全局单值**。`reconnectNonce` 配合 `key={id}-{活动标签?nonce:0}` 的写法，在任意一次重连之后，每次切换标签都会同时改变新旧两个标签的 key，React 于是把两个终端**都**卸载重建——正在跑的会话被关闭，交互输入的口令因租约归还而重新追问。改为 `reconnectNonces: Record<string, number>` 按会话计数，切换标签不再触碰任何 key。
- **`terminalId` 未知期按 `connectionId` 兜底匹配事件**。同一连接开两个标签时，两边会互相写入对方的开场输出。改为**始终按 `terminalId` 严格过滤**：未知期先把同连接事件按各自 `terminalId` 缓存（上限 512 条），拿到自己的 id 后只回放属于自己的部分。顺带删掉后端从不发出的 `timed_out` 分支，以及 `ssh_terminal_close` 里那条 `connection_id: -1` 的重复退出事件（输出线程读到 EOF 后本就会发一条正确的）。
- **主机密钥校验 fail-open**。`check_server_key` 的兜底是 `Err(_) => Ok(true)`：known_hosts 中该主机的某行条目格式损坏会让整次校验返回 `Err`，于是"校验没跑成"被当作"校验通过"，TOFU 形同虚设。现在只有 `NoHomeDir`（无处可查也无处可记）保持宽松放行，其余错误一律拒绝并带上原文；`learn_known_hosts` 的写入失败也不再静默丢弃（改为告警，与 OpenSSH 写不进 known_hosts 时仅告警一致）——长期写不进去意味着每次连接都被当成首次，"变更即拒"永远不会生效。
- **DNS 解析没有超时**。`lookup_host` 走系统解析器，DNS 不可达时它自己的重试可能长达十几秒，叠加后续阶段就会顶穿外层兜底，让阶段 5 刚消灭的那句笼统提示重新出现。新增 `DNS_RESOLVE_TIMEOUT`（10 秒）并把它计入 `CONNECT_READY_TIMEOUT`（75 → 90 秒）的不变式断言。

同批次的次要调整：认证各分支保留底层原文（`Err` 是"请求没能完成"，与"口令不对"的 `Ok(false)` 是两件事，压成一句"认证失败"会让用户对着正确的口令反复重试）；`read_private_key_file` 的提示带上路径与系统原因；`request_pty` 改 `want_reply=true`，服务端拒绝分配 PTY 时能立刻报出来而不是给个残缺终端（OpenSSH 正是靠它报 "PTY allocation request failed"）；`close()` 给会话任务 3 秒宽限期再 `abort()`，否则 `run_session` 末尾的礼貌断开几乎来不及执行；SFTP 会话建立后 `set_timeout(60)` 覆盖 russh-sftp 的 10 秒默认值，传输块从 64 KiB 提到 128 KiB（下载在库内串行，吞吐上限是"块大小 ÷ RTT"）；`commit_remote` 在同名目标是目录时拒绝提交，避免备份逻辑把整个目录改名搬走；删除已无调用者的 `map_handler_error`。

### 阶段 7（2026-08-23）：跨平台细节——GUI 进程不该 spawn 控制台程序，远端名字是不可信输入

前六个阶段都在打磨 SSH/SFTP 本身，这一批修的是"两个平台上表现不一致或只在某一个平台上出错"的地方。

- **能力上报仍在探测系统 OpenSSH**。`platform/mod.rs` 的 `ssh_transport()` 会 spawn `ssh -V`（Windows 上是 `ssh.exe`）并把结果报成 `openssh-available` / `openssh-unavailable`。阶段 1 迁到进程内 russh 之后这个探测既无用又有害：没装 OpenSSH 的 Windows 会被报成"不可用"，让用户以为连不上 SSH；从 GUI 子系统进程 spawn 控制台程序还会在启动时闪一下黑窗，`output()` 又要同步等子进程退出。现在固定返回 `in-process-russh`，探测整体删除，macOS 测试里"系统必须装 ssh"的隐含契约随之消失。
- **本地 Shell 探测同样 spawn 了 `where.exe`**。两个候选（`pwsh.exe`、`powershell.exe`）各闪一次黑窗，还要等两次进程创建才能打开本地终端。改为自己走一遍 `PATH`：绝对路径直接落地检查，带分隔符的相对路径按当前目录解析（与 `CreateProcess` 的查找规则一致），单名走 `PATH` 逐目录拼接，没写扩展名时按 `PATHEXT` 补全（缺失则退回 Windows 出厂的 `.COM;.EXE;.BAT;.CMD`）。既没有窗口也没有等待。
- **远端条目名被直接拼进本地路径**。`download_tree` 的 `local_dir.join(&name)` 里 `name` 来自远端 `read_dir`，属于不可信输入：`..` 会让 `join` 向上跳级，Windows 上 `\` 与盘符前缀还会被当成路径语法，异常或恶意的服务端因此能让递归下载写到目标目录之外。新增 `ensure_safe_child_name` 在拼接前拦截，与 OpenSSH `sftp`、WinSCP 的客户端侧校验一致。
  - 判定交给 `std::path` 的组件解析而**不是**手写分隔符黑名单：合法条目名解析后必须恰好是一个 `Component::Normal`，`.`/`..` 会落到 `CurDir`/`ParentDir`，带分隔符或盘符的名字会解析出两个以上组件或 `Prefix`。这样 `a\b` 在 Windows 上按分隔符拒绝、在 macOS 上按普通文件名放行——它在 Unix 上确实是一个合法名字，一律拒绝就成了误伤。
  - 冒号单独按平台拦：Windows 上 `notes.txt:secret` 会静默写进 `notes.txt` 的备用数据流，资源管理器里看不到任何新文件；Unix 上冒号只是普通字符。
- **上传时本地文件名用 `to_string_lossy` 静默改名**。非 UTF-8 的本地文件名会被换成 `U+FFFD` 再发给远端，结果是"上传成功"但远端文件名已被悄悄改写。现在显式以 `localReadFailed` 拒绝并把 lossy 结果放进提示，让用户知道是哪个文件。macOS 的 APFS 强制 UTF-8，这条主要防 Unix 侧的任意字节文件名。

### 平台差异的组织方式：默认内联 `#[cfg]`，越线才拆文件

平台适配一律**内联 `#[cfg]`**，不为 Windows 与 macOS 各建一套并行文件树。理由是绝大多数差异属于"同一套逻辑、不同常量或不同入口"——`ssh/mod.rs` 里 SSH Agent 取身份（Unix 读 `SSH_AUTH_SOCK`、Windows 连 OpenSSH 命名管道）各两行，`sftp.rs` 里冒号是否算备用数据流只有一行。为一行差异拆文件会让读者为了看懂一个函数在两个文件间来回跳，`std` 自身也是这么处理小差异的。凭据存储与 PTY 的大块差异则由 `keyring`、`portable-pty` 在依赖层消化，因此 `credential/mod.rs` 与领域层、用例层、命令层、DTO 层的平台 `cfg` 数为 **0**。

越线的判据是**规模**而非风格：某一侧的平台专属代码超过百行、或占文件三成以上、或需要引入平台专属依赖（`windows-sys`、`core-foundation`）时才拆。目前只有本地 Shell 选择越线——Windows 侧自行走 `PATH`/`PATHEXT` 查找可执行文件，含测试约 110 行、占 `terminal/mod.rs` 的三分之一，且在 macOS 上一行都不编译。它已拆到 `terminal/shell.rs`（Windows 6 处 `cfg`、143 行），`terminal/mod.rs` 随之回落到 221 行且平台 `cfg` 归零，只剩 PTY 生命周期。

顺带说明文件行数与平台适配无关：三个最大的文件（`ssh/mod.rs` 1084 行、`sftp.rs` 935 行、`sqlite_connection_repository.rs` 848 行）的平台代码占比都在 2% 以内，长度来自业务本身——五阶段建连、四种认证、SFTP 传输语义与 SQL 迁移。

### 遗留

- 暂不支持带密码短语的加密私钥与证书型主机密钥。
- 暂不支持 ProxyJump/跳板机。
- 目录传输按条目类型分派，符号链接不跟随：远端指向目录的链接不会被递归镜像，本地指向目录的链接也不会被当作目录上传，会以 `localReadFailed` 上抛而非静默跳过。
- 目录上传的子文件直接写入目标名而非逐个临时提交，因此中途取消会在远端留下已传部分；文件级上传仍保证“临时名 + 原子提交”，取消不会覆盖已有目标。远端残留无法由本地清理，取消目录上传后需人工确认远端目录。
- Windows 的保留名（`CON`/`NUL`/`PRN`/`AUX` 等）与 `? * < > " |` 等非法字符不做转义：这些名字在 Linux 上合法、在 Windows 上无法创建，下载时由操作系统报错并以 `localWriteFailed` 带上系统原文上抛。WinSCP 会把它们转义成 `%3F` 之类，本项目暂不引入这套映射——转义后本地名与远端名不再一致，再上传回去会变成另一个文件。
- macOS 的 APFS 把文件名按 NFD（分解形式）存储，Linux 一般是 NFC。从 macOS 上传含重音符号的文件名到 Linux，远端拿到的是分解形式，与在 Linux 本地创建的同名文件字节序列不同。主流客户端多数也不做规范化转换（Cyberduck 提供选项、WinSCP 不处理），本项目同样不转换。
