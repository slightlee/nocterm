use std::{io::Read, thread};

use tauri::{AppHandle, Emitter, State};

use crate::{
    commands::terminal_text::Utf8Stream,
    dto::{
        error::ErrorResponse,
        local_terminal::{LocalTerminalExit, LocalTerminalOpenResponse, LocalTerminalOutput},
    },
    state::{AppState, Osc633Scanner},
};

#[tauri::command]
pub fn local_terminal_open(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<LocalTerminalOpenResponse, ErrorResponse> {
    let terminal_service = state.local_terminal_service().clone();
    let opened = terminal_service
        .open(cols, rows)
        .map_err(ErrorResponse::from)?;
    let terminal_id = opened.id;
    let local_terminals = state.local_terminals().clone();
    local_terminals.bind(session_id.clone(), terminal_id.clone());
    let mut reader = opened.reader;
    let reader_terminal_id = terminal_id.clone();
    let reader_registry = local_terminals.clone();

    // 本地 Shell 输出可能长期阻塞，读取和资源回收都必须离开 IPC 线程。
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        // 解码器跨读取保持状态，否则被 8 KiB 边界切开的汉字会永久变成 U+FFFD。
        let mut decoder = Utf8Stream::default();
        // 633 扫描器同样跨分块保持状态：序列可能被 PTY 从任意字节处切开（#82）。
        let mut osc_scanner = Osc633Scanner::new();
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(length) => {
                    // 先剥离 shell integration 控制序列：它们不参与 UTF-8 解码、
                    // 输出过滤与 UI 转发；事件交注册表驱动集成检测与完成等待。
                    let (stripped, events) = osc_scanner.feed(&buffer[..length]);
                    for event in &events {
                        reader_registry.record_integration_event(&reader_terminal_id, event);
                    }
                    if stripped.is_empty() {
                        continue;
                    }
                    let data = decoder.push(&stripped);
                    if data.is_empty() {
                        // 整块都是某个字符的前半截，等下一块补齐再发，避免前端收到空事件。
                        continue;
                    }
                    let visible_data = reader_registry.visible_output(&reader_terminal_id, &data);
                    // 先推进过滤状态，再唤醒 AI 执行线程，避免清理竞态让内部标记泄漏到 UI。
                    reader_registry.publish(&reader_terminal_id, &data);
                    if !visible_data.is_empty() {
                        let _ = app.emit(
                            "local-terminal-output",
                            LocalTerminalOutput {
                                terminal_id: reader_terminal_id.clone(),
                                session_id: session_id.clone(),
                                data: visible_data,
                            },
                        );
                    }
                }
            }
        }
        let _ = terminal_service.close(&reader_terminal_id);
        reader_registry.unbind(&session_id, &reader_terminal_id);
        let _ = app.emit(
            "local-terminal-exit",
            LocalTerminalExit {
                terminal_id: reader_terminal_id,
                session_id,
            },
        );
    });

    Ok(LocalTerminalOpenResponse { terminal_id })
}

#[tauri::command]
pub fn local_terminal_write(
    state: State<'_, AppState>,
    terminal_id: String,
    data: String,
) -> Result<(), ErrorResponse> {
    let registry = state.local_terminals();
    // AI 命令执行期间的输入门控（#82）：用户键盘写入被静默拦截，避免用户手敲
    // 命令的完成信号冒充 AI 命令的退出码；AI 自身的执行写入不经过本命令。
    if registry.input_gated(&terminal_id) {
        return Ok(());
    }
    state
        .local_terminal_service()
        .write(&terminal_id, &data)
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub fn local_terminal_resize(
    state: State<'_, AppState>,
    terminal_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), ErrorResponse> {
    state
        .local_terminal_service()
        .resize(&terminal_id, cols, rows)
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub fn local_terminal_close(
    state: State<'_, AppState>,
    terminal_id: String,
) -> Result<(), ErrorResponse> {
    let result = state
        .local_terminal_service()
        .close(&terminal_id)
        .map_err(ErrorResponse::from);
    state.local_terminals().unbind_terminal(&terminal_id);
    result
}
