//! SSH 终端会话实现：会话管理器、命令/取消注册表、会任务事件循环、
//! 独立 exec channel、异步到同步的通道桥接，以及领域 `SshTerminalPort` 适配。
//!
//! 连接、认证与主机密钥策略统一由 `super::connection` 提供；本模块只负责
//! 会话建立之后的生命周期管理与资源清理。

use std::{
    collections::HashMap,
    io::{self, Read},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use nocterm_domain::{
    connection::ConnectionProfile,
    terminal::{
        OpenedSshExecution, OpenedTerminal, SshExecutionCompletion, SshShellIntegrationPlan,
        SshShellKind, SshTerminalError, SshTerminalPort,
    },
};
use russh::{ChannelMsg, Disconnect, client, client::Handle};
use tokio::{
    runtime::Runtime,
    sync::{mpsc, oneshot},
    time::timeout,
};

use super::{
    SshError,
    connection::{Auth, ClientHandler, build_auth, connect_authenticated_coded},
};

/// 同步 `open()` 等待会话就绪的兜底上限，必须**大于**连接各阶段超时之和：
/// 外层若先超时，就会用一句笼统的"超时"盖掉阶段级的精确提示，等于白做分阶段计时。
pub(super) const CONNECT_READY_TIMEOUT: Duration = Duration::from_secs(90);
/// 主动关闭后等待会话任务自行收尾的宽限期，超过即中止任务。
/// 礼貌断开只需一个 RTT，3 秒足够；再长会让退出应用时的清理明显变慢。
const CLOSE_GRACE_TIMEOUT: Duration = Duration::from_secs(3);
/// 独立 exec channel 从排队到远端接受命令的上限；同步调用方按同一预算等待。
const EXEC_READY_TIMEOUT: Duration = Duration::from_secs(10);
const EXEC_CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// spawn 后等待远端对 exec 应答的超时；正常一次 RTT 内到达。
const EXEC_SPAWN_REPLY_TIMEOUT: Duration = Duration::from_secs(10);
/// 会话激活只处理降级路径的初始目录写入，正常路径不向 PTY 写入任何字节。
const ACTIVATE_TIMEOUT: Duration = Duration::from_secs(10);

/// 会话任务能接收的控制指令，均由同步命令层通过通道投递。
enum TerminalCommand {
    /// Host 已完成 nonce、输出抑制器和终端绑定注册，现在允许写入 bootstrap。
    Activate {
        completion_tx: std::sync::mpsc::Sender<Result<(), String>>,
    },
    /// 用户键入的字节流，原样写入远端 PTY 通道。
    Data(Vec<u8>),
    /// 终端尺寸调整，映射为 SSH 的 window-change 请求。
    Resize { cols: u16, rows: u16 },
    /// 在同一条已认证 SSH 连接上开启独立 exec channel。
    /// 输出不会进入用户当前可见的交互式 Shell 通道。
    Exec(ExecChannelRequest),
    /// 主动关闭：结束事件循环并礼貌断开连接。
    Close,
}

/// 一次独立 exec 的所有权载荷；聚合后可作为单元在会话任务与 channel 任务间转交。
struct ExecChannelRequest {
    execution_id: String,
    command: Vec<u8>,
    output_tx: std::sync::mpsc::Sender<Vec<u8>>,
    completion_tx: std::sync::mpsc::Sender<SshExecutionCompletion>,
    ready_tx: std::sync::mpsc::Sender<Result<(), String>>,
    cancel_rx: oneshot::Receiver<()>,
}

/// 单个终端在管理器侧的句柄：仅保留投递指令与中止任务所需的最小状态。
struct TerminalHandle {
    connection_id: i64,
    command_tx: mpsc::UnboundedSender<TerminalCommand>,
    task: tokio::task::JoinHandle<()>,
}

/// AI exec channel 的取消表。终端任务结束时会按 owner 一次性清理，避免关闭可见
/// PTY 后还有后台命令继续占用远端资源。
struct ExecutionHandle {
    owner_terminal_id: String,
    cancel: oneshot::Sender<()>,
}

#[derive(Default)]
struct ExecutionRegistry {
    handles: Mutex<HashMap<String, ExecutionHandle>>,
}

impl ExecutionRegistry {
    fn insert(
        &self,
        execution_id: String,
        owner_terminal_id: String,
        cancel: oneshot::Sender<()>,
    ) -> Result<(), SshError> {
        let mut handles = self
            .handles
            .lock()
            .map_err(|_| SshError::new("SSH exec 取消状态锁已损坏"))?;
        if handles.contains_key(&execution_id) {
            return Err(SshError::new("SSH exec 执行标识冲突"));
        }
        handles.insert(
            execution_id,
            ExecutionHandle {
                owner_terminal_id,
                cancel,
            },
        );
        Ok(())
    }

    fn remove(&self, execution_id: &str) {
        if let Ok(mut handles) = self.handles.lock() {
            handles.remove(execution_id);
        }
    }

    fn cancel(&self, execution_id: &str) -> bool {
        let handle = self
            .handles
            .lock()
            .ok()
            .and_then(|mut handles| handles.remove(execution_id));
        handle.is_some_and(|handle| handle.cancel.send(()).is_ok())
    }

    fn cancel_owned(&self, owner_terminal_id: &str) {
        let handles = self
            .handles
            .lock()
            .ok()
            .map(|mut handles| {
                let ids = handles
                    .iter()
                    .filter(|(_, handle)| handle.owner_terminal_id == owner_terminal_id)
                    .map(|(id, _)| id.clone())
                    .collect::<Vec<_>>();
                ids.into_iter()
                    .filter_map(|id| handles.remove(&id))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for handle in handles {
            let _ = handle.cancel.send(());
        }
    }

    fn cancel_all(&self) {
        let handles = self
            .handles
            .lock()
            .ok()
            .map(|mut handles| {
                handles
                    .drain()
                    .map(|(_, handle)| handle)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for handle in handles {
            let _ = handle.cancel.send(());
        }
    }
}

/// 一次 SSH 会话建立的 integration 结果：成功计划或降级原因（可同时为空表示
/// 普通 shell 且无降级语义——保留以区分"未尝试"）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshIntegrationOutcome {
    pub shell_integration: Option<SshShellIntegrationPlan>,
    pub fallback_reason: Option<&'static str>,
}

/// Bootstrap 的内部结果同时保留 Shell 识别事实和 spawn 预载计划。
///
/// 二者不能合并：例如 PowerShell 已识别但预载文件上传失败时，AI 必须降级独立
/// exec，普通终端的初始目录仍应使用 `Set-Location` 而不是 POSIX `cd`。
struct BootstrapOutcome {
    detected_shell: Option<SshShellKind>,
    /// spawn 预载就绪：启动命令 + 已上传文件。`None` 表示按普通登录 shell 走。
    spawn: Option<super::bootstrap::SpawnPlan>,
    fallback_reason: Option<&'static str>,
}

impl BootstrapOutcome {
    fn unavailable(reason: &'static str) -> Self {
        Self {
            detected_shell: None,
            spawn: None,
            fallback_reason: Some(reason),
        }
    }

    fn detected_but_unavailable(shell: SshShellKind, reason: &'static str) -> Self {
        Self {
            detected_shell: Some(shell),
            spawn: None,
            fallback_reason: Some(reason),
        }
    }

    fn ready(spawn: super::bootstrap::SpawnPlan) -> Self {
        Self {
            detected_shell: Some(spawn.plan.shell),
            spawn: Some(spawn),
            fallback_reason: None,
        }
    }
}

/// 同步 open() 的完整产物：终端标识、输出流与 integration 结果。
pub struct SshOpenedSession {
    pub terminal_id: String,
    pub reader: Box<dyn std::io::Read + Send>,
    pub integration: SshIntegrationOutcome,
}

/// 会话管理器统一持有运行时与所有终端句柄，避免命令层散落异步资源。
pub struct SshTerminalManager {
    runtime: Runtime,
    terminals: Mutex<HashMap<String, TerminalHandle>>,
    next_id: AtomicU64,
    next_execution_id: AtomicU64,
    executions: Arc<ExecutionRegistry>,
}

impl Default for SshTerminalManager {
    fn default() -> Self {
        // 多线程运行时保证读写与保活在会话任务阻塞时仍可推进。
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("构建 SSH 运行时失败");
        Self {
            runtime,
            terminals: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(0),
            next_execution_id: AtomicU64::new(0),
            executions: Arc::new(ExecutionRegistry::default()),
        }
    }
}

impl SshTerminalManager {
    /// 打开一个 SSH 终端：在运行时上派生会话任务，同步阻塞直至认证并开壳成功。
    ///
    /// 返回 integration 计划（bootstrap 成功）与降级原因（回退普通 shell）。
    /// 两者都属于控制面事实，命令层据此注册 nonce 与 capability；open 本身
    /// 不因 bootstrap 失败而失败。
    pub fn open(
        &self,
        profile: &ConnectionProfile,
        cols: u16,
        rows: u16,
        password: Option<&str>,
        private_key: Option<&str>,
    ) -> Result<SshOpenedSession, SshError> {
        // 依据认证方式装配所需凭据，缺失时提前失败以给出明确提示。
        let auth = build_auth(profile, password, private_key)?;

        let terminal_id = format!("ssh-{}", self.next_id.fetch_add(1, Ordering::Relaxed) + 1);
        let host = profile.host.clone();
        let port = profile.port;
        let username = profile.username.clone();
        let initial_path = normalize_initial_path(profile.remote_initial_path.as_deref())?;

        // 输出走同步通道桥接为 Read；指令走异步通道；就绪结果走同步通道以便阻塞等待。
        let (output_tx, output_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let (command_tx, command_rx) = mpsc::unbounded_channel::<TerminalCommand>();
        let (ready_tx, ready_rx) =
            std::sync::mpsc::channel::<Result<SshIntegrationOutcome, SshError>>();

        let task = self.runtime.spawn(run_session(
            host,
            port,
            username,
            auth,
            cols,
            rows,
            initial_path,
            output_tx,
            command_rx,
            ready_tx,
            terminal_id.clone(),
            Arc::clone(&self.executions),
        ));

        // 各建连阶段自带精确超时，这里只做兜底：会话任务被意外中止或运行时僵死时，
        // 同步调用方不至于永久阻塞。正常路径下永远不会走到这一支。
        let integration = match ready_rx.recv_timeout(CONNECT_READY_TIMEOUT) {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(error)) => {
                task.abort();
                return Err(error);
            }
            Err(_) => {
                task.abort();
                return Err(SshError::coded(
                    "timeout",
                    format!(
                        "连接 {}:{} 无响应：等待 {} 秒后仍未完成建连，请重试",
                        profile.host,
                        profile.port,
                        CONNECT_READY_TIMEOUT.as_secs()
                    ),
                ));
            }
        };

        let mut terminals = self
            .terminals
            .lock()
            .map_err(|_| SshError::new("SSH 终端状态锁已损坏"))?;
        terminals.insert(
            terminal_id.clone(),
            TerminalHandle {
                connection_id: profile.id,
                command_tx,
                task,
            },
        );
        Ok(SshOpenedSession {
            terminal_id,
            reader: Box::new(ChannelReader {
                rx: output_rx,
                buffer: Vec::new(),
                position: 0,
            }),
            integration,
        })
    }

    /// 在已有终端所属的认证连接上建立独立 exec channel。
    ///
    /// 终端没有活跃会话时返回 `Ok(None)`；一旦找到活跃会话，私钥/密码不会再次
    /// 从凭据层读取，也不会向可见 PTY 写入命令。
    pub fn exec_existing(
        &self,
        connection_id: i64,
        command: &str,
        cancellation: &AtomicBool,
    ) -> Result<Option<OpenedSshExecution>, SshError> {
        if cancellation.load(Ordering::Acquire) {
            return Err(SshError::new("AI 任务已停止，远程命令未执行"));
        }
        let (owner_terminal_id, command_tx) = {
            let mut terminals = self
                .terminals
                .lock()
                .map_err(|_| SshError::new("SSH 终端状态锁已损坏"))?;
            // 远端 EOF 与 UI 清理之间存在短窗口，先剔除已结束或已关闭的句柄。
            terminals.retain(|_, terminal| {
                !terminal.task.is_finished() && !terminal.command_tx.is_closed()
            });
            let Some((terminal_id, terminal)) = terminals
                .iter()
                .filter(|(_, terminal)| terminal.connection_id == connection_id)
                .min_by(|(left, _), (right, _)| left.cmp(right))
            else {
                return Ok(None);
            };
            (terminal_id.clone(), terminal.command_tx.clone())
        };
        let execution_id = format!(
            "ssh-exec-{}",
            self.next_execution_id.fetch_add(1, Ordering::Relaxed) + 1
        );
        let (output_tx, output_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let (completion_tx, completion) = std::sync::mpsc::channel();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let (cancel_tx, cancel_rx) = oneshot::channel();
        self.executions
            .insert(execution_id.clone(), owner_terminal_id, cancel_tx)?;
        if command_tx
            .send(TerminalCommand::Exec(ExecChannelRequest {
                execution_id: execution_id.clone(),
                command: command.as_bytes().to_vec(),
                output_tx,
                completion_tx,
                ready_tx,
                cancel_rx,
            }))
            .is_err()
        {
            self.executions.cancel(&execution_id);
            return Err(SshError::new("当前 SSH 连接已停止接收 AI 命令"));
        }
        let deadline = std::time::Instant::now() + EXEC_READY_TIMEOUT;
        loop {
            if cancellation.load(Ordering::Acquire) {
                self.executions.cancel(&execution_id);
                return Err(SshError::new("AI 任务已停止，远程命令未执行"));
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                self.executions.cancel(&execution_id);
                return Err(SshError::new("在当前 SSH 连接上打开 AI 执行通道超时"));
            }
            match ready_rx.recv_timeout(remaining.min(EXEC_CANCEL_POLL_INTERVAL)) {
                Ok(Ok(())) => {
                    if cancellation.load(Ordering::Acquire) {
                        self.executions.cancel(&execution_id);
                        return Err(SshError::new("AI 任务已停止，远程命令未执行"));
                    }
                    return Ok(Some(OpenedSshExecution {
                        id: execution_id,
                        reader: Box::new(ChannelReader {
                            rx: output_rx,
                            buffer: Vec::new(),
                            position: 0,
                        }),
                        completion,
                    }));
                }
                Ok(Err(message)) => return Err(SshError::new(message)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    self.executions.cancel(&execution_id);
                    return Err(SshError::new("SSH exec 通道在启动完成前意外关闭"));
                }
            }
        }
    }

    pub fn write(&self, terminal_id: &str, data: &str) -> Result<(), SshError> {
        let terminals = self
            .terminals
            .lock()
            .map_err(|_| SshError::new("SSH 终端状态锁已损坏"))?;
        let terminal = terminals
            .get(terminal_id)
            .ok_or_else(|| SshError::new("SSH 终端不存在或已关闭"))?;
        terminal
            .command_tx
            .send(TerminalCommand::Data(data.as_bytes().to_vec()))
            .map_err(|_| SshError::new("SSH 终端已停止接收输入"))
    }

    /// 在调用方完成输出控制面注册后激活会话，建立严格的 arm-before-write 顺序。
    pub fn activate(&self, terminal_id: &str) -> Result<(), SshError> {
        let command_tx = {
            let terminals = self
                .terminals
                .lock()
                .map_err(|_| SshError::new("SSH 终端状态锁已损坏"))?;
            terminals
                .get(terminal_id)
                .ok_or_else(|| SshError::new("SSH 终端不存在或已关闭"))?
                .command_tx
                .clone()
        };
        let (completion_tx, completion_rx) = std::sync::mpsc::channel();
        command_tx
            .send(TerminalCommand::Activate { completion_tx })
            .map_err(|_| SshError::new("SSH 终端已停止接收激活请求"))?;
        match completion_rx.recv_timeout(ACTIVATE_TIMEOUT) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(message)) => Err(SshError::new(message)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                Err(SshError::new("SSH 终端激活超时"))
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                Err(SshError::new("SSH 终端在激活完成前意外关闭"))
            }
        }
    }

    pub fn resize(&self, terminal_id: &str, cols: u16, rows: u16) -> Result<(), SshError> {
        let terminals = self
            .terminals
            .lock()
            .map_err(|_| SshError::new("SSH 终端状态锁已损坏"))?;
        let terminal = terminals
            .get(terminal_id)
            .ok_or_else(|| SshError::new("SSH 终端不存在或已关闭"))?;
        terminal
            .command_tx
            .send(TerminalCommand::Resize { cols, rows })
            .map_err(|_| SshError::new("SSH 终端已停止响应"))
    }

    pub fn close(&self, terminal_id: &str) -> Result<(), SshError> {
        let handle = self
            .terminals
            .lock()
            .map_err(|_| SshError::new("SSH 终端状态锁已损坏"))?
            .remove(terminal_id);
        if let Some(handle) = handle {
            // 先请求礼貌关闭，给会话任务一点时间发出 channel close 与 SSH disconnect：
            // 原实现紧接着就 `abort()`，任务几乎来不及被唤醒，`run_session` 末尾的
            // 礼貌收尾等于形同虚设，远端会看到连接被硬断（sshd 记为异常断开）。
            // 宽限期结束仍未退出才中止，保证不会有任务永久滞留。
            let _ = handle.command_tx.send(TerminalCommand::Close);
            let mut task = handle.task;
            self.runtime.spawn(async move {
                if timeout(CLOSE_GRACE_TIMEOUT, &mut task).await.is_err() {
                    task.abort();
                }
            });
        } else {
            // AI exec channel 使用同一个 close 入口，超时或取消时不会误关用户可见 PTY。
            self.executions.cancel(terminal_id);
        }
        Ok(())
    }

    pub fn close_connection(&self, connection_id: i64) -> Result<(), SshError> {
        let terminal_ids = self
            .terminals
            .lock()
            .map_err(|_| SshError::new("SSH 终端状态锁已损坏"))?
            .iter()
            .filter(|(_, terminal)| terminal.connection_id == connection_id)
            .map(|(terminal_id, _)| terminal_id.clone())
            .collect::<Vec<_>>();

        let mut first_error = None;
        for terminal_id in terminal_ids {
            if let Err(error) = self.close(&terminal_id) {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

impl Drop for SshTerminalManager {
    fn drop(&mut self) {
        // 进程退出或管理器销毁时中止全部会话任务，避免悬挂的网络连接。
        self.executions.cancel_all();
        if let Ok(mut terminals) = self.terminals.lock() {
            for (_, handle) in terminals.drain() {
                handle.task.abort();
            }
        }
    }
}

impl SshTerminalPort for SshTerminalManager {
    fn open(
        &self,
        profile: &ConnectionProfile,
        cols: u16,
        rows: u16,
        password: Option<&str>,
        private_key: Option<&str>,
    ) -> Result<OpenedTerminal, SshTerminalError> {
        let opened = SshTerminalManager::open(self, profile, cols, rows, password, private_key)
            .map_err(|error| SshTerminalError::new(error.code, error.message))?;
        Ok(OpenedTerminal {
            id: opened.terminal_id,
            reader: opened.reader,
            shell_integration: opened.integration.shell_integration,
            integration_fallback_reason: opened.integration.fallback_reason,
        })
    }

    fn write(&self, terminal_id: &str, data: &str) -> Result<(), String> {
        SshTerminalManager::write(self, terminal_id, data).map_err(|error| error.to_string())
    }

    fn activate(&self, terminal_id: &str) -> Result<(), String> {
        SshTerminalManager::activate(self, terminal_id).map_err(|error| error.to_string())
    }

    fn resize(&self, terminal_id: &str, cols: u16, rows: u16) -> Result<(), String> {
        SshTerminalManager::resize(self, terminal_id, cols, rows).map_err(|error| error.to_string())
    }

    fn close(&self, terminal_id: &str) -> Result<(), String> {
        SshTerminalManager::close(self, terminal_id).map_err(|error| error.to_string())
    }

    fn close_connection(&self, connection_id: i64) -> Result<(), String> {
        SshTerminalManager::close_connection(self, connection_id).map_err(|error| error.to_string())
    }

    fn exec_existing(
        &self,
        connection_id: i64,
        command: &str,
        cancellation: &AtomicBool,
    ) -> Result<Option<OpenedSshExecution>, String> {
        SshTerminalManager::exec_existing(self, connection_id, command, cancellation)
            .map_err(|error| error.to_string())
    }
}

/// 将 russh 通道的异步字节流桥接为同步 `Read`：读取线程阻塞在 recv 上。
struct ChannelReader {
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    buffer: Vec<u8>,
    position: usize,
}

impl Read for ChannelReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        // 缓冲耗尽时阻塞等待下一块；发送端关闭即视为 EOF。
        if self.position >= self.buffer.len() {
            match self.rx.recv() {
                Ok(chunk) => {
                    self.buffer = chunk;
                    self.position = 0;
                    if self.buffer.is_empty() {
                        return Ok(0);
                    }
                }
                Err(_) => return Ok(0),
            }
        }
        let available = self.buffer.len() - self.position;
        let count = std::cmp::min(out.len(), available);
        out[..count].copy_from_slice(&self.buffer[self.position..self.position + count]);
        self.position += count;
        Ok(count)
    }
}

/// 校验远程初始路径：换行符会破坏后续 shell 指令，必须拒绝。
fn normalize_initial_path(raw: Option<&str>) -> Result<Option<String>, SshError> {
    let path = raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if path
        .as_deref()
        .is_some_and(|value| value.contains('\n') || value.contains('\r'))
    {
        return Err(SshError::new("远程初始路径不能包含换行符"));
    }
    Ok(path)
}

/// 会话任务主体：建连认证成功后进入读写事件循环，直至关闭或断开。
#[allow(clippy::too_many_arguments)]
async fn run_session(
    host: String,
    port: u16,
    username: String,
    auth: Auth,
    cols: u16,
    rows: u16,
    initial_path: Option<String>,
    output_tx: std::sync::mpsc::Sender<Vec<u8>>,
    mut command_rx: mpsc::UnboundedReceiver<TerminalCommand>,
    ready_tx: std::sync::mpsc::Sender<Result<SshIntegrationOutcome, SshError>>,
    terminal_id: String,
    executions: Arc<ExecutionRegistry>,
) {
    // 建连与认证阶段的结果先回传给同步 open()，再进入长期事件循环。
    // integration 结果随就绪信号一起回传；bootstrap 失败不阻塞建连。
    let session = match connect_session(
        &host,
        port,
        &username,
        auth,
        cols,
        rows,
        initial_path.as_deref(),
        &output_tx,
    )
    .await
    {
        Ok(setup) => {
            // 计划同时供两处消费：outcome 给命令层注册 nonce，activate 判断
            // 是否还需要降级写入——必须 clone 而非 take。
            let outcome = SshIntegrationOutcome {
                shell_integration: setup.shell_integration.clone(),
                fallback_reason: setup.integration_fallback_reason,
            };
            let _ = ready_tx.send(Ok(outcome));
            setup
        }
        Err(error) => {
            let _ = ready_tx.send(Err(error));
            return;
        }
    };
    let SessionSetup {
        handle,
        read_half: mut session_read_half,
        write_half,
        shell_integration,
        detected_shell,
        spawn_files,
        ..
    } = session;
    // russh 的 Handle 本身不可 Clone；用异步互斥封装后，AI exec channel 可以在
    // 保持可见 Shell 读写循环运行的同时复用同一条已认证 SSH 连接。
    let handle = Arc::new(tokio::sync::Mutex::new(handle));
    let mut execution_tasks: Vec<tokio::task::JoinHandle<()>> = Vec::new();

    let mut activation_started = false;

    loop {
        // 已完成的 AI 通道只留下极短的 JoinHandle；持续对话不会因为历史执行次数
        // 无限增长内存占用。
        execution_tasks.retain(|task| !task.is_finished());
        tokio::select! {
            message = session_read_half.wait() => match message {
                // 标准输出与扩展输出（stderr）都回传给前端渲染。
                Some(ChannelMsg::Data { data }) => {
                    if output_tx.send(data.to_vec()).is_err() {
                        break;
                    }
                }
                Some(ChannelMsg::ExtendedData { data, .. }) => {
                    if output_tx.send(data.to_vec()).is_err() {
                        break;
                    }
                }
                // 远端结束或通道关闭：退出循环，reader 将读到 EOF。
                Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => break,
                Some(_) => {}
            },
            command = command_rx.recv() => match command {
                Some(TerminalCommand::Activate { completion_tx }) => {
                    if activation_started {
                        let _ = completion_tx.send(Err("SSH 终端已经激活".to_string()));
                        continue;
                    }
                    activation_started = true;
                    // spawn 预载路径下 activate 不写入任何字节；只有降级路径
                    // （无 integration 且配置了初始目录）才向可见 PTY 敲入 cd。
                    let result = activate_session(
                        &write_half,
                        shell_integration.as_ref(),
                        initial_path.as_deref(),
                        detected_shell,
                        &terminal_id,
                    )
                    .await;
                    let _ = completion_tx.send(result);
                }
                Some(TerminalCommand::Data(bytes)) => {
                    let _ = write_half.data_bytes(bytes).await;
                }
                Some(TerminalCommand::Resize { cols, rows }) => {
                    let _ = write_half
                        .window_change(u32::from(cols), u32::from(rows), 0, 0)
                        .await;
                }
                Some(TerminalCommand::Exec(request)) => {
                    let task = tokio::spawn(run_exec_channel(
                        Arc::clone(&handle),
                        request,
                        Arc::clone(&executions),
                    ));
                    execution_tasks.push(task);
                }
                // 显式关闭或发送端全部释放：结束循环。
                Some(TerminalCommand::Close) | None => break,
            },
        }
    }

    // 先通知所有独立 exec channel，再中止其任务；否则关闭可见 Shell 后，
    // AI 任务可能继续占用同一 SSH 连接直到远端命令自然结束。
    executions.cancel_owned(&terminal_id);
    for task in execution_tasks {
        task.abort();
    }
    // spawn 预载的临时文件正常由 rc 尾部自删除；shell 未读到 rc 就退出或远端
    // 异常时在此兜底。rm 对已不存在的路径是幂等 no-op，正常关闭的代价可忽略。
    if !spawn_files.is_empty() {
        let handle = handle.lock().await;
        super::bootstrap::cleanup_files(&handle, &spawn_files).await;
    }
    // 尽力礼貌关闭通道并断开连接，忽略此时已不可达的错误。
    let _ = write_half.close().await;
    let handle = handle.lock().await;
    let _ = handle.disconnect(Disconnect::ByApplication, "", "").await;
}

/// spawn 预载路径下集成已随 shell 启动加载，activate 不写入任何字节；
/// 仅降级路径（无 integration 且配置了初始目录）需要向可见 PTY 敲入一次 cd。
/// 这一条 `cd` 是架构不变量（PTY 输入只允许用户键入与已授权可见执行）下
/// 唯一保留的机制性写入，内容无害且仅出现在不支持的 shell 上。
async fn activate_session(
    write_half: &russh::ChannelWriteHalf<client::Msg>,
    shell_integration: Option<&SshShellIntegrationPlan>,
    initial_path: Option<&str>,
    detected_shell: Option<SshShellKind>,
    terminal_id: &str,
) -> Result<(), String> {
    if shell_integration.is_some() {
        eprintln!(
            "[nocterm-ssh-bootstrap] spawn 预载已生效，激活无需写入（terminal={terminal_id}）"
        );
        return Ok(());
    }
    if let Some(path) = initial_path {
        write_half
            .data_bytes(
                super::bootstrap::initial_directory_command(path, detected_shell).into_bytes(),
            )
            .await
            .map_err(|error| format!("切换 SSH 初始目录失败：{error}"))?;
    }
    Ok(())
}

/// 在已认证 Handle 上执行一次远程命令。该 channel 没有 PTY，输出只回传给
/// AI 调用方，天然与用户正在看的交互式 Shell 隔离。
async fn run_exec_channel(
    handle: Arc<tokio::sync::Mutex<Handle<ClientHandler>>>,
    request: ExecChannelRequest,
    executions: Arc<ExecutionRegistry>,
) {
    let ExecChannelRequest {
        execution_id,
        command,
        output_tx,
        completion_tx,
        ready_tx,
        mut cancel_rx,
    } = request;
    let channel = tokio::select! {
        _ = &mut cancel_rx => {
            executions.remove(&execution_id);
            return;
        }
        result = async {
            let handle = handle.lock().await;
            handle.channel_open_session().await
        } => {
            match result {
            Ok(channel) => channel,
            Err(error) => {
                executions.remove(&execution_id);
                let _ = ready_tx.send(Err(format!("打开 SSH exec 通道失败：{error}")));
                return;
            }
        }
        }
    };
    let exec_result = tokio::select! {
        _ = &mut cancel_rx => {
            executions.remove(&execution_id);
            let _ = channel.close().await;
            return;
        }
        result = channel.exec(true, command) => result,
    };
    if let Err(error) = exec_result {
        executions.remove(&execution_id);
        let _ = ready_tx.send(Err(format!("启动远程命令失败：{error}")));
        return;
    }
    let (mut read_half, write_half) = channel.split();
    if ready_tx.send(Ok(())).is_err() {
        executions.remove(&execution_id);
        let _ = write_half.close().await;
        return;
    }
    let mut exit_code = None;
    loop {
        tokio::select! {
            message = read_half.wait() => match message {
                Some(russh::ChannelMsg::Data { data })
                | Some(russh::ChannelMsg::ExtendedData { data, .. }) => {
                    if output_tx.send(data.to_vec()).is_err() {
                        break;
                    }
                }
                // EOF 只表示远端不再发送数据，exit-status 仍可能随后到达；等 Close 再收尾。
                Some(russh::ChannelMsg::Eof) => {}
                Some(russh::ChannelMsg::Close) | None => break,
                Some(russh::ChannelMsg::ExitStatus { exit_status }) => {
                    exit_code = Some(exit_status);
                }
                Some(_) => {}
            },
            _ = &mut cancel_rx => break,
        }
    }
    let _ = completion_tx.send(SshExecutionCompletion { exit_code });
    executions.remove(&execution_id);
    let _ = write_half.close().await;
}

/// 建连产物：会话三元组 + integration 计划/降级原因 + 预载临时文件清单。
struct SessionSetup {
    handle: Handle<ClientHandler>,
    read_half: russh::ChannelReadHalf,
    write_half: russh::ChannelWriteHalf<client::Msg>,
    /// spawn 预载成功时的注入计划；命令层据此注册 nonce 控制面。
    shell_integration: Option<SshShellIntegrationPlan>,
    /// 独立于 integration 成功与否的 Shell 识别结果，用于普通终端命令语义。
    detected_shell: Option<SshShellKind>,
    integration_fallback_reason: Option<&'static str>,
    /// spawn 预载上传的远端临时文件；会话关闭时兜底删除（正常路径 rc 已自删）。
    spawn_files: Vec<String>,
}

/// 建立 TCP 连接、完成认证，并尝试 spawn 预载 shell integration。
///
/// 流程：建连认证（分阶段分类不变）→ 探针 + 预载文件上传（全部走独立 exec
/// channel，不触碰交互通道）→ **pty + exec 启动带预载的交互 shell**。exec 被
/// 远端拒绝（ForceCommand、受限 shell 等）时降级普通 `request_shell`，集成
/// 不可用但登录会话照常。任何 bootstrap 失败都不让 `open()` 失败。
#[allow(clippy::too_many_arguments)]
async fn connect_session(
    host: &str,
    port: u16,
    username: &str,
    auth: Auth,
    cols: u16,
    rows: u16,
    initial_path: Option<&str>,
    output_tx: &std::sync::mpsc::Sender<Vec<u8>>,
) -> Result<SessionSetup, SshError> {
    let handle = connect_authenticated_coded(host, port, username, auth)
        .await
        .map_err(|failure| SshError::coded(failure.code, failure.message))?;

    // bootstrap 先行：spawn 预载要求临时文件先于 shell 存在。
    let bootstrap = run_bootstrap(&handle, initial_path).await;
    if let Some(spawn) = bootstrap.spawn {
        match spawn_integrated_shell(&handle, cols, rows, &spawn, output_tx).await {
            Ok((read_half, write_half)) => {
                eprintln!(
                    "[nocterm-ssh-bootstrap] spawn 预载完成：shell={:?} nonce={}（集成随 shell 启动加载，零 PTY 注入）",
                    spawn.plan.shell.as_str(),
                    spawn.plan.nonce
                );
                let spawn_files = spawn.file_paths();
                return Ok(SessionSetup {
                    handle,
                    read_half,
                    write_half,
                    shell_integration: Some(spawn.plan),
                    detected_shell: bootstrap.detected_shell,
                    integration_fallback_reason: None,
                    spawn_files,
                });
            }
            Err(error) => {
                eprintln!(
                    "[nocterm-ssh-bootstrap] spawn 失败（{}），降级普通登录 shell",
                    error.message
                );
                super::bootstrap::cleanup_files(&handle, &spawn.file_paths()).await;
                return plain_login_shell(
                    handle,
                    cols,
                    rows,
                    bootstrap.detected_shell,
                    Some("spawn_unavailable"),
                )
                .await;
            }
        }
    }
    let reason = bootstrap.fallback_reason.unwrap_or("bootstrap_unavailable");
    eprintln!("[nocterm-ssh-bootstrap] bootstrap 降级（{reason}）：AI 命令将走独立 exec 通道");
    plain_login_shell(handle, cols, rows, bootstrap.detected_shell, Some(reason)).await
}

/// 普通登录 shell 通道：request_pty + request_shell，登录语义由 sshd 原样保留。
/// 支持 shell 的 spawn 失败与不支持的 shell 都落到这里，集成均不可用。
async fn plain_login_shell(
    handle: Handle<ClientHandler>,
    cols: u16,
    rows: u16,
    detected_shell: Option<SshShellKind>,
    fallback_reason: Option<&'static str>,
) -> Result<SessionSetup, SshError> {
    let channel = open_pty_channel(&handle, cols, rows).await?;
    channel
        .request_shell(true)
        .await
        .map_err(|error| SshError::new(format!("启动远程 Shell 失败：{error}")))?;
    let (read_half, write_half) = channel.split();
    Ok(SessionSetup {
        handle,
        read_half,
        write_half,
        shell_integration: None,
        detected_shell,
        integration_fallback_reason: fallback_reason,
        spawn_files: Vec::new(),
    })
}

/// 在 pty 通道上以 exec 启动带预载的交互 shell，并等待远端应答。
///
/// russh 的 `request_pty`/`exec` 都只发送请求；Success/Failure 以消息形式按序
/// 到达读流：先是 pty-req 应答，然后是 exec 应答。协议保证应答先于任何通道
/// 数据，但为容错不合规实现，先到达的数据会被缓存并在成功后回灌输出流，
/// 绝不静默丢弃。
async fn spawn_integrated_shell(
    handle: &Handle<ClientHandler>,
    cols: u16,
    rows: u16,
    spawn: &super::bootstrap::SpawnPlan,
    output_tx: &std::sync::mpsc::Sender<Vec<u8>>,
) -> Result<(russh::ChannelReadHalf, russh::ChannelWriteHalf<client::Msg>), SshError> {
    let channel = open_pty_channel(handle, cols, rows).await?;
    let (mut read_half, write_half) = channel.split();
    // 消费 pty-req 的应答，避免与 exec 应答混淆（服务端按序回复：pty → exec）。
    expect_channel_reply(&mut read_half, "PTY 分配").await?;
    // want_reply=true：远端拒绝启动命令时以 Failure 消息可见，调用方据此降级。
    write_half
        .exec(true, spawn.command.as_bytes().to_vec())
        .await
        .map_err(|error| SshError::new(format!("发送集成 shell 启动命令失败：{error}")))?;
    let mut buffered: Vec<u8> = Vec::new();
    let deadline = std::time::Instant::now() + EXEC_SPAWN_REPLY_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(SshError::new("远端对集成 shell 启动请求无应答"));
        }
        match tokio::time::timeout(remaining, read_half.wait()).await {
            Err(_) => {
                return Err(SshError::new("等待集成 shell 启动应答超时"));
            }
            Ok(None) => {
                return Err(SshError::new("集成 shell 启动通道被远端关闭"));
            }
            Ok(Some(russh::ChannelMsg::Success)) => {
                if !buffered.is_empty()
                    && let Err(error) = output_tx.send(std::mem::take(&mut buffered))
                {
                    return Err(SshError::new(format!("输出通道已关闭：{error}")));
                }
                return Ok((read_half, write_half));
            }
            Ok(Some(russh::ChannelMsg::Failure)) => {
                return Err(SshError::new("远端拒绝集成 shell 启动命令"));
            }
            Ok(Some(russh::ChannelMsg::Data { data }))
            | Ok(Some(russh::ChannelMsg::ExtendedData { data, .. })) => {
                buffered.extend_from_slice(&data);
            }
            Ok(Some(_)) => {}
        }
    }
}

/// 等待通道上下一个请求应答消息：Success 放行，Failure/关闭/超时报错。
/// 仅用于 spawn 前消费 pty-req 应答；exec 应答由 spawn_integrated_shell 的
/// 主循环处理（需要透传先行到达的数据）。
async fn expect_channel_reply(
    read_half: &mut russh::ChannelReadHalf,
    label: &str,
) -> Result<(), SshError> {
    let deadline = std::time::Instant::now() + EXEC_SPAWN_REPLY_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(SshError::new(format!("等待{label}应答超时")));
        }
        match tokio::time::timeout(remaining, read_half.wait()).await {
            Err(_) => return Err(SshError::new(format!("等待{label}应答超时"))),
            Ok(None) => return Err(SshError::new(format!("{label}通道被远端关闭"))),
            Ok(Some(russh::ChannelMsg::Success)) => return Ok(()),
            Ok(Some(russh::ChannelMsg::Failure)) => {
                return Err(SshError::coded(
                    "ptyDenied",
                    format!("远程主机拒绝{label}请求"),
                ));
            }
            Ok(Some(_)) => {}
        }
    }
}

/// 打开会话 channel 并请求 PTY；ptyDenied 分类与既有行为一致。
async fn open_pty_channel(
    handle: &Handle<ClientHandler>,
    cols: u16,
    rows: u16,
) -> Result<russh::Channel<client::Msg>, SshError> {
    let channel = handle
        .channel_open_session()
        .await
        .map_err(|error| SshError::new(format!("打开 SSH 会话通道失败：{error}")))?;
    // want_reply=true：多一次往返，换来"服务端拒绝分配 PTY"能被立刻发现。
    // 若不等应答，被拒时会得到一个没有行编辑、没有作业控制的残缺终端，
    // 而界面上没有任何提示——OpenSSH 正是用 want_reply 才能报出
    // "PTY allocation request failed"。
    channel
        .request_pty(
            true,
            "xterm-256color",
            u32::from(cols),
            u32::from(rows),
            0,
            0,
            &[],
        )
        .await
        .map_err(|error| {
            SshError::coded(
                "ptyDenied",
                format!("远程主机拒绝分配 PTY：{error}。请确认该账号允许交互式登录（未被限制为仅 SFTP/命令模式）"),
            )
        })?;
    Ok(channel)
}

/// 执行 bootstrap 的探针与预载文件上传；返回 spawn 预载计划。
///
/// 失败返回降级原因。所有失败都不产生用户可见错误——登录 shell 尚未启动，
/// 降级后走普通 request_shell，AI 命令改走独立 exec 通道。
async fn run_bootstrap(
    handle: &Handle<ClientHandler>,
    initial_path: Option<&str>,
) -> BootstrapOutcome {
    // 阶段 1：登录 shell 探针（独立无 PTY channel，短超时）。
    let probe_output = match super::bootstrap::exec_probe(handle).await {
        Ok(output) => output,
        Err(reason) => return BootstrapOutcome::unavailable(reason),
    };
    let shell = match super::bootstrap::parse_shell_probe(&probe_output) {
        Some(shell) => {
            eprintln!("[nocterm-ssh-bootstrap] 识别到 shell：{}", shell.as_str());
            shell
        }
        None => {
            eprintln!(
                "[nocterm-ssh-bootstrap] 未识别到受支持的 shell，降级独立 exec：{probe_output:?}"
            );
            return BootstrapOutcome::unavailable("unsupported_shell");
        }
    };
    // 阶段 2：生成 nonce 与随机远程路径，构造 spawn 预载计划。
    let mut nonce_bytes = [0_u8; 16];
    if getrandom::fill(&mut nonce_bytes).is_err() {
        return BootstrapOutcome::detected_but_unavailable(shell, "bootstrap_unavailable");
    }
    let nonce: String = nonce_bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let plan = SshShellIntegrationPlan { nonce, shell };
    let spawn = super::bootstrap::prepare_spawn(plan.clone(), initial_path);
    // 阶段 3：umask 077 上传预载文件（PowerShell 无文件，内容在启动命令内编码）。
    // rc 尾部自删除；上传中断由 cleanup_files 兜底清理。
    for file in &spawn.files {
        if let Err(reason) =
            super::bootstrap::upload_file(handle, &file.path, file.content.as_bytes()).await
        {
            // 上传中断可能已在远端留下半截文件：清理全部预载路径。
            eprintln!("[nocterm-ssh-bootstrap] 上传失败（{reason}），清理预载文件");
            super::bootstrap::cleanup_files(handle, &spawn.file_paths()).await;
            return BootstrapOutcome::detected_but_unavailable(shell, reason);
        }
    }
    eprintln!(
        "[nocterm-ssh-bootstrap] bootstrap 完成：shell={:?} nonce={} files={}，等待 pty+exec spawn",
        plan.shell.as_str(),
        plan.nonce,
        spawn.files.len()
    );
    BootstrapOutcome::ready(spawn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::connection::{
        AUTH_TIMEOUT, DNS_RESOLVE_TIMEOUT, HANDSHAKE_TIMEOUT, TCP_CONNECT_TIMEOUT,
    };
    #[test]
    fn reports_no_active_session_without_attempting_hidden_reauthentication() {
        let manager = SshTerminalManager::default();
        let result = manager
            .exec_existing(7, "docker ps", &AtomicBool::new(false))
            .expect("session lookup should not fail");
        assert!(result.is_none());
    }

    #[test]
    fn detected_shell_survives_integration_failure_for_terminal_semantics() {
        let outcome = BootstrapOutcome::detected_but_unavailable(
            SshShellKind::PowerShell,
            "upload_remote_failed",
        );

        assert_eq!(outcome.detected_shell, Some(SshShellKind::PowerShell));
        assert!(outcome.spawn.is_none());
        assert_eq!(outcome.fallback_reason, Some("upload_remote_failed"));
        assert_eq!(
            super::super::bootstrap::initial_directory_command(
                r"C:\Users\O'Brien",
                outcome.detected_shell,
            ),
            "Set-Location -LiteralPath 'C:\\Users\\O''Brien'\r\n"
        );
    }

    #[test]
    fn cancelled_exec_is_rejected_before_session_lookup_or_remote_start() {
        let manager = SshTerminalManager::default();
        let cancellation = AtomicBool::new(true);

        let error = match manager.exec_existing(7, "docker ps", &cancellation) {
            Ok(_) => panic!("cancelled command must not open an exec channel"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("未执行"));
    }

    #[test]
    fn execution_registry_can_cancel_before_channel_task_starts() {
        let registry = ExecutionRegistry::default();
        let (cancel_tx, cancel_rx) = oneshot::channel();
        registry
            .insert("exec-early".into(), "terminal-1".into(), cancel_tx)
            .expect("register before dispatch");

        assert!(registry.cancel("exec-early"));
        assert!(Runtime::new().unwrap().block_on(cancel_rx).is_ok());
    }

    #[test]
    fn channel_reader_reports_eof_when_sender_drops() {
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        tx.send(b"hi".to_vec()).expect("send");
        drop(tx);
        let mut reader = ChannelReader {
            rx,
            buffer: Vec::new(),
            position: 0,
        };
        let mut buffer = [0_u8; 8];
        assert_eq!(reader.read(&mut buffer).expect("read"), 2);
        assert_eq!(&buffer[..2], b"hi");
        // 发送端已释放，后续读取应稳定返回 EOF。
        assert_eq!(reader.read(&mut buffer).expect("eof"), 0);
    }

    #[test]
    fn rejects_initial_path_with_newlines() {
        assert!(normalize_initial_path(Some("/tmp\ninvalid")).is_err());
        assert_eq!(
            normalize_initial_path(Some("  /var/log  ")).expect("trim"),
            Some("/var/log".to_string())
        );
        assert_eq!(normalize_initial_path(Some("   ")).expect("empty"), None);
    }

    /// 分阶段超时之和必须落在同步 open() 的兜底预算之内：兜底留在 ssh::mod，
    /// 这里核对两侧常量的约束关系，防止任何一侧单独改动后悄然失衡。
    #[test]
    fn connect_stage_budgets_fit_within_the_ready_backstop() {
        let stages_total =
            DNS_RESOLVE_TIMEOUT + TCP_CONNECT_TIMEOUT + HANDSHAKE_TIMEOUT + AUTH_TIMEOUT;
        assert!(
            stages_total < CONNECT_READY_TIMEOUT,
            "兜底超时必须大于解析+拨号+握手+认证之和"
        );
    }
}
