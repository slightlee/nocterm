use std::{io::Read, thread};

use tauri::{AppHandle, Emitter, State};

use crate::{
    commands::terminal_text::Utf8Stream,
    dto::{
        error::ErrorResponse,
        local_terminal::{LocalTerminalExit, LocalTerminalOpenResponse, LocalTerminalOutput},
    },
    state::AppState,
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
    let mut reader = opened.reader;
    let reader_terminal_id = terminal_id.clone();

    // 本地 Shell 输出可能长期阻塞，读取和资源回收都必须离开 IPC 线程。
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        // 解码器跨读取保持状态，否则被 8 KiB 边界切开的汉字会永久变成 U+FFFD。
        let mut decoder = Utf8Stream::default();
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(length) => {
                    let data = decoder.push(&buffer[..length]);
                    if data.is_empty() {
                        // 整块都是某个字符的前半截，等下一块补齐再发，避免前端收到空事件。
                        continue;
                    }
                    let _ = app.emit(
                        "local-terminal-output",
                        LocalTerminalOutput {
                            terminal_id: reader_terminal_id.clone(),
                            session_id: session_id.clone(),
                            data,
                        },
                    );
                }
            }
        }
        let _ = terminal_service.close(&reader_terminal_id);
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
    state
        .local_terminal_service()
        .close(&terminal_id)
        .map_err(ErrorResponse::from)
}
