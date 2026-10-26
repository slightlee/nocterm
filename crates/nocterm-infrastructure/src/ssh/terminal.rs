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
    terminal::{OpenedSshExecution, OpenedTerminal, SshExecutionCompletion, SshTerminalPort},
};
use russh::{ChannelMsg, Disconnect, client, client::Handle};
use tokio::{
    runtime::Runtime,
    sync::{mpsc, oneshot},
    time::timeout,
};

use super::{
    SshError,
    connection::{Auth, ClientHandler, build_auth, connect_authenticated},
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

/// 会话任务能接收的控制指令，均由同步命令层通过通道投递。
enum TerminalCommand {
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

/// 已认证会话持有的 russh 资源三元组：连接句柄 + 读半区 + 写半区。
type Session = (
    Handle<ClientHandler>,
    russh::ChannelReadHalf,
    russh::ChannelWriteHalf<client::Msg>,
);

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
            .map_err(|_| SshError("SSH exec 取消状态锁已损坏".to_string()))?;
        if handles.contains_key(&execution_id) {
            return Err(SshError("SSH exec 执行标识冲突".to_string()));
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
    pub fn open(
        &self,
        profile: &ConnectionProfile,
        cols: u16,
        rows: u16,
        password: Option<&str>,
        private_key: Option<&str>,
    ) -> Result<(String, Box<dyn Read + Send>), SshError> {
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
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();

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
        match ready_rx.recv_timeout(CONNECT_READY_TIMEOUT) {
            Ok(Ok(())) => {}
            Ok(Err(message)) => {
                task.abort();
                return Err(SshError(message));
            }
            Err(_) => {
                task.abort();
                return Err(SshError(format!(
                    "连接 {}:{} 无响应：等待 {} 秒后仍未完成建连，请重试",
                    profile.host,
                    profile.port,
                    CONNECT_READY_TIMEOUT.as_secs()
                )));
            }
        }

        let mut terminals = self
            .terminals
            .lock()
            .map_err(|_| SshError("SSH 终端状态锁已损坏".to_string()))?;
        terminals.insert(
            terminal_id.clone(),
            TerminalHandle {
                connection_id: profile.id,
                command_tx,
                task,
            },
        );
        Ok((
            terminal_id,
            Box::new(ChannelReader {
                rx: output_rx,
                buffer: Vec::new(),
                position: 0,
            }),
        ))
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
            return Err(SshError("AI 任务已停止，远程命令未执行".to_string()));
        }
        let (owner_terminal_id, command_tx) = {
            let mut terminals = self
                .terminals
                .lock()
                .map_err(|_| SshError("SSH 终端状态锁已损坏".to_string()))?;
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
            return Err(SshError("当前 SSH 连接已停止接收 AI 命令".to_string()));
        }
        let deadline = std::time::Instant::now() + EXEC_READY_TIMEOUT;
        loop {
            if cancellation.load(Ordering::Acquire) {
                self.executions.cancel(&execution_id);
                return Err(SshError("AI 任务已停止，远程命令未执行".to_string()));
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                self.executions.cancel(&execution_id);
                return Err(SshError(
                    "在当前 SSH 连接上打开 AI 执行通道超时".to_string(),
                ));
            }
            match ready_rx.recv_timeout(remaining.min(EXEC_CANCEL_POLL_INTERVAL)) {
                Ok(Ok(())) => {
                    if cancellation.load(Ordering::Acquire) {
                        self.executions.cancel(&execution_id);
                        return Err(SshError("AI 任务已停止，远程命令未执行".to_string()));
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
                Ok(Err(message)) => return Err(SshError(message)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    self.executions.cancel(&execution_id);
                    return Err(SshError("SSH exec 通道在启动完成前意外关闭".to_string()));
                }
            }
        }
    }

    pub fn write(&self, terminal_id: &str, data: &str) -> Result<(), SshError> {
        let terminals = self
            .terminals
            .lock()
            .map_err(|_| SshError("SSH 终端状态锁已损坏".to_string()))?;
        let terminal = terminals
            .get(terminal_id)
            .ok_or_else(|| SshError("SSH 终端不存在或已关闭".to_string()))?;
        terminal
            .command_tx
            .send(TerminalCommand::Data(data.as_bytes().to_vec()))
            .map_err(|_| SshError("SSH 终端已停止接收输入".to_string()))
    }

    pub fn resize(&self, terminal_id: &str, cols: u16, rows: u16) -> Result<(), SshError> {
        let terminals = self
            .terminals
            .lock()
            .map_err(|_| SshError("SSH 终端状态锁已损坏".to_string()))?;
        let terminal = terminals
            .get(terminal_id)
            .ok_or_else(|| SshError("SSH 终端不存在或已关闭".to_string()))?;
        terminal
            .command_tx
            .send(TerminalCommand::Resize { cols, rows })
            .map_err(|_| SshError("SSH 终端已停止响应".to_string()))
    }

    pub fn close(&self, terminal_id: &str) -> Result<(), SshError> {
        let handle = self
            .terminals
            .lock()
            .map_err(|_| SshError("SSH 终端状态锁已损坏".to_string()))?
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
            .map_err(|_| SshError("SSH 终端状态锁已损坏".to_string()))?
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
    ) -> Result<OpenedTerminal, String> {
        let (id, reader) =
            SshTerminalManager::open(self, profile, cols, rows, password, private_key)
                .map_err(|error| error.to_string())?;
        Ok(OpenedTerminal { id, reader })
    }

    fn write(&self, terminal_id: &str, data: &str) -> Result<(), String> {
        SshTerminalManager::write(self, terminal_id, data).map_err(|error| error.to_string())
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
        return Err(SshError("远程初始路径不能包含换行符".to_string()));
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
    ready_tx: std::sync::mpsc::Sender<Result<(), String>>,
    terminal_id: String,
    executions: Arc<ExecutionRegistry>,
) {
    // 建连与认证阶段的结果先回传给同步 open()，再进入长期事件循环。
    let (handle, mut read_half, write_half) =
        match connect_session(&host, port, &username, auth, cols, rows).await {
            Ok(session) => {
                let _ = ready_tx.send(Ok(()));
                session
            }
            Err(message) => {
                let _ = ready_tx.send(Err(message));
                return;
            }
        };
    // russh 的 Handle 本身不可 Clone；用异步互斥封装后，AI exec channel 可以在
    // 保持可见 Shell 读写循环运行的同时复用同一条已认证 SSH 连接。
    let handle = Arc::new(tokio::sync::Mutex::new(handle));
    let mut execution_tasks: Vec<tokio::task::JoinHandle<()>> = Vec::new();

    // 若配置了远程初始目录，开壳后立即切换，等价旧实现的 `cd` 行为。
    if let Some(path) = initial_path {
        let escaped = path.replace('\'', "'\\''");
        let _ = write_half.data_bytes(format!("cd '{escaped}'\n")).await;
    }

    loop {
        // 已完成的 AI 通道只留下极短的 JoinHandle；持续对话不会因为历史执行次数
        // 无限增长内存占用。
        execution_tasks.retain(|task| !task.is_finished());
        tokio::select! {
            message = read_half.wait() => match message {
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
    // 尽力礼貌关闭通道并断开连接，忽略此时已不可达的错误。
    let _ = write_half.close().await;
    let handle = handle.lock().await;
    let _ = handle.disconnect(Disconnect::ByApplication, "", "").await;
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

/// 建立 TCP 连接、完成认证、开启带 PTY 的交互式 shell 通道。
async fn connect_session(
    host: &str,
    port: u16,
    username: &str,
    auth: Auth,
    cols: u16,
    rows: u16,
) -> Result<Session, String> {
    let handle = connect_authenticated(host, port, username, auth).await?;

    let channel = handle
        .channel_open_session()
        .await
        .map_err(|error| format!("打开 SSH 会话通道失败：{error}"))?;
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
            format!("远程主机拒绝分配 PTY：{error}。请确认该账号允许交互式登录（未被限制为仅 SFTP/命令模式）")
        })?;
    channel
        .request_shell(true)
        .await
        .map_err(|error| format!("启动远程 Shell 失败：{error}"))?;

    let (read_half, write_half) = channel.split();
    Ok((handle, read_half, write_half))
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
