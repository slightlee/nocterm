use std::{
    collections::HashMap,
    io::{Read, Write},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use nocterm_domain::terminal::{LocalTerminalPort, OpenedTerminal};
use portable_pty::{Child, MasterPty, PtySize, native_pty_system};

/// 默认 Shell 的选择是本模块唯一带平台差异的一步，单独成文件；PTY 生命周期本身
/// 由 `portable-pty` 抹平差异，因此下面的代码不含任何平台 `cfg`。
mod shell;

use shell::local_shell_command;

/// PTY 基础设施错误保留底层上下文，由 Application 边界转换为稳定错误码。
#[derive(Debug)]
pub struct TerminalError(String);

impl std::fmt::Display for TerminalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for TerminalError {}

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
            .spawn_command(local_shell_command())
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

fn error<Source: std::fmt::Display>(context: &'static str) -> impl FnOnce(Source) -> TerminalError {
    move |source| TerminalError(format!("{context}: {source}"))
}

#[cfg(test)]
mod tests {
    use std::{sync::mpsc, thread, time::Duration};

    use super::*;

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
