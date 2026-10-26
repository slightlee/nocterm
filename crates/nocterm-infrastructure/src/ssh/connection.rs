//! SSH 连接与认证原语：主机密钥 TOFU 校验、分阶段建连（解析 → 拨号 → 握手 →
//! 认证）及其错误分类，供终端会话与 SFTP 子模块共用同一套建连与安全策略。
//!
//! 该模块是 `ssh` 模块树的依赖叶子：不反向依赖终端会话或 SFTP 实现。

use std::{io, net::SocketAddr, time::Duration};

use russh::{
    client::{self, Config, Handle, KeyboardInteractiveAuthResponse},
    keys::{
        PrivateKeyWithHashAlg, PublicKeyOrCertificate,
        agent::{AgentIdentity, client::AgentClient},
    },
};
use tokio::{net::TcpStream, time::timeout};

use nocterm_domain::connection::{AuthenticationMethod, ConnectionProfile};

use super::SshError;

/// 主机名解析的上限。`lookup_host` 走系统解析器，DNS 不可达时它自己的重试可能长达
/// 十几秒；解析和拨号是两类故障（写错域名 vs 安全组没放通），预算必须分开计。
pub(super) const DNS_RESOLVE_TIMEOUT: Duration = Duration::from_secs(10);
/// TCP 拨号的整体预算。取 15 秒有两层考虑：与 OpenSSH `ConnectTimeout` 的常用取值一致，
/// 且短于 Windows 默认的 SYN 重试时长（约 21 秒）——否则"端口被防火墙静默丢包"会先由
/// 操作系统报错，我们拿到的只是一个笼统的 io 错误，无法指明卡在拨号阶段。
pub(super) const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// SSH 握手（版本交换、算法协商、密钥交换、主机密钥校验）的上限。
/// 正常在一两个 RTT 内完成；超时几乎只出现在对端不是 SSH 服务或中间设备劫持了端口。
pub(super) const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);
/// 认证阶段的上限。留得比握手宽，是因为服务端可能在此调用 PAM/LDAP 等外部依赖。
pub(super) const AUTH_TIMEOUT: Duration = Duration::from_secs(30);

/// 主机密钥校验期间可能产生的领域级失败，供 russh 的 Handler 向上传播。
// 随 `ClientHandler` 一并暴露到 ssh 模块树可见性：它是该 Handler 关联类型 `Error`，
// 否则会触发 E0446“private type in public interface”。
#[derive(Debug)]
pub(super) enum HandlerError {
    /// russh 协议层错误；保留源错误用于向用户展示握手失败原因。
    Ssh(russh::Error),
    /// 记录在案的主机密钥与本次不一致，疑似中间人攻击，必须拒绝。
    HostKeyChanged,
    /// known_hosts 存在但无法据此得出结论（条目格式损坏、文件不可读等）。
    /// 这种"校验没跑成"的情况必须拒绝而不是放行：放行等于在无法判断的前提下
    /// 默认信任对端，正是 TOFU 要防的场景。附带诊断原文供用户修复该文件。
    HostKeyUnverifiable(String),
}

impl From<russh::Error> for HandlerError {
    fn from(source: russh::Error) -> Self {
        Self::Ssh(source)
    }
}

/// russh 客户端回调：目前仅覆盖主机密钥校验，其余沿用默认实现。
// 可见性与返回它的 `connect_authenticated*` 保持一致（pub(super)），
// 消除 private_interfaces 警告——该类型本就随建连句柄在 ssh 模块树内共享。
pub(super) struct ClientHandler {
    host: String,
    port: u16,
}

impl client::Handler for ClientHandler {
    type Error = HandlerError;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        // 证书型主机密钥（CA 签发）暂不在支持范围内，保守拒绝而非放行。
        let key = match server_public_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key,
            PublicKeyOrCertificate::Certificate(_) => return Ok(false),
        };
        match russh::keys::known_hosts::check_known_hosts(&self.host, self.port, key) {
            // 已记录且一致：正常放行。
            Ok(true) => Ok(true),
            // 首次遇到该主机：记录后放行，等价 OpenSSH 的 accept-new 策略。
            Ok(false) => {
                // 写入失败不阻断连接（与 OpenSSH 写不进 known_hosts 时仅告警一致），
                // 但必须留下痕迹：若长期写不进去，"变更即拒"就永远不会生效——
                // 每次连接都被当成首次，静默退化成不做校验。
                if let Err(error) =
                    russh::keys::known_hosts::learn_known_hosts(&self.host, self.port, key)
                {
                    eprintln!(
                        "warning: failed to record host key for {}:{} in known_hosts: {error}",
                        self.host, self.port
                    );
                }
                Ok(true)
            }
            // 密钥发生变更：坚决拒绝，交由上层给出安全提示。
            Err(russh::keys::Error::KeyChanged { .. }) => Err(HandlerError::HostKeyChanged),
            // 找不到家目录时无处可查也无处可记，按首次信任放行但不落盘；
            // 这是环境缺失而非校验结论异常，拒绝会让无 HOME 的环境彻底连不上。
            Err(russh::keys::Error::NoHomeDir) => Ok(true),
            // 其余错误意味着 known_hosts 存在却无法据此判断（例如该主机的某行条目
            // 格式损坏会让整次校验返回 Err）。此时既不能断言一致也不能断言首次，
            // 一律拒绝并把原文交给用户，而不是默默放行。
            Err(error) => Err(HandlerError::HostKeyUnverifiable(error.to_string())),
        }
    }
}

/// 认证方式的运行期载荷，携带进入会话任务所需的所有权数据。
pub(super) enum Auth {
    Password(String),
    PrivateKey(String),
    Agent,
}

/// 按连接资料与已解出的凭据装配认证载荷；缺失必要凭据时给出明确提示。
/// 终端与 SFTP 共用同一套凭据校验，避免两处出现不一致的登录前置条件。
///
/// 口令缺失在这里只是最后一道防线：命令层会先返回 `SSH_PASSWORD_REQUIRED` 让终端
/// 就地提示输入，所以提示语不再要求用户"先去保存密码"。
pub(super) fn build_auth(
    profile: &ConnectionProfile,
    password: Option<&str>,
    private_key: Option<&str>,
) -> Result<Auth, SshError> {
    match profile.authentication {
        AuthenticationMethod::Password => password
            .map(|secret| Auth::Password(secret.to_string()))
            .ok_or_else(|| {
                SshError("缺少用于登录的密码，请在终端提示符处输入或为该连接保存密码".to_string())
            }),
        AuthenticationMethod::PrivateKey => private_key
            .map(|key| Auth::PrivateKey(key.to_string()))
            .ok_or_else(|| SshError("缺少用于登录的 SSH 私钥".to_string())),
        AuthenticationMethod::SshAgent => Ok(Auth::Agent),
    }
}

/// 建立 TCP 连接并完成认证，返回已认证的连接句柄。
/// 终端在此基础上申请 PTY/Shell，SFTP 在此基础上请求 sftp 子系统。
pub(super) async fn connect_authenticated(
    host: &str,
    port: u16,
    username: &str,
    auth: Auth,
) -> Result<Handle<ClientHandler>, String> {
    connect_authenticated_coded(host, port, username, auth)
        .await
        .map_err(|failure| failure.message)
}

/// 建连失败的结构化描述：`code` 为前端稳定错误码，`message` 为面向用户的中文提示。
/// SFTP 依赖 `code` 归一化到既有的 `__NOCTERM_SFTP_ERROR__` 契约，终端只取用 `message`。
pub(super) struct ConnectFailure {
    pub(super) code: &'static str,
    pub(super) message: String,
}

/// 与 `connect_authenticated` 相同，但保留可分类的错误码，供 SFTP 归一化到前端契约。
///
/// 建连被拆成"解析 → 拨号 → 握手 → 认证"四步并各自计时。原实现把全部四步交给
/// `client::connect` 并只在最外层设一个 30 秒总超时，任何一步卡住都只报"连接远程主机超时"，
/// 用户无从判断该改端口、开安全组还是换凭据。分阶段后每种失败都能指名道姓。
pub(super) async fn connect_authenticated_coded(
    host: &str,
    port: u16,
    username: &str,
    auth: Auth,
) -> Result<Handle<ClientHandler>, ConnectFailure> {
    // 保活参数与旧 OpenSSH 实现保持一致，避免 NAT/防火墙静默断连。
    let config = Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        ..Default::default()
    };
    let handler = ClientHandler {
        host: host.to_string(),
        port,
    };
    // 自己完成 TCP 拨号，再把已连通的流交给 russh，握手才能独立计时。
    let stream = dial(host, port).await?;
    let handshake = timeout(
        HANDSHAKE_TIMEOUT,
        client::connect_stream(std::sync::Arc::new(config), stream, handler),
    );
    let mut handle = match handshake.await {
        Ok(Ok(handle)) => handle,
        Ok(Err(HandlerError::HostKeyChanged)) => {
            return Err(ConnectFailure {
                code: "hostKeyFailed",
                message: "远程主机密钥已变更，可能存在中间人攻击风险，已拒绝连接".to_string(),
            });
        }
        Ok(Err(HandlerError::HostKeyUnverifiable(reason))) => {
            return Err(ConnectFailure {
                code: "hostKeyFailed",
                message: format!(
                    "无法校验 {host}:{port} 的主机密钥（{reason}）：已拒绝连接。请检查 known_hosts 文件中该主机的条目是否损坏或文件是否可读"
                ),
            });
        }
        Ok(Err(HandlerError::Ssh(error))) => {
            return Err(classify_handshake_error(host, port, error));
        }
        // TCP 已通但对端不按 SSH 协议应答：典型是端口上跑的不是 sshd，或被中间设备劫持。
        Err(_) => {
            return Err(ConnectFailure {
                code: "timeout",
                message: format!(
                    "{host}:{port} 的 SSH 握手在 {} 秒内没有完成：端口已连通但对端未按 SSH 协议应答，请确认该端口上运行的确实是 SSH 服务",
                    HANDSHAKE_TIMEOUT.as_secs()
                ),
            });
        }
    };

    let attempt = async {
        match auth {
            Auth::Password(password) => {
                authenticate_password(&mut handle, username, &password).await
            }
            Auth::PrivateKey(key) => authenticate_private_key(&mut handle, username, &key).await,
            Auth::Agent => authenticate_agent(&mut handle, username).await,
        }
    };
    let authenticated = match timeout(AUTH_TIMEOUT, attempt).await {
        Ok(result) => result,
        Err(_) => {
            return Err(ConnectFailure {
                code: "timeout",
                message: format!(
                    "{host}:{port} 在 {} 秒内没有返回认证结果：服务端可能正等待其它认证方式或后端目录服务无响应",
                    AUTH_TIMEOUT.as_secs()
                ),
            });
        }
    };
    // 认证阶段无论返回错误还是失败，对用户而言都是“认证未通过”，统一归为 authFailed。
    match authenticated {
        Ok(true) => Ok(handle),
        Ok(false) => Err(ConnectFailure {
            code: "authFailed",
            message: "认证失败：请检查用户名、密码或密钥后重试".to_string(),
        }),
        Err(message) => Err(ConnectFailure {
            code: "authFailed",
            message,
        }),
    }
}

/// 解析主机名并在给定预算内逐个地址拨号，返回首个连通的 TCP 流。
///
/// 不直接用 `client::connect`（它内部 `TcpStream::connect(addrs)` 无超时），
/// 也不把 `&str` 丢给 `TcpStream::connect`：那样 DNS 失败与拨号失败会混成同一个
/// `io::Error`，而 Windows 上 getaddrinfo 的失败并不映射到 `ErrorKind::NotFound`，
/// 按 kind 分类会全部落进兜底分支。显式分两步才能给出准确提示。
async fn dial(host: &str, port: u16) -> Result<TcpStream, ConnectFailure> {
    let addresses = resolve(host, port).await?;
    let deadline = tokio::time::Instant::now() + TCP_CONNECT_TIMEOUT;
    let mut last_error: Option<io::Error> = None;
    let mut exhausted = false;

    for address in addresses {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            exhausted = true;
            break;
        }
        match timeout(remaining, TcpStream::connect(address)).await {
            Ok(Ok(stream)) => return Ok(stream),
            // 单个地址失败就继续下一个：双栈主机常同时解析出 IPv6 与 IPv4，
            // 只有 IPv6 不通时若就此放弃，可用的 IPv4 地址就被浪费了。
            Ok(Err(error)) => last_error = Some(error),
            Err(_) => {
                exhausted = true;
                break;
            }
        }
    }

    // 预算用尽时以超时为准：某个地址此前快速失败（如 IPv6 拒绝）不能掩盖真正的卡点。
    Err(match last_error {
        Some(error) if !exhausted => classify_dial_error(host, port, &error),
        _ => ConnectFailure {
            code: "timeout",
            message: format!(
                "连接 {host}:{port} 超时：{} 秒内没能建立 TCP 连接。请确认服务器已开机、SSH 端口正确，以及云安全组/防火墙放通了本机出口 IP",
                TCP_CONNECT_TIMEOUT.as_secs()
            ),
        },
    })
}

/// 主机名解析单独成步：把"地址写错、DNS 不可用"与"能解析但连不上"彻底分开。
///
/// 解析必须自带超时。`lookup_host` 在 DNS 服务器不可达时会一路等到系统解析器放弃
/// （Windows 上可达十几秒甚至更久），叠加后续阶段就可能顶穿外层 `CONNECT_READY_TIMEOUT`，
/// 让那句笼统的"无响应"重新盖掉分阶段提示——分阶段计时就白做了。
async fn resolve(host: &str, port: u16) -> Result<Vec<SocketAddr>, ConnectFailure> {
    let lookup = match timeout(DNS_RESOLVE_TIMEOUT, tokio::net::lookup_host((host, port))).await {
        Ok(result) => result,
        Err(_) => {
            return Err(ConnectFailure {
                code: "hostResolveFailed",
                message: format!(
                    "解析主机名 {host} 超过 {} 秒仍未返回：请检查本机 DNS 设置与网络连通性",
                    DNS_RESOLVE_TIMEOUT.as_secs()
                ),
            });
        }
    };
    let addresses = lookup.map(|resolved| resolved.collect());
    finish_resolution(host, addresses)
}

/// 将系统解析器结果收口成稳定错误码；独立于真实 DNS，便于跨平台确定性测试。
fn finish_resolution(
    host: &str,
    lookup: io::Result<Vec<SocketAddr>>,
) -> Result<Vec<SocketAddr>, ConnectFailure> {
    let addresses = lookup.map_err(|error| ConnectFailure {
        code: "hostResolveFailed",
        message: format!("无法解析主机名 {host}：{error}"),
    })?;
    if addresses.is_empty() {
        return Err(ConnectFailure {
            code: "hostResolveFailed",
            message: format!("主机名 {host} 没有解析到任何 IP 地址"),
        });
    }
    Ok(addresses)
}

/// TCP 拨号失败的分类。错误码沿用既有集合（SFTP 前端按码取固定文案，不能新增），
/// `message` 则补上 `host:port` 与可执行的排查方向，供终端直接展示。
fn classify_dial_error(host: &str, port: u16, error: &io::Error) -> ConnectFailure {
    let (code, message): (&'static str, String) = match error.kind() {
        io::ErrorKind::ConnectionRefused => (
            "connectionRefused",
            format!(
                "{host}:{port} 拒绝了连接：端口可达但没有服务在监听，请确认 sshd 已启动且端口号正确"
            ),
        ),
        io::ErrorKind::TimedOut => (
            "timeout",
            format!(
                "连接 {host}:{port} 超时：请确认服务器已开机、SSH 端口正确，以及云安全组/防火墙放通了本机出口 IP"
            ),
        ),
        io::ErrorKind::HostUnreachable | io::ErrorKind::NetworkUnreachable => (
            "hostUnreachable",
            format!("无法路由到 {host}:{port}：请检查本机网络与目标网段之间的可达性"),
        ),
        // 归类不到的网络错误保留底层原文；错误码取 hostUnreachable 而非
        // connectionRefused——“拒绝”是个明确结论，不该用来兜底未知故障。
        other => (
            "hostUnreachable",
            format!("无法建立到 {host}:{port} 的 TCP 连接（{other:?}）：{error}"),
        ),
    };
    ConnectFailure { code, message }
}

/// 把握手阶段的 russh 错误映射到前端既有错误码。
/// 此时 TCP 已连通，因此不再区分网络可达性，只关心协议层结论：
/// 算法协商失败、协议版本不符、服务端主动断开都落在兜底分支并附带原文——
/// 丢掉原文就等于放弃定位能力。russh 的错误只描述协议状态，不含口令或密钥材料。
fn classify_handshake_error(host: &str, port: u16, error: russh::Error) -> ConnectFailure {
    match &error {
        russh::Error::IO(io) => ConnectFailure {
            code: "hostUnreachable",
            message: format!("与 {host}:{port} 的 SSH 握手中断：{io}"),
        },
        other => ConnectFailure {
            code: "unknown",
            message: format!("与 {host}:{port} 的 SSH 握手失败：{other}"),
        },
    }
}

/// 密码认证；服务端仅开放 keyboard-interactive 时用同一口令回退尝试。
///
/// 各认证分支的 `Err` 表示"请求没能完成"（连接被断开、方法不被支持等），
/// 与"口令不对"（`Ok(false)`）是两件事，因此一律带上底层原文：把两者都压成
/// 一句"认证失败"会让用户对着正确的口令反复重试。russh 的错误只描述协议状态。
async fn authenticate_password(
    handle: &mut Handle<ClientHandler>,
    username: &str,
    password: &str,
) -> Result<bool, String> {
    let result = handle
        .authenticate_password(username, password)
        .await
        .map_err(|error| format!("密码认证请求失败：{error}"))?;
    if result.success() {
        return Ok(true);
    }
    keyboard_interactive_authenticate(handle, username, password).await
}

/// 键盘交互认证：对每个提示回填同一登录口令，覆盖仅支持该方式的服务器。
async fn keyboard_interactive_authenticate(
    handle: &mut Handle<ClientHandler>,
    username: &str,
    password: &str,
) -> Result<bool, String> {
    let mut response = handle
        .authenticate_keyboard_interactive_start(username, None)
        .await
        .map_err(|error| format!("键盘交互认证请求失败：{error}"))?;
    loop {
        match response {
            KeyboardInteractiveAuthResponse::Success => return Ok(true),
            KeyboardInteractiveAuthResponse::Failure { .. } => return Ok(false),
            KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                let answers = prompts.iter().map(|_| password.to_string()).collect();
                response = handle
                    .authenticate_keyboard_interactive_respond(answers)
                    .await
                    .map_err(|error| format!("键盘交互认证请求失败：{error}"))?;
            }
        }
    }
}

/// 私钥认证：解析 OpenSSH/PEM 私钥，RSA 密钥自动选用服务端支持的最佳哈希。
async fn authenticate_private_key(
    handle: &mut Handle<ClientHandler>,
    username: &str,
    private_key: &str,
) -> Result<bool, String> {
    let key = russh::keys::decode_secret_key(private_key, None).map_err(|error| match error {
        russh::keys::Error::KeyIsEncrypted => {
            "私钥已加密，暂不支持带密码短语的私钥登录".to_string()
        }
        // 保留解析原文（"格式不支持""内容损坏"处置方式完全不同）；
        // russh 的密钥错误只描述格式问题，不包含密钥材料。
        other => format!("无法解析 SSH 私钥：{other}"),
    })?;
    let key = std::sync::Arc::new(key);
    let hash = best_rsa_hash(handle, key.algorithm().is_rsa()).await;
    let result = handle
        .authenticate_publickey(username, PrivateKeyWithHashAlg::new(key, hash))
        .await
        .map_err(|error| format!("私钥认证请求失败：{error}"))?;
    Ok(result.success())
}

/// ssh-agent 认证：Unix 走 SSH_AUTH_SOCK，Windows 走 OpenSSH 命名管道。
async fn authenticate_agent(
    handle: &mut Handle<ClientHandler>,
    username: &str,
) -> Result<bool, String> {
    #[cfg(unix)]
    {
        let mut agent = AgentClient::connect_env()
            .await
            .map_err(|error| format!("连接 SSH Agent 失败（SSH_AUTH_SOCK）：{error}"))?;
        agent_authenticate(handle, username, &mut agent).await
    }
    #[cfg(windows)]
    {
        let mut agent = AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent")
            .await
            .map_err(|error| {
                format!(
                    "连接 SSH Agent 失败（请确认 OpenSSH Authentication Agent 服务已启动）：{error}"
                )
            })?;
        agent_authenticate(handle, username, &mut agent).await
    }
}

/// 遍历 agent 中的公钥身份逐一尝试认证，任一成功即返回。
async fn agent_authenticate<R>(
    handle: &mut Handle<ClientHandler>,
    username: &str,
    agent: &mut AgentClient<R>,
) -> Result<bool, String>
where
    R: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
{
    let identities = agent
        .request_identities()
        .await
        .map_err(|error| format!("读取 SSH Agent 身份列表失败：{error}"))?;
    for identity in identities {
        // 仅处理裸公钥身份；证书身份不在当前支持范围。
        let key = match &identity {
            AgentIdentity::PublicKey { key, .. } => key.clone(),
            AgentIdentity::Certificate { .. } => continue,
        };
        let hash = best_rsa_hash(handle, key.algorithm().is_rsa()).await;
        if let Ok(result) = handle
            .authenticate_publickey_with(username, key, hash, agent)
            .await
            && result.success()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// 仅对 RSA 密钥查询服务端支持的最佳签名哈希（rsa-sha2-512/256），其余返回 None。
async fn best_rsa_hash(
    handle: &Handle<ClientHandler>,
    is_rsa: bool,
) -> Option<russh::keys::HashAlg> {
    if !is_rsa {
        return None;
    }
    handle
        .best_supported_rsa_hash()
        .await
        .ok()
        .flatten()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_auth_requires_a_secret() {
        let result = build_auth(&profile(AuthenticationMethod::Password), None, None);
        match result {
            Ok(_) => panic!("missing password must fail"),
            Err(error) => assert_eq!(
                error.to_string(),
                "缺少用于登录的密码，请在终端提示符处输入或为该连接保存密码"
            ),
        }
    }

    #[test]
    fn private_key_auth_requires_key_material() {
        let result = build_auth(&profile(AuthenticationMethod::PrivateKey), None, None);
        match result {
            Ok(_) => panic!("missing key must fail"),
            Err(error) => assert_eq!(error.to_string(), "缺少用于登录的 SSH 私钥"),
        }
    }

    /// 用回环地址覆盖拨号的三种结局，不触碰真实远端，也不写 known_hosts。
    #[tokio::test]
    async fn dial_reaches_a_listening_local_port() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        assert!(dial("127.0.0.1", port).await.is_ok());
    }

    #[tokio::test]
    async fn dial_reports_a_refused_port_as_refused_rather_than_timeout() {
        // 先绑定再释放，拿到一个此刻确定无人监听的端口：
        // 连接会立即被 RST 拒绝，不应被归类成超时。
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        drop(listener);

        let failure = dial("127.0.0.1", port)
            .await
            .expect_err("closed port must fail");
        assert_eq!(failure.code, "connectionRefused");
        // 提示必须点名 host:port，否则用户无法判断是不是端口填错了。
        assert!(failure.message.contains(&format!("127.0.0.1:{port}")));
    }

    #[test]
    fn resolve_reports_lookup_failures_without_external_dns() {
        // 代理或 VPN 可能把 .invalid 映射到 Fake-IP；人工构造错误才能保持跨网络环境确定。
        let lookup = Err(io::Error::new(
            io::ErrorKind::NotFound,
            "fixture lookup failure",
        ));
        let failure = finish_resolution("nocterm-nonexistent.invalid", lookup)
            .expect_err("failed lookup must be classified");
        assert_eq!(failure.code, "hostResolveFailed");
        assert!(failure.message.contains("nocterm-nonexistent.invalid"));
    }

    fn profile(authentication: AuthenticationMethod) -> ConnectionProfile {
        ConnectionProfile {
            id: 1,
            name: "Test".to_string(),
            host: "127.0.0.1".to_string(),
            port: 22,
            username: "tester".to_string(),
            authentication,
            created_at: 0,
            updated_at: 0,
            group_id: None,
            group_name: None,
            remark: None,
            sync_mode: "local_only".to_string(),
            execution_target: "remote_terminal".to_string(),
            remote_initial_path: None,
            icon: None,
            sort_order: None,
            private_key_path: None,
            credential_kind: None,
            credential_status: "missing".to_string(),
        }
    }
}
