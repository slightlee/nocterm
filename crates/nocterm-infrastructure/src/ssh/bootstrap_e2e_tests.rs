//! SSH bootstrap 全链路集成测试：进程内 russh 内存服务器 + 真实
//! `SshTerminalManager` 连接，自主验证探针、临时 rc 上传与 source 注入，
//! 不依赖任何外部服务器或人工操作（真实环境验证发现的回归以此防复发）。
//!
//! 桩服务器模拟一个 bash 登录环境：
//! - exec 探针命令 → 返回版本串 `5.2.1|0.0||`
//! - exec `umask 077 && cat > <path>` → 收集 stdin 为 rc 内容（断言用）
//! - pty+shell → 记录客户端敲入，并在收到 `source` 后模拟 bash 发出
//!   nonce 匹配的 `633;D;0;<nonce>` / `633;A;<nonce>`
//!
//! 断言覆盖真实环境发现过的全部回归：
//! 1. rc 内容含 nonce 导出与钩子、**不含 rm**（删除责任在注入命令）
//! 2. rc 内容**不含**用户 rc 重建（登录链由 request_shell 原样保留）
//! 3. shell channel 上收到 ` source '<path>'; rm -f -- '<path>'` 注入
//!    （此前该敲入代码在历次修改中丢失，集成从未生效）

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
    /// 上传的 rc：远端路径 → 内容。
    uploaded_rcs: HashMap<String, String>,
    /// 可见 shell 通道收到的敲入（按到达顺序）。
    shell_inputs: Vec<String>,
    /// 已建立 pty+shell 的通道。
    shell_channels: HashSet<russh::ChannelId>,
}

/// 一次 exec 的处置结果，由 Handler 主体写入共享状态。
#[derive(Clone)]
enum ExecKind {
    Probe,
    Upload { path: String },
    Other,
}

struct TestServer {
    shared: Arc<Mutex<SharedState>>,
}

impl russh::server::Server for TestServer {
    type Handler = TestHandler;
    fn new_client(&mut self, _peer: Option<std::net::SocketAddr>) -> Self::Handler {
        TestHandler {
            shared: Arc::clone(&self.shared),
            uploads: Mutex::new(HashMap::new()),
        }
    }
}

struct TestHandler {
    shared: Arc<Mutex<SharedState>>,
    /// exec 通道 → 处置类型；数据通道据此分派。
    uploads: Mutex<HashMap<russh::ChannelId, ExecKind>>,
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
        let kind = if command.contains("$BASH_VERSION") {
            let mut shared = self.shared.lock().unwrap();
            shared.probe_count += 1;
            ExecKind::Probe
        } else if let Some(path) = command.strip_prefix("umask 077 && cat > ") {
            ExecKind::Upload {
                path: path.trim_matches('\'').to_string(),
            }
        } else {
            ExecKind::Other
        };
        match &kind {
            // 探针：回版本串，EOF + 关闭。
            ExecKind::Probe => {
                session.data(channel, b"5.2.1|0.0||\n".to_vec())?;
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
            ExecKind::Other => {
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
            // 上传通道的 stdin：rc 内容提交。
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
            // 可见 shell 通道的敲入：记录并模拟 bash 渲染提示符时的钩子输出。
            _ => {
                let nonce = {
                    let shared = self.shared.lock().unwrap();
                    shared
                        .uploaded_rcs
                        .values()
                        .find_map(|rc| extract_nonce_from_rc(rc))
                };
                self.shared.lock().unwrap().shell_inputs.push(text.clone());
                if let Some(nonce) = nonce.filter(|_| text.contains("source")) {
                    session.data(
                        channel,
                        format!("\r\n\x1b]633;D;0;{nonce}\x07\x1b]633;A;{nonce}\x07").into_bytes(),
                    )?;
                }
            }
        }
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        // 上传通道 EOF：rc 内容收集完毕（Nocterm 侧 upload_rc 等待通道关闭）。
        let kind = self.uploads.lock().unwrap().get(&channel).cloned();
        if matches!(kind, Some(ExecKind::Upload { .. })) {
            // 内容已在 data 事件中提交；此处仅兜底终结通道。
            session.close(channel)?;
        }
        Ok(())
    }
}

/// 从 rc 内容首行提取 nonce（`export NOCTERM_OSC_NONCE='<hex>'`）。
fn extract_nonce_from_rc(rc: &str) -> Option<String> {
    let line = rc.lines().next()?;
    let value = line.strip_prefix("export NOCTERM_OSC_NONCE='")?;
    let value = value.strip_suffix('\'')?;
    Some(value.to_string())
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

#[test]
fn bootstrap_full_flow_over_in_memory_ssh_server() {
    let shared = Arc::new(Mutex::new(SharedState::default()));

    // 桩服务器：独立线程 + 独立 runtime（run_on_socket 借用 listener 与
    // &mut self，线程内存活至测试进程退出）。
    let (port_tx, port_rx) = std::sync::mpsc::channel();
    let server_shared = Arc::clone(&shared);
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
                shared: server_shared,
            };
            server
                .run_on_socket(config, &listener)
                .await
                .expect("server run");
        });
    });

    // 桩就绪后，真实管理器连入（open 同步阻塞，内部自带 runtime 与建连兜底）。
    let port = port_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("server port");
    let manager = SshTerminalManager::default();
    let profile = test_profile(port);
    let opened = manager
        .open(&profile, 80, 24, Some("test"), None)
        .expect("open must succeed");

    // bootstrap 结论：集成计划为 bash（探针 + 上传全通过）。
    let plan = opened
        .integration
        .shell_integration
        .expect("bootstrap must produce an integration plan");
    assert_eq!(plan.shell, SshShellKind::Bash);

    // 等待 run_session 的 source 注入到达桩。
    std::thread::sleep(std::time::Duration::from_millis(500));

    let shared = shared.lock().unwrap();
    // 探针被调用。
    assert_eq!(shared.probe_count, 1);

    // rc 内容契约（真实环境回归）：
    // 1) nonce 已导出；2) 钩子已装；3) 不含 rm（删除责任在注入命令）；
    // 4) 不重建用户 rc（登录链由 request_shell 保留）。
    let (path, rc) = shared
        .uploaded_rcs
        .iter()
        .next()
        .expect("rc must be uploaded");
    assert!(path.contains("/tmp/nocterm-ssh-"));
    assert!(
        rc.contains(&format!("export NOCTERM_OSC_NONCE='{}'", plan.nonce)),
        "rc must export the nonce"
    );
    assert!(
        rc.contains("PROMPT_COMMAND="),
        "bash hooks must be installed"
    );
    assert!(!rc.contains("rm -f"), "rc must not carry the delete itself");
    assert!(!rc.contains(". \"$HOME/.bashrc\""));

    // source 注入敲入契约（真实环境回归：该敲入曾整体丢失）：
    let expected_path = super::bootstrap::remote_rc_path(&plan.nonce);
    assert!(
        shared.shell_inputs.iter().any(|input| {
            input.contains(&format!("source '{}'", expected_path))
                && input.contains(&format!("rm -f -- '{}'", expected_path))
        }),
        "run_session must type the source+rm command into the visible shell; inputs: {:?}",
        shared.shell_inputs
    );
    assert_eq!(plan.nonce, extract_nonce_from_rc(rc).expect("nonce in rc"));
    drop(server_thread);
}
