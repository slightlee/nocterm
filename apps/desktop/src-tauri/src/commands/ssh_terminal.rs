use std::{io::Read, thread, time::Instant};

use nocterm_application::error::AppError;
use nocterm_domain::connection::{AuthenticationMethod, ConnectionProfile};
use nocterm_domain::terminal::SshShellKind;
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
    state::{AppState, INTEGRATION_ECHO_SUPPRESSION_WINDOW, Osc633Scanner},
};

/// 用户输入被 AI 门控时返回的稳定错误码；前端据此提示"AI 命令执行中"。
pub(crate) const SSH_TERMINAL_BUSY_CODE: &str = "SSH_TERMINAL_BUSY";

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
    workspace_session_id: String,
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
    // 绑定控制面并取 generation：重连会得到更大的世代，旧授权自然失效。
    let generation =
        match state
            .ssh_terminals()
            .bind(&workspace_session_id, &terminal_id, connection_id)
        {
            Ok(generation) => generation,
            Err(message) => {
                let _ = terminal_service.close(&terminal_id);
                return Err(ErrorResponse::from(AppError::new(
                    "SSH_TERMINAL_FAILED",
                    message,
                    true,
                )));
            }
        };
    // bootstrap 成功后注册 nonce；只有匹配 nonce 的事件才能作为 Prompt Ready 证据。
    if let Some(plan) = &opened.shell_integration {
        state
            .ssh_terminals()
            .register_integration(&terminal_id, &plan.nonce);
        // integration 注入回显对用户是噪音：POSIX 匹配临时路径 + rm，
        // PowerShell 匹配内存解码调用；只隐藏一次，不解析提示符文本。
        // 抑制窗口有界：截止后注册表降级直通，钩子安装失败不会让终端假死。
        let echo_token = nocterm_infrastructure::ssh::bootstrap::integration_echo_token(plan);
        let required_fragment = if matches!(plan.shell, SshShellKind::PowerShell) {
            "FromBase64String"
        } else {
            "rm -f"
        };
        state.ssh_terminals().begin_integration_echo_suppression(
            &terminal_id,
            &echo_token,
            required_fragment,
            Instant::now() + INTEGRATION_ECHO_SUPPRESSION_WINDOW,
        );
    } else if let Some(reason) = opened.integration_fallback_reason {
        state
            .ssh_terminals()
            .register_integration_unavailable(&terminal_id, reason);
    }

    // 只有 nonce、抑制器和 generation 全部注册后才能写入 bootstrap。此前会话任务
    // 在 open() 发出 ready 后立即注入，首个快速回显可能抢在抑制器之前进入 UI。
    if let Err(error) = terminal_service.activate(&terminal_id) {
        let _ = terminal_service.close(&terminal_id);
        state
            .ssh_terminals()
            .unbind(&workspace_session_id, &terminal_id);
        if holds_password_lease {
            session_passwords.release(connection_id);
        }
        return Err(ErrorResponse::from(error));
    }
    let mut reader = opened.reader;
    let reader_terminal_id = terminal_id.clone();
    let reader_terminal_service = terminal_service.clone();
    let reader_registry = state.ssh_terminals().clone();

    // 通道输出必须离开 IPC 线程，按会话标识隔离后通过事件回传前端。
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        // 网络分片与字符边界无关，解码状态必须跨读取保持，否则汉字会在分片处变成 U+FFFD。
        let mut decoder = Utf8Stream::default();
        // 633 扫描器同样跨分块保持状态：序列可能被 PTY 从任意字节处切开。
        // 控制面扫描发生在 UTF-8 解码之前，与本地终端读取链路一致。
        let mut osc_scanner = Osc633Scanner::new();
        let mut failed = false;
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Err(_) => {
                    failed = true;
                    break;
                }
                Ok(length) => {
                    // 先剥离 shell integration 控制序列：它们不参与解码、过滤与 UI 转发。
                    let (stripped, events) = osc_scanner.feed(&buffer[..length]);
                    let mut visible_data = String::new();
                    if !stripped.is_empty() {
                        let data = decoder.push(&stripped);
                        if !data.is_empty() {
                            // 同一网络块可能同时含最后一段输出和 CommandFinished；必须先
                            // 发布数据再推进完成状态，否则执行线程会先返回并丢掉尾部输出。
                            visible_data =
                                reader_registry.visible_output(&reader_terminal_id, &data);
                            reader_registry.publish(&reader_terminal_id, &data);
                        }
                    }
                    for event in &events {
                        let observation =
                            reader_registry.record_integration_event(&reader_terminal_id, event);
                        if observation.nonce_bound {
                            visible_data.push_str(
                                &reader_registry
                                    .finish_integration_echo_on_osc(&reader_terminal_id),
                            );
                        }
                    }
                    if !visible_data.is_empty() {
                        let _ = app.emit(
                            "ssh-terminal-output",
                            SshTerminalOutput {
                                terminal_id: reader_terminal_id.clone(),
                                connection_id,
                                data: visible_data,
                            },
                        );
                    }
                }
            }
        }
        let _ = reader_terminal_service.close(&reader_terminal_id);
        reader_registry.unbind(&workspace_session_id, &reader_terminal_id);
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

    Ok(SshTerminalOpenResponse {
        terminal_id,
        terminal_generation: generation,
        integration_shell: opened
            .shell_integration
            .as_ref()
            .map(|plan| plan.shell.as_str().to_string()),
        integration_fallback_reason: opened.integration_fallback_reason.map(str::to_string),
    })
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
    // AI 命令执行期间的输入门控：与本地不同，这里返回稳定错误码而不是静默吞键，
    // 让前端能明确提示"本次输入未发送"；不缓存、不延迟重放（方案 §10.6）。
    if !state.ssh_terminals().begin_user_input(&terminal_id) {
        return Err(ErrorResponse::from(AppError::new(
            SSH_TERMINAL_BUSY_CODE,
            "AI 正在执行终端命令，本次输入未发送，请稍后重试",
            false,
        )));
    }
    state
        .terminal_service()
        .write(&terminal_id, &data)
        .map_err(ErrorResponse::from)?;
    Ok(())
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
    // 主动关闭时同步清理控制面；读线程的 EOF 收尾是兜底（读中断时 close 不会被调用）。
    state.ssh_terminals().unbind_terminal(&terminal_id);
    state
        .terminal_service()
        .close(&terminal_id)
        .map_err(ErrorResponse::from)
}
