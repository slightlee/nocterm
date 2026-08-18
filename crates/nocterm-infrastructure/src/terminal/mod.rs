use std::{
    collections::HashMap,
    io::{Read, Write},
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use nocterm_domain::{
    connection::{AuthenticationMethod, ConnectionProfile},
    terminal::{LocalTerminalPort, OpenedTerminal, SshTerminalPort},
};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

/// PTY 基础设施错误保留底层上下文，由 Application 边界转换为稳定错误码。
#[derive(Debug)]
pub struct TerminalError(String);

impl std::fmt::Display for TerminalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for TerminalError {}

struct SshTerminal {
    connection_id: i64,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
    secret_dir: Option<PathBuf>,
}

/// 单一会话管理器统一持有 PTY 资源，避免命令层散落锁与进程生命周期。
#[derive(Default)]
pub struct SshTerminalManager {
    terminals: Mutex<HashMap<String, SshTerminal>>,
    next_id: AtomicU64,
}

impl SshTerminalManager {
    pub fn open(
        &self,
        profile: &ConnectionProfile,
        cols: u16,
        rows: u16,
        private_key: Option<&str>,
    ) -> Result<(String, Box<dyn Read + Send>), TerminalError> {
        let terminal_id = format!("ssh-{}", self.next_id.fetch_add(1, Ordering::Relaxed) + 1);
        let secret_dir =
            private_key.map(|_| std::env::temp_dir().join(format!("nocterm-ssh-{terminal_id}")));
        let private_key_path = if let Some(secret) = private_key {
            let directory = secret_dir.as_ref().expect("private key directory");
            if let Err(source) = std::fs::create_dir_all(directory) {
                cleanup_secret_dir(secret_dir.as_ref());
                return Err(TerminalError(format!(
                    "创建 SSH 临时凭据目录失败: {source}"
                )));
            }
            let path = directory.join("identity");
            if let Err(source) = std::fs::write(&path, secret) {
                cleanup_secret_dir(secret_dir.as_ref());
                return Err(TerminalError(format!("写入 SSH 私钥失败: {source}")));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Err(source) =
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                {
                    cleanup_secret_dir(secret_dir.as_ref());
                    return Err(TerminalError(format!("设置 SSH 私钥权限失败: {source}")));
                }
            }
            Some(path)
        } else {
            None
        };
        let pair = match native_pty_system().openpty(PtySize {
            rows: rows.max(1),
            cols: cols.max(1),
            pixel_width: 0,
            pixel_height: 0,
        }) {
            Ok(pair) => pair,
            Err(source) => {
                cleanup_secret_dir(secret_dir.as_ref());
                return Err(TerminalError(format!("创建 SSH PTY 失败: {source}")));
            }
        };
        let command = match build_command(profile, private_key_path.as_deref()) {
            Ok(command) => command,
            Err(error) => {
                cleanup_secret_dir(secret_dir.as_ref());
                return Err(error);
            }
        };
        let mut child = match pair.slave.spawn_command(command) {
            Ok(child) => child,
            Err(source) => {
                cleanup_secret_dir(secret_dir.as_ref());
                return Err(TerminalError(format!("启动 SSH 进程失败: {source}")));
            }
        };
        let writer = match pair.master.take_writer() {
            Ok(writer) => writer,
            Err(source) => {
                let _ = child.kill();
                cleanup_secret_dir(secret_dir.as_ref());
                return Err(TerminalError(format!("打开 SSH 输入流失败: {source}")));
            }
        };
        let reader = match pair.master.try_clone_reader() {
            Ok(reader) => reader,
            Err(source) => {
                let _ = child.kill();
                cleanup_secret_dir(secret_dir.as_ref());
                return Err(TerminalError(format!("打开 SSH 输出流失败: {source}")));
            }
        };

        let mut terminals = match self.terminals.lock() {
            Ok(terminals) => terminals,
            Err(_) => {
                let _ = child.kill();
                cleanup_secret_dir(secret_dir.as_ref());
                return Err(TerminalError("SSH 终端状态锁已损坏".to_string()));
            }
        };
        terminals.insert(
            terminal_id.clone(),
            SshTerminal {
                connection_id: profile.id,
                writer,
                child,
                master: pair.master,
                secret_dir: secret_dir.clone(),
            },
        );
        Ok((terminal_id, reader))
    }

    pub fn write(&self, terminal_id: &str, data: &str) -> Result<(), TerminalError> {
        let mut terminals = self
            .terminals
            .lock()
            .map_err(|_| TerminalError("SSH 终端状态锁已损坏".to_string()))?;
        let terminal = terminals
            .get_mut(terminal_id)
            .ok_or_else(|| TerminalError("SSH 终端不存在或已关闭".to_string()))?;
        terminal
            .writer
            .write_all(data.as_bytes())
            .and_then(|_| terminal.writer.flush())
            .map_err(error("写入 SSH 终端失败"))
    }

    pub fn resize(&self, terminal_id: &str, cols: u16, rows: u16) -> Result<(), TerminalError> {
        let terminals = self
            .terminals
            .lock()
            .map_err(|_| TerminalError("SSH 终端状态锁已损坏".to_string()))?;
        let terminal = terminals
            .get(terminal_id)
            .ok_or_else(|| TerminalError("SSH 终端不存在或已关闭".to_string()))?;
        terminal
            .master
            .resize(PtySize {
                rows: rows.max(1),
                cols: cols.max(1),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(error("调整 SSH 终端大小失败"))
    }

    pub fn close(&self, terminal_id: &str) -> Result<(), TerminalError> {
        let mut terminal = self
            .terminals
            .lock()
            .map_err(|_| TerminalError("SSH 终端状态锁已损坏".to_string()))?
            .remove(terminal_id);
        if let Some(terminal) = terminal.as_mut() {
            let kill_error = terminal.child.kill().err();
            cleanup_secret_dir(terminal.secret_dir.take().as_ref());
            if let Some(source) = kill_error {
                return Err(TerminalError(format!("关闭 SSH 终端失败: {source}")));
            }
        }
        Ok(())
    }

    pub fn close_connection(&self, connection_id: i64) -> Result<(), TerminalError> {
        let terminal_ids = self
            .terminals
            .lock()
            .map_err(|_| TerminalError("SSH 终端状态锁已损坏".to_string()))?
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
        let terminals = match self.terminals.get_mut() {
            Ok(terminals) => std::mem::take(terminals),
            Err(_) => return,
        };
        for mut terminal in terminals.into_values() {
            let _ = terminal.child.kill();
            cleanup_secret_dir(terminal.secret_dir.take().as_ref());
        }
    }
}

impl SshTerminalPort for SshTerminalManager {
    fn open(
        &self,
        profile: &ConnectionProfile,
        cols: u16,
        rows: u16,
        private_key: Option<&str>,
    ) -> Result<OpenedTerminal, String> {
        let (id, reader) = SshTerminalManager::open(self, profile, cols, rows, private_key)
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
}

struct LocalTerminal {
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
}

/// 本地终端管理器只负责默认 Shell 的 PTY 资源，不读取连接或凭据资料。
#[derive(Default)]
pub struct LocalTerminalManager {
    terminals: Mutex<HashMap<String, LocalTerminal>>,
    next_id: AtomicU64,
}

impl LocalTerminalManager {
    pub fn open(
        &self,
        cols: u16,
        rows: u16,
    ) -> Result<(String, Box<dyn Read + Send>), TerminalError> {
        let terminal_id = format!("local-{}", self.next_id.fetch_add(1, Ordering::Relaxed) + 1);
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: rows.max(1),
                cols: cols.max(1),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(error("创建本地 PTY 失败"))?;
        let mut child = pair
            .slave
            .spawn_command(CommandBuilder::new_default_prog())
            .map_err(error("启动本地 Shell 失败"))?;
        let writer = match pair.master.take_writer() {
            Ok(writer) => writer,
            Err(source) => {
                let _ = child.kill();
                return Err(TerminalError(format!("打开本地终端输入流失败: {source}")));
            }
        };
        let reader = match pair.master.try_clone_reader() {
            Ok(reader) => reader,
            Err(source) => {
                let _ = child.kill();
                return Err(TerminalError(format!("打开本地终端输出流失败: {source}")));
            }
        };
        let mut terminals = match self.terminals.lock() {
            Ok(terminals) => terminals,
            Err(_) => {
                let _ = child.kill();
                return Err(TerminalError("本地终端状态锁已损坏".to_string()));
            }
        };
        terminals.insert(
            terminal_id.clone(),
            LocalTerminal {
                writer,
                child,
                master: pair.master,
            },
        );
        Ok((terminal_id, reader))
    }

    pub fn write(&self, terminal_id: &str, data: &str) -> Result<(), TerminalError> {
        let mut terminals = self
            .terminals
            .lock()
            .map_err(|_| TerminalError("本地终端状态锁已损坏".to_string()))?;
        let terminal = terminals
            .get_mut(terminal_id)
            .ok_or_else(|| TerminalError("本地终端不存在或已关闭".to_string()))?;
        terminal
            .writer
            .write_all(data.as_bytes())
            .and_then(|_| terminal.writer.flush())
            .map_err(error("写入本地终端失败"))
    }

    pub fn resize(&self, terminal_id: &str, cols: u16, rows: u16) -> Result<(), TerminalError> {
        let terminals = self
            .terminals
            .lock()
            .map_err(|_| TerminalError("本地终端状态锁已损坏".to_string()))?;
        let terminal = terminals
            .get(terminal_id)
            .ok_or_else(|| TerminalError("本地终端不存在或已关闭".to_string()))?;
        terminal
            .master
            .resize(PtySize {
                rows: rows.max(1),
                cols: cols.max(1),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(error("调整本地终端大小失败"))
    }

    pub fn close(&self, terminal_id: &str) -> Result<(), TerminalError> {
        let mut terminal = self
            .terminals
            .lock()
            .map_err(|_| TerminalError("本地终端状态锁已损坏".to_string()))?
            .remove(terminal_id);
        if let Some(terminal) = terminal.as_mut()
            && let Err(source) = terminal.child.kill()
        {
            return Err(TerminalError(format!("关闭本地终端失败: {source}")));
        }
        Ok(())
    }
}

impl Drop for LocalTerminalManager {
    fn drop(&mut self) {
        let terminals = match self.terminals.get_mut() {
            Ok(terminals) => std::mem::take(terminals),
            Err(_) => return,
        };
        for mut terminal in terminals.into_values() {
            let _ = terminal.child.kill();
        }
    }
}

impl LocalTerminalPort for LocalTerminalManager {
    fn open(&self, cols: u16, rows: u16) -> Result<OpenedTerminal, String> {
        let (id, reader) =
            LocalTerminalManager::open(self, cols, rows).map_err(|error| error.to_string())?;
        Ok(OpenedTerminal { id, reader })
    }

    fn write(&self, terminal_id: &str, data: &str) -> Result<(), String> {
        LocalTerminalManager::write(self, terminal_id, data).map_err(|error| error.to_string())
    }

    fn resize(&self, terminal_id: &str, cols: u16, rows: u16) -> Result<(), String> {
        LocalTerminalManager::resize(self, terminal_id, cols, rows)
            .map_err(|error| error.to_string())
    }

    fn close(&self, terminal_id: &str) -> Result<(), String> {
        LocalTerminalManager::close(self, terminal_id).map_err(|error| error.to_string())
    }
}

fn cleanup_secret_dir(directory: Option<&PathBuf>) {
    if let Some(directory) = directory {
        let _ = std::fs::remove_dir_all(directory);
    }
}

fn build_command(
    profile: &ConnectionProfile,
    private_key_path: Option<&std::path::Path>,
) -> Result<CommandBuilder, TerminalError> {
    let remote_initial_path = profile
        .remote_initial_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if remote_initial_path.is_some_and(|value| value.contains('\n') || value.contains('\r')) {
        return Err(TerminalError("远程初始路径不能包含换行符".to_string()));
    }
    let mut command = CommandBuilder::new(ssh_binary());
    command.args([
        "-tt",
        "-p",
        &profile.port.to_string(),
        "-o",
        "ServerAliveInterval=30",
        "-o",
        "ServerAliveCountMax=3",
        "-o",
        "ConnectTimeout=15",
        "-o",
        // 首次连接必须由用户在终端确认指纹，禁止静默写入 known_hosts。
        "StrictHostKeyChecking=ask",
    ]);
    if profile.authentication == AuthenticationMethod::Password {
        command.args([
            "-o",
            "PreferredAuthentications=password,keyboard-interactive",
            "-o",
            "PubkeyAuthentication=no",
        ]);
    }
    if private_key_path.is_some() || profile.authentication == AuthenticationMethod::SshAgent {
        command.args(["-o", "BatchMode=yes"]);
    }
    if let Some(path) = private_key_path {
        command.args(["-i", path.to_string_lossy().as_ref()]);
    }
    command.arg(format!("{}@{}", profile.username, profile.host));
    if let Some(path) = remote_initial_path {
        let escaped_path = path.replace('\'', "'\"'\"");
        command.arg(format!("cd '{escaped_path}' && exec ${{SHELL:-sh}} -l"));
    }
    Ok(command)
}

#[cfg(target_os = "windows")]
const fn ssh_binary() -> &'static str {
    "ssh.exe"
}

#[cfg(not(target_os = "windows"))]
const fn ssh_binary() -> &'static str {
    "ssh"
}

fn error<Source: std::fmt::Display>(context: &'static str) -> impl FnOnce(Source) -> TerminalError {
    move |source| TerminalError(format!("{context}: {source}"))
}

#[cfg(test)]
mod tests {
    use std::{sync::mpsc, thread, time::Duration};

    use super::*;

    #[test]
    fn removes_private_key_directory_when_command_validation_fails() {
        let manager = SshTerminalManager::default();
        let directory = std::env::temp_dir().join("nocterm-ssh-ssh-1");
        let _ = std::fs::remove_dir_all(&directory);
        let profile = ConnectionProfile {
            id: 1,
            name: "Smoke test".to_string(),
            host: "127.0.0.1".to_string(),
            port: 22,
            username: "tester".to_string(),
            authentication: AuthenticationMethod::PrivateKey,
            created_at: 0,
            updated_at: 0,
            group_id: None,
            group_name: None,
            remark: None,
            sync_mode: "local_only".to_string(),
            execution_target: "remote_terminal".to_string(),
            remote_initial_path: Some("/tmp\ninvalid".to_string()),
            icon: None,
            sort_order: None,
            credential_kind: Some("private_key".to_string()),
            credential_status: "bound".to_string(),
        };

        let result = manager.open(&profile, 80, 24, Some("not-a-real-key"));

        assert!(result.is_err());
        assert!(!directory.exists());
    }

    #[test]
    fn opens_local_shell_and_exchanges_pty_output() {
        let manager = LocalTerminalManager::default();
        let (terminal_id, mut reader) = manager.open(80, 24).expect("open local terminal");
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut buffer = [0_u8; 4096];
            while let Ok(length) = reader.read(&mut buffer) {
                if length == 0 {
                    break;
                }
                if sender
                    .send(String::from_utf8_lossy(&buffer[..length]).to_string())
                    .is_err()
                {
                    break;
                }
            }
        });

        manager
            .write(&terminal_id, "echo NOCTERM_LOCAL_PTY_READY\r")
            .expect("write local terminal");
        let mut output = String::new();
        while !output.contains("NOCTERM_LOCAL_PTY_READY") {
            output.push_str(
                &receiver
                    .recv_timeout(Duration::from_secs(3))
                    .expect("read local terminal output"),
            );
        }
        manager.resize(&terminal_id, 100, 30).expect("resize");
        manager.close(&terminal_id).expect("close");
    }
}
