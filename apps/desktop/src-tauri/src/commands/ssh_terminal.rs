use std::{io::Read, thread};

use nocterm_application::error::AppError;
use nocterm_domain::connection::{AuthenticationMethod, ConnectionProfile};
use tauri::{AppHandle, Emitter, State};

use crate::{
    commands::{
        credential::{read_secret, resolve_private_key},
        terminal_text::Utf8Stream,
    },
    dto::{
        error::ErrorResponse,
        ssh_terminal::{SshTerminalExit, SshTerminalOpenResponse, SshTerminalOutput},
    },
    state::AppState,
};

/// 口令的来源，决定连接失败后是否需要丢弃缓存并让用户重新输入。
#[derive(Debug, PartialEq, Eq)]
enum PasswordSource {
    /// 私钥或 SSH Agent 登录，本次连接不需要口令。
    NotRequired,
    /// 用户本次在终端提示符下输入。
    Typed,
    /// 该连接仍有活跃会话期间缓存的交互口令。
    Session,
    /// 用户显式保存到系统凭据库的口令。
    Stored,
}

/// 纯决策函数：在不接触系统凭据库的前提下确定本次应使用的口令来源。
/// 单独拆出是为了让优先级规则可被单元测试覆盖——读取凭据库需要完整的 `AppState`，
/// 在单元测试里无法装配。返回 `None` 表示三种来源都没有，需要提示用户现场输入。
fn choose_password_source(
    authentication: AuthenticationMethod,
    has_typed: bool,
    has_cached: bool,
    credential_status: &str,
) -> Option<PasswordSource> {
    if authentication != AuthenticationMethod::Password {
        return Some(PasswordSource::NotRequired);
    }
    if has_typed {
        return Some(PasswordSource::Typed);
    }
    if has_cached {
        return Some(PasswordSource::Session);
    }
    (credential_status == "bound").then_some(PasswordSource::Stored)
}

/// 口令来源优先级：本次交互输入 > 会话内存缓存 > 系统凭据库。
///
/// 三者都没有时返回 `SSH_PASSWORD_REQUIRED`，由前端在终端里就地提示用户输入。
/// 不把"必须先保存密码"当作硬性前置条件——进程内 russh 需要在建连前拿到口令，
/// 但这不应该退化成"不保存密码就连不上"，主流客户端都支持连接时现场输入。
fn resolve_password(
    state: &AppState,
    profile: &ConnectionProfile,
    typed: Option<&str>,
) -> Result<(Option<String>, PasswordSource), ErrorResponse> {
    let typed = typed.filter(|value| !value.is_empty());
    let cached = state.session_passwords().get(profile.id);
    let source = choose_password_source(
        profile.authentication,
        typed.is_some(),
        cached.is_some(),
        profile.credential_status.as_str(),
    )
    .ok_or_else(|| {
        ErrorResponse::from(AppError::new(
            "SSH_PASSWORD_REQUIRED",
            "该连接尚未保存登录密码，请在终端中输入",
            true,
        ))
    })?;
    let secret = match source {
        PasswordSource::NotRequired => None,
        PasswordSource::Typed => typed.map(str::to_string),
        PasswordSource::Session => cached,
        PasswordSource::Stored => Some(read_secret(state, &profile.id.to_string(), "password")?),
    };
    Ok((secret, source))
}

/// 建连与认证会阻塞到远端应答（最长 30 秒），必须交给 Tauri 的工作线程执行；
/// 若沿用同步命令会占住主线程，窗口在连接期间无法重绘，与 SFTP 命令的既有约定不一致。
///
/// `password` 只在前端收到 `SSH_PASSWORD_REQUIRED` 后由终端提示符收集并回传，
/// 仅用于本次建连；成功后连同一个会话租约存入内存缓存，不写入系统凭据库。
#[tauri::command(async)]
pub fn ssh_terminal_open(
    app: AppHandle,
    state: State<'_, AppState>,
    connection_id: i64,
    cols: u16,
    rows: u16,
    password: Option<String>,
) -> Result<SshTerminalOpenResponse, ErrorResponse> {
    let profile = state.connection_service().get(connection_id)?;
    // russh 在返回前完成认证，因此口令必须在打开阶段提供而非事后注入 PTY。
    let (password, password_source) = resolve_password(&state, &profile, password.as_deref())?;
    let private_key = if profile.authentication == AuthenticationMethod::PrivateKey {
        Some(resolve_private_key(&state, &profile)?)
    } else {
        None
    };
    let terminal_service = state.terminal_service().clone();
    // 会话收尾发生在输出线程里，那里拿不到 `State`，因此先克隆一份缓存句柄带进去。
    let session_passwords = state.session_passwords().clone();
    let opened = match terminal_service.open(
        &profile,
        cols,
        rows,
        password.as_deref(),
        private_key.as_deref(),
    ) {
        Ok(opened) => {
            // 口令只在该连接还有活跃会话时保留：认证通过即占用一个租约，
            // 由下方输出线程在会话结束时归还，归零后口令丢弃、下次连接重新提示。
            if matches!(
                password_source,
                PasswordSource::Typed | PasswordSource::Session
            ) {
                session_passwords.retain(connection_id, password.as_deref().unwrap_or_default());
            }
            opened
        }
        Err(error) => {
            // 交互来源的口令不可靠（可能输错、也可能服务端已改密），失败即丢弃，
            // 让下一次连接重新提示输入，而不是拿旧口令反复撞服务端的失败计数。
            if matches!(
                password_source,
                PasswordSource::Typed | PasswordSource::Session
            ) {
                session_passwords.forget(connection_id);
            }
            return Err(ErrorResponse::from(error));
        }
    };
    // 只有真正占用了租约的会话才需要归还，否则会把别的会话的计数减掉。
    let holds_password_lease = matches!(
        password_source,
        PasswordSource::Typed | PasswordSource::Session
    );
    let terminal_id = opened.id;
    let mut reader = opened.reader;
    let reader_terminal_id = terminal_id.clone();
    let reader_terminal_service = terminal_service.clone();

    // 通道输出必须离开 IPC 线程，按会话标识隔离后通过事件回传前端。
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        // 网络分片与字符边界无关，解码状态必须跨读取保持，否则汉字会在分片处变成 U+FFFD。
        let mut decoder = Utf8Stream::default();
        let mut failed = false;
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Err(_) => {
                    failed = true;
                    break;
                }
                Ok(length) => {
                    let data = decoder.push(&buffer[..length]);
                    if data.is_empty() {
                        // 整块只有半个字符时先不发事件，等后续分片补齐。
                        continue;
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
        // 主动关闭标签、远端退出与读中断都会走到这里，是归还租约唯一需要的挂钩点：
        // `close` 会中止会话任务并丢弃发送端，读取随即返回 EOF。
        if holds_password_lease {
            session_passwords.release(connection_id);
        }
        let _ = app.emit(
            "ssh-terminal-exit",
            SshTerminalExit {
                terminal_id: reader_terminal_id,
                connection_id,
                // 认证与建连错误在 open 阶段即返回；此处仅区分正常收尾与读中断。
                reason: if failed { "failed" } else { "closed" },
            },
        );
    });

    Ok(SshTerminalOpenResponse { terminal_id })
}

#[cfg(test)]
mod tests {
    use super::{PasswordSource, choose_password_source};
    use nocterm_domain::connection::AuthenticationMethod;

    #[test]
    fn prefers_typed_then_cached_then_stored_password() {
        // 现场输入代表用户最新意图，优先于任何既有口令。
        assert_eq!(
            choose_password_source(AuthenticationMethod::Password, true, true, "bound"),
            Some(PasswordSource::Typed)
        );
        assert_eq!(
            choose_password_source(AuthenticationMethod::Password, false, true, "bound"),
            Some(PasswordSource::Session)
        );
        assert_eq!(
            choose_password_source(AuthenticationMethod::Password, false, false, "bound"),
            Some(PasswordSource::Stored)
        );
    }

    #[test]
    fn asks_for_interactive_input_when_no_password_is_available() {
        // 未绑定凭据且没有缓存时必须提示输入，而不是把连接判死。
        assert_eq!(
            choose_password_source(AuthenticationMethod::Password, false, false, "missing"),
            None
        );
    }

    #[test]
    fn never_requires_a_password_for_key_or_agent_login() {
        assert_eq!(
            choose_password_source(AuthenticationMethod::PrivateKey, false, false, "bound"),
            Some(PasswordSource::NotRequired)
        );
        assert_eq!(
            choose_password_source(AuthenticationMethod::SshAgent, false, false, "missing"),
            Some(PasswordSource::NotRequired)
        );
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

/// 主动关闭只中止会话：输出线程随即读到 EOF，由它统一发出退出事件。
/// 这里再补发一条只会让前端收到重复退出，且当时手上没有真实的 `connection_id`。
#[tauri::command]
pub fn ssh_terminal_close(
    state: State<'_, AppState>,
    terminal_id: String,
) -> Result<(), ErrorResponse> {
    state
        .terminal_service()
        .close(&terminal_id)
        .map_err(ErrorResponse::from)
}
