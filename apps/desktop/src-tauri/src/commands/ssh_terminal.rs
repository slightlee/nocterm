use std::{io::Read, thread};

use tauri::{AppHandle, Emitter, State};

use crate::{
    commands::credential::read_secret,
    dto::{
        error::ErrorResponse,
        ssh_terminal::{SshTerminalExit, SshTerminalOpenResponse, SshTerminalOutput},
    },
    state::AppState,
};

#[tauri::command]
pub fn ssh_terminal_open(
    app: AppHandle,
    state: State<'_, AppState>,
    connection_id: i64,
    cols: u16,
    rows: u16,
) -> Result<SshTerminalOpenResponse, ErrorResponse> {
    let profile = state.connection_service().get(connection_id)?;
    // 未保存密码时保留 OpenSSH 的交互提示，行为与旧客户端一致。
    let password =
        if should_read_saved_password(profile.authentication, profile.credential_status.as_str()) {
            Some(read_secret(&state, &connection_id.to_string(), "password")?)
        } else {
            None
        };
    let private_key =
        if profile.authentication == nocterm_domain::connection::AuthenticationMethod::PrivateKey {
            Some(read_secret(
                &state,
                &connection_id.to_string(),
                "private_key",
            )?)
        } else {
            None
        };
    let terminal_service = state.terminal_service().clone();
    let opened = terminal_service
        .open(&profile, cols, rows, private_key.as_deref())
        .map_err(ErrorResponse::from)?;
    let terminal_id = opened.id;
    let mut reader = opened.reader;
    let reader_terminal_id = terminal_id.clone();
    let reader_terminal_id_for_prompt = terminal_id.clone();
    let reader_terminal_service = terminal_service.clone();

    // PTY 读取必须离开 IPC 线程，输出通过按会话标识隔离的事件回传。
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        let mut prompt_buffer = String::new();
        let mut password = password;
        let mut failed = false;
        let mut timed_out = false;
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Err(_) => {
                    failed = true;
                    break;
                }
                Ok(length) => {
                    let data = String::from_utf8_lossy(&buffer[..length]).to_string();
                    timed_out |= is_ssh_timeout_output(&data);
                    failed |= is_ssh_failure_output(&data);
                    if let Some(secret) = password.as_ref() {
                        prompt_buffer.push_str(&data);
                        if prompt_buffer.len() > 512 {
                            let keep_from = prompt_buffer.len() - 512;
                            prompt_buffer = prompt_buffer[keep_from..].to_string();
                        }
                        if is_password_prompt(&prompt_buffer) {
                            let _ = reader_terminal_service
                                .write(&reader_terminal_id_for_prompt, &format!("{secret}\r"));
                            password = None;
                            prompt_buffer.clear();
                        }
                    }
                    let _ = app.emit(
                        "ssh-terminal-output",
                        SshTerminalOutput {
                            terminal_id: reader_terminal_id.clone(),
                            connection_id,
                            data,
                        },
                    );
                }
            }
        }
        let _ = reader_terminal_service.close(&reader_terminal_id);
        let _ = app.emit(
            "ssh-terminal-exit",
            SshTerminalExit {
                terminal_id: reader_terminal_id,
                connection_id,
                reason: if timed_out {
                    "timed_out"
                } else if failed {
                    "failed"
                } else {
                    "closed"
                },
            },
        );
    });

    Ok(SshTerminalOpenResponse { terminal_id })
}

fn is_password_prompt(output: &str) -> bool {
    let normalized = output.to_ascii_lowercase();
    normalized.contains("password:") && !normalized.contains("passphrase")
}

/// OpenSSH 的失败文本只用于状态归类，绝不作为 UI 错误内容或协议分支。
fn is_ssh_failure_output(output: &str) -> bool {
    let normalized = output.to_ascii_lowercase();
    [
        "permission denied",
        "host key verification failed",
        "connection timed out",
        "could not resolve hostname",
        "connection refused",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

fn is_ssh_timeout_output(output: &str) -> bool {
    output.to_ascii_lowercase().contains("connection timed out")
}

fn should_read_saved_password(
    authentication: nocterm_domain::connection::AuthenticationMethod,
    credential_status: &str,
) -> bool {
    authentication == nocterm_domain::connection::AuthenticationMethod::Password
        && credential_status == "bound"
}

#[cfg(test)]
mod tests {
    use super::{
        is_password_prompt, is_ssh_failure_output, is_ssh_timeout_output,
        should_read_saved_password,
    };
    use nocterm_domain::connection::AuthenticationMethod;

    #[test]
    fn recognizes_password_prompts_case_insensitively() {
        assert!(is_password_prompt("user@host's Password: "));
        assert!(is_password_prompt("PASSWORD:"));
    }

    #[test]
    fn does_not_send_login_password_to_a_key_passphrase_prompt() {
        assert!(!is_password_prompt(
            "Enter passphrase for key '/tmp/id_ed25519':"
        ));
    }

    #[test]
    fn reads_only_a_bound_password_from_the_system_store() {
        assert!(should_read_saved_password(
            AuthenticationMethod::Password,
            "bound"
        ));
        assert!(!should_read_saved_password(
            AuthenticationMethod::Password,
            "missing"
        ));
        assert!(!should_read_saved_password(
            AuthenticationMethod::PrivateKey,
            "bound"
        ));
    }

    #[test]
    fn categorizes_known_openssh_connection_failures_without_exposing_them() {
        assert!(is_ssh_failure_output("Permission denied (publickey)."));
        assert!(is_ssh_failure_output("Connection timed out"));
        assert!(is_ssh_timeout_output("Connection timed out"));
        assert!(!is_ssh_failure_output("Last login: today"));
    }
}

#[tauri::command]
pub fn ssh_terminal_write(
    state: State<'_, AppState>,
    terminal_id: String,
    data: String,
) -> Result<(), ErrorResponse> {
    state
        .terminal_service()
        .write(&terminal_id, &data)
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub fn ssh_terminal_resize(
    state: State<'_, AppState>,
    terminal_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), ErrorResponse> {
    state
        .terminal_service()
        .resize(&terminal_id, cols, rows)
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub fn ssh_terminal_close(
    app: AppHandle,
    state: State<'_, AppState>,
    terminal_id: String,
) -> Result<(), ErrorResponse> {
    state
        .terminal_service()
        .close(&terminal_id)
        .map_err(ErrorResponse::from)?;
    // 主动关闭是用户取消，不应与远端正常退出或连接失败混为一谈。
    let _ = app.emit(
        "ssh-terminal-exit",
        SshTerminalExit {
            terminal_id,
            connection_id: -1,
            reason: "cancelled",
        },
    );
    Ok(())
}
