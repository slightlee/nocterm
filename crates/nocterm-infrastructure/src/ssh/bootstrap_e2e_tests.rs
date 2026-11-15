//! SSH bootstrap 全链路集成测试：进程内 russh 内存服务器 + 真实
//! `SshTerminalManager` 连接，自主验证 POSIX/PowerShell 探针、预载文件上传与
//! pty+exec spawn 启动，不依赖任何外部服务器或人工操作。
//!
//! 桩服务器模拟 bash 与 Windows OpenSSH PowerShell 登录环境：
//! - exec 探针命令 → 返回版本串 `5.2.1|0.0||`（PowerShell 场景先让 POSIX 落空）
//! - exec `umask 077 && mkdir -p && cat > <path>` → 收集 stdin 为预载文件内容
//! - pty 通道上的 exec spawn → 记录启动命令、应答 Success，并模拟钩子在首个
//!   提示符发出 nonce 匹配的 `633;D;0;<nonce>` / `633;A;<nonce>`
//! - 可配置拒绝 spawn（`reject_spawn`）→ 应答 Failure，验证降级回 request_shell
//!
//! 断言覆盖架构不变量与真实环境回归：
//! 1. 集成 shell 经 pty+exec 启动，全程**无 shell-request、无 PTY 敲入**；
//! 2. 预载 rc 含 nonce 导出、登录链复刻、MOTD 补偿与尾部自删除；
//! 3. 回传链路契约——桩发出的 nonce-bound OSC 633 必须到达 reader（命令层
//!    Prompt Ready 证据的物理来源）；
//! 4. PowerShell 先经历 POSIX 未命中再走专用探针；无临时文件，spawn 命令携带
//!    `-EncodedCommand`；
//! 5. spawn 被拒时降级 request_shell、清理已上传文件、fallback 原因可观测。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use russh::ChannelId;
use russh::keys::PrivateKey;
use russh::server::{Auth, Handler, Msg, Server as _, Session};

use super::terminal::SshTerminalManager;
use nocterm_domain::connection::{AuthenticationMethod, ConnectionProfile};
use nocterm_domain::terminal::SshShellKind;

/// 桩与测试断言共享的状态。
#[derive(Default)]
struct SharedState {
    /// 探针调用次数。
    probe_count: usize,
    /// 上传的预载文件：远端路径 → 内容。
    uploaded_rcs: HashMap<String, String>,
    /// pty 通道上收到的 spawn 启动命令（按到达顺序）。
    spawn_commands: Vec<String>,
    /// 收到的 shell-request 次数（降级路径才会出现）。
    shell_request_count: usize,
    /// 可见 shell 通道收到的敲入（仅降级路径的初始目录 cd）。
    shell_inputs: Vec<String>,
    /// 兜底清理命令删除的路径。
    cleaned_paths: Vec<String>,
    /// 已建立 pty 的通道。
    shell_channels: HashSet<russh::ChannelId>,
}

/// 一次 exec 的处置结果，由 Handler 主体写入共享状态。
#[derive(Clone)]
enum ExecKind {
    Probe,
    Upload {
        path: String,
    },
    /// pty 通道上的集成 shell 启动命令。
    Spawn,
    /// 兜底清理命令（路径已写入 shared.cleaned_paths）。
    Cleanup,
    Other,
}

struct TestServer {
    shared: Arc<Mutex<SharedState>>,
    shell: TestShell,
    reject_spawn: bool,
}

#[derive(Clone, Copy)]
enum TestShell {
    Bash,
    PowerShell,
}

impl russh::server::Server for TestServer {
    type Handler = TestHandler;
    fn new_client(&mut self, _peer: Option<std::net::SocketAddr>) -> Self::Handler {
        TestHandler {
            shared: Arc::clone(&self.shared),
            uploads: Mutex::new(HashMap::new()),
            shell: self.shell,
            reject_spawn: self.reject_spawn,
        }
    }
}

struct TestHandler {
    shared: Arc<Mutex<SharedState>>,
    /// exec 通道 → 处置类型；数据通道据此分派。
    uploads: Mutex<HashMap<russh::ChannelId, ExecKind>>,
    shell: TestShell,
    reject_spawn: bool,
}

impl Handler for TestHandler {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(match (user, password) {
            ("test", "test") => Auth::Accept,
            _ => Auth::reject(),
        })
    }

    async fn channel_open_session(
        &mut self,
        _channel: russh::Channel<Msg>,
        reply: russh::server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        _col: u32,
        _row: u32,
        _w: u32,
        _h: u32,
        _modes: &[(russh::Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.shared.lock().unwrap().shell_channels.insert(channel);
        session.channel_success(channel)?;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.shared.lock().unwrap().shell_request_count += 1;
        session.channel_success(channel)?;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let command = String::from_utf8_lossy(data).to_string();
        let on_pty_shell = self
            .shared
            .lock()
            .unwrap()
            .shell_channels
            .contains(&channel);
        let kind = if command.contains("$BASH_VERSION") {
            let mut shared = self.shared.lock().unwrap();
            shared.probe_count += 1;
            let output = match self.shell {
                TestShell::Bash => b"5.2.1|0.0||\n".to_vec(),
                TestShell::PowerShell => b"|||\n".to_vec(),
            };
            session.data(channel, output)?;
            ExecKind::Probe
        } else if command.contains("__NOCTERM_POWERSHELL__") {
            self.shared.lock().unwrap().probe_count += 1;
            if matches!(self.shell, TestShell::PowerShell) {
                session.data(channel, b"__NOCTERM_POWERSHELL__|7.5.4\r\n".to_vec())?;
            }
            ExecKind::Probe
        } else if let Some(path) = command
            .split("&& cat > ")
            .nth(1)
            .map(|tail| tail.trim().trim_matches('\'').to_string())
        {
            ExecKind::Upload { path }
        } else if let Some(path) = command
            .strip_prefix("rm -rf -- ")
            .or_else(|| command.strip_prefix("rm -f -- "))
            .map(|tail| tail.trim().trim_matches('\'').to_string())
        {
            self.shared.lock().unwrap().cleaned_paths.push(path.clone());
            ExecKind::Cleanup
        } else if on_pty_shell {
            self.shared
                .lock()
                .unwrap()
                .spawn_commands
                .push(command.clone());
            ExecKind::Spawn
        } else {
            ExecKind::Other
        };
        match &kind {
            // 探针：回版本串，EOF + 关闭。
            ExecKind::Probe => {
                session.eof(channel)?;
                session.close(channel)?;
            }
            // 上传：等客户端 stdin 数据（data 事件）与 EOF（channel_eof）。
            ExecKind::Upload { path } => {
                self.uploads
                    .lock()
                    .unwrap()
                    .insert(channel, ExecKind::Upload { path: path.clone() });
                session.channel_success(channel)?;
            }
            // spawn：应答 Success/Failure；成功后模拟钩子在首个提示符的 OSC 输出。
            ExecKind::Spawn => {
                if self.reject_spawn {
                    session.channel_failure(channel)?;
                    return Ok(());
                }
                session.channel_success(channel)?;
                let nonce = {
                    let shared = self.shared.lock().unwrap();
                    shared
                        .uploaded_rcs
                        .values()
                        .find_map(|rc| extract_nonce_from_rc(rc))
                        .or_else(|| decode_encoded_nonce(&command))
                };
                if let Some(nonce) = nonce {
                    session.data(
                        channel,
                        format!("\r\n\x1b]633;D;0;{nonce}\x07\x1b]633;A;{nonce}\x07").into_bytes(),
                    )?;
                }
            }
            // 清理与未知命令：应答后立即收尾。
            ExecKind::Cleanup | ExecKind::Other => {
                session.channel_success(channel)?;
                session.eof(channel)?;
                session.close(channel)?;
            }
        }
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let text = String::from_utf8_lossy(data).to_string();
        let kind = self.uploads.lock().unwrap().get(&channel).cloned();
        match kind {
            // 上传通道的 stdin：预载文件内容提交。
            Some(ExecKind::Upload { path }) => {
                self.shared
                    .lock()
                    .unwrap()
                    .uploaded_rcs
                    .insert(path, text.clone());
                session.exit_status_request(channel, 0)?;
                session.eof(channel)?;
                session.close(channel)?;
            }
            // 可见 shell 通道的敲入（仅降级路径的初始目录 cd）。
            _ => {
                self.shared.lock().unwrap().shell_inputs.push(text.clone());
            }
        }
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        // 上传通道 EOF：内容收集完毕（Nocterm 侧 upload_file 等待通道关闭）。
        let kind = self.uploads.lock().unwrap().get(&channel).cloned();
        if matches!(kind, Some(ExecKind::Upload { .. })) {
            // 内容已在 data 事件中提交；此处仅兜底终结通道。
            session.close(channel)?;
        }
        Ok(())
    }
}

/// 从 rc 内容提取 nonce（`export NOCTERM_OSC_NONCE='<hex>'`）。
fn extract_nonce_from_rc(rc: &str) -> Option<String> {
    rc.lines().find_map(|line| {
        let value = line
            .trim_start()
            .strip_prefix("export NOCTERM_OSC_NONCE='")?;
        let value = value.strip_suffix('\'')?;
        Some(value.to_string())
    })
}

/// 从 PowerShell `-EncodedCommand` 的 Base64(UTF-16LE) 载荷中解出 nonce。
fn decode_encoded_nonce(command: &str) -> Option<String> {
    let payload = command.split("-EncodedCommand ").nth(1)?.trim();
    let bytes = decode_base64(payload)?;
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let script = String::from_utf16(&units).ok()?;
    let value = script.split("$env:NOCTERM_OSC_NONCE = '").nth(1)?;
    Some(value.split('\'').next()?.to_string())
}

/// 测试专用 Base64 解码（标准字母表 + 填充），仅用于验证 EncodedCommand 载荷。
fn decode_base64(input: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut values = Vec::with_capacity(input.len());
    for character in input.bytes() {
        if character == b'=' {
            break;
        }
        let index = ALPHABET
            .iter()
            .position(|candidate| *candidate == character)?;
        values.push(index as u32);
    }
    let mut out = Vec::with_capacity(values.len() * 3 / 4);
    for group in values.chunks(4) {
        let mut acc = 0_u32;
        for (offset, value) in group.iter().enumerate() {
            acc |= value << (18 - 6 * offset);
        }
        match group.len() {
            4 => out.extend_from_slice(&[(acc >> 16) as u8, (acc >> 8) as u8, acc as u8]),
            3 => out.extend_from_slice(&[(acc >> 16) as u8, (acc >> 8) as u8]),
            2 => out.push((acc >> 16) as u8),
            _ => return None,
        }
    }
    Some(out)
}

fn test_profile(port: u16) -> ConnectionProfile {
    ConnectionProfile {
        id: 1,
        name: "in-memory-test".to_string(),
        host: "127.0.0.1".to_string(),
        port,
        username: "test".to_string(),
        authentication: AuthenticationMethod::Password,
        created_at: 0,
        updated_at: 0,
        group_id: None,
        group_name: None,
        remark: None,
        sync_mode: "manual".to_string(),
        execution_target: "ssh".to_string(),
        remote_initial_path: None,
        icon: None,
        sort_order: None,
        private_key_path: None,
        credential_kind: None,
        credential_status: "none".to_string(),
    }
}

fn start_server(
    shell: TestShell,
    shared: Arc<Mutex<SharedState>>,
) -> (u16, std::thread::JoinHandle<()>) {
    start_server_with(shell, shared, false)
}

fn start_server_with(
    shell: TestShell,
    shared: Arc<Mutex<SharedState>>,
    reject_spawn: bool,
) -> (u16, std::thread::JoinHandle<()>) {
    let (port_tx, port_rx) = std::sync::mpsc::channel();
    let server_thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().expect("server runtime");
        runtime.block_on(async move {
            let config = Arc::new(russh::server::Config {
                keys: vec![
                    PrivateKey::random(&mut rand::rng(), russh::keys::Algorithm::Ed25519)
                        .expect("random host key"),
                ],
                ..Default::default()
            });
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind in-memory server");
            let port = listener.local_addr().expect("local addr").port();
            port_tx.send(port).expect("send port");
            let mut server = TestServer {
                shared,
                shell,
                reject_spawn,
            };
            server
                .run_on_socket(config, &listener)
                .await
                .expect("server run");
        });
    });
    let port = port_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("server port");
    (port, server_thread)
}

/// 等待 reader 收齐 nonce-bound OSC 633 事件（D 与 A）。reader 被消费。
fn wait_for_osc_events(mut reader: Box<dyn std::io::Read + Send>) -> String {
    let (chunk_tx, chunk_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buffer = [0_u8; 1024];
        while let Ok(length) = reader.read(&mut buffer) {
            if length == 0 || chunk_tx.send(buffer[..length].to_vec()).is_err() {
                break;
            }
        }
    });
    let mut received = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if let Ok(chunk) = chunk_rx.recv_timeout(std::time::Duration::from_millis(100)) {
            received.extend_from_slice(&chunk);
        }
        let finished = ["633;D;0;", "633;A;"].iter().all(|marker| {
            std::str::from_utf8(&received)
                .map(|text| text.contains(marker))
                .unwrap_or(false)
        });
        if finished {
            break;
        }
    }
    String::from_utf8_lossy(&received).into_owned()
}

#[test]
fn bootstrap_full_flow_over_in_memory_ssh_server() {
    let shared = Arc::new(Mutex::new(SharedState::default()));
    let (port, server_thread) = start_server(TestShell::Bash, Arc::clone(&shared));
    let manager = SshTerminalManager::default();
    let profile = test_profile(port);
    let opened = manager
        .open(&profile, 80, 24, Some("test"), None)
        .expect("open must succeed");
    let terminal_id = opened.terminal_id.clone();

    // bootstrap 结论：集成计划为 bash（探针 + 上传 + spawn 全通过）。
    let plan = opened
        .integration
        .shell_integration
        .expect("bootstrap must produce an integration plan");
    assert_eq!(plan.shell, SshShellKind::Bash);
    assert!(opened.integration.fallback_reason.is_none());

    // 架构不变量：activate 在支持 shell 的 spawn 预载路径下是 no-op，
    // 不向可见 PTY 写入任何字节。
    std::thread::sleep(std::time::Duration::from_millis(100));
    manager
        .activate(&terminal_id)
        .expect("activation must be a no-op for spawn-preloaded sessions");
    std::thread::sleep(std::time::Duration::from_millis(100));

    // 回传链路契约：桩模拟钩子生效后发出的 nonce-bound OSC 633 必须经
    // output_tx 到达 reader——这是命令层 Prompt Ready 证据的物理来源。
    let echoed = wait_for_osc_events(opened.reader);
    assert!(
        echoed.contains(&format!("633;D;0;{}", plan.nonce)) && echoed.contains("633;A;"),
        "reader must receive nonce-bound OSC 633 events, got {echoed:?}"
    );

    let shared = shared.lock().unwrap();
    // 探针被调用；集成 shell 经 pty+exec 启动，无任何 shell-request。
    assert_eq!(shared.probe_count, 1);
    assert_eq!(
        shared.shell_request_count, 0,
        "integrated sessions must not fall back to request_shell"
    );
    let expected_path = super::bootstrap::remote_rc_path(&plan.nonce);
    assert!(
        shared
            .spawn_commands
            .iter()
            .any(|command| command == &format!("exec \"$SHELL\" --rcfile '{expected_path}' -i")),
        "pty channel must exec the spawn command; got {:?}",
        shared.spawn_commands
    );
    assert!(
        shared.shell_inputs.is_empty(),
        "nothing may be typed into the visible PTY; got {:?}",
        shared.shell_inputs
    );

    // 预载 rc 内容契约：nonce 导出、登录链复刻、MOTD 补偿、钩子安装、尾部自删除。
    let rc = shared
        .uploaded_rcs
        .get(&expected_path)
        .expect("rc must be uploaded to the expected path");
    assert!(
        rc.contains(&format!("export NOCTERM_OSC_NONCE='{}'", plan.nonce)),
        "rc must export the nonce"
    );
    assert!(
        rc.contains(". /etc/profile"),
        "login chain must be replicated"
    );
    assert!(
        rc.contains("for _nocterm_f in \"$HOME/.bash_profile\""),
        "user login file chain must be replicated"
    );
    assert!(
        rc.contains("cat /var/run/motd.dynamic"),
        "MOTD compensation must be present"
    );
    assert!(
        rc.contains("PROMPT_COMMAND="),
        "bash hooks must be installed after the user chain"
    );
    assert!(
        rc.contains(&format!("rm -f -- '{expected_path}'")),
        "rc must delete itself after reading"
    );
    assert_eq!(plan.nonce, extract_nonce_from_rc(rc).expect("nonce in rc"));
    drop(server_thread);
}

#[test]
fn powershell_bootstrap_uses_second_probe_and_encoded_spawn() {
    let shared = Arc::new(Mutex::new(SharedState::default()));
    let (port, server_thread) = start_server(TestShell::PowerShell, Arc::clone(&shared));
    let manager = SshTerminalManager::default();
    let opened = manager
        .open(&test_profile(port), 80, 24, Some("test"), None)
        .expect("open must succeed");
    let terminal_id = opened.terminal_id.clone();
    let plan = opened
        .integration
        .shell_integration
        .expect("PowerShell bootstrap must produce an integration plan");
    assert_eq!(plan.shell, SshShellKind::PowerShell);

    std::thread::sleep(std::time::Duration::from_millis(100));
    manager
        .activate(&terminal_id)
        .expect("activation must be a no-op for spawn-preloaded sessions");
    std::thread::sleep(std::time::Duration::from_millis(100));

    let echoed = wait_for_osc_events(opened.reader);
    assert!(
        echoed.contains(&format!("633;D;0;{}", plan.nonce)),
        "reader must receive nonce-bound OSC events, got {echoed:?}"
    );

    let shared = shared.lock().unwrap();
    assert_eq!(shared.probe_count, 2);
    // PowerShell 无临时文件；spawn 命令携带 EncodedCommand 且解出的 nonce 与计划一致。
    assert!(shared.uploaded_rcs.is_empty());
    assert_eq!(shared.shell_request_count, 0);
    assert!(
        shared
            .spawn_commands
            .iter()
            .any(|command| command.starts_with("powershell.exe -NoLogo -NoExit -EncodedCommand ")),
        "pty channel must exec the encoded PowerShell spawn; got {:?}",
        shared.spawn_commands
    );
    assert_eq!(
        shared
            .spawn_commands
            .iter()
            .find_map(|command| decode_encoded_nonce(command))
            .as_deref(),
        Some(plan.nonce.as_str())
    );
    drop(server_thread);
}

/// spawn 被远端拒绝（ForceCommand、受限 shell 等）时：降级普通 request_shell、
/// 已上传的预载文件被兜底清理、降级原因可观测、open 不失败。
#[test]
fn rejected_spawn_falls_back_to_plain_login_shell_and_cleans_up() {
    let shared = Arc::new(Mutex::new(SharedState::default()));
    let (port, server_thread) = start_server_with(TestShell::Bash, Arc::clone(&shared), true);
    let manager = SshTerminalManager::default();
    let opened = manager
        .open(&test_profile(port), 80, 24, Some("test"), None)
        .expect("open must succeed even when spawn is rejected");

    assert!(opened.integration.shell_integration.is_none());
    assert_eq!(
        opened.integration.fallback_reason,
        Some("spawn_unavailable"),
        "fallback reason must be observable for diagnostics"
    );

    std::thread::sleep(std::time::Duration::from_millis(100));
    let shared = shared.lock().unwrap();
    // 降级路径确实回退到 shell-request。
    assert_eq!(shared.shell_request_count, 1);
    // 上传确实发生过，且被兜底清理命令删除。
    assert_eq!(shared.uploaded_rcs.len(), 1);
    assert!(
        shared
            .cleaned_paths
            .iter()
            .any(|path| shared.uploaded_rcs.contains_key(path)),
        "uploaded files must be cleaned up after spawn failure; cleaned={:?}",
        shared.cleaned_paths
    );
    drop(server_thread);
}
