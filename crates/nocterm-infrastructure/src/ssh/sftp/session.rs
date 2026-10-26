//! SFTP 会话池与建连：按 `connection_id` 汇聚 russh 连接，复用同一套认证与
//! 主机密钥策略；并发建连以先入者为准，断开类错误触发池剔除以便下次重连。

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use nocterm_domain::connection::ConnectionProfile;
use russh::client::Handle;
use russh_sftp::client::SftpSession;
use std::time::Duration;

use super::error::SftpError;
use crate::ssh::connection::{Auth, ClientHandler, build_auth, connect_authenticated_coded};

/// 单个 SFTP 请求的等待上限，替换 russh-sftp 的 10 秒默认值。
pub(super) const SFTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// 建连所需的运行期参数，从连接资料与已解出的凭据装配后移动进异步任务。
pub(super) struct ConnectParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: Auth,
}

/// 池中的单个 SFTP 会话：持有 russh 连接句柄以维持底层连接，附带其上的 SFTP 会话。
pub(super) struct SftpConnection {
    /// 保活连接句柄；一旦释放，russh 会断开连接并关闭 SFTP 通道。
    _handle: Handle<ClientHandler>,
    pub sftp: SftpSession,
}

/// 会话池类型别名：按连接标识汇聚，供多个操作以 `Arc` 并发共享同一 SFTP 会话。
pub(super) type SessionPool = Arc<Mutex<HashMap<i64, Arc<SftpConnection>>>>;

/// 依据连接资料与凭据装配建连参数；缺失必要凭据时给出明确提示。
pub(super) fn connect_params(
    profile: &ConnectionProfile,
    password: Option<&str>,
    private_key: Option<&str>,
) -> Result<ConnectParams, SftpError> {
    let auth = build_auth(profile, password, private_key)
        .map_err(|error| SftpError::new("authFailed", error.to_string()))?;
    Ok(ConnectParams {
        host: profile.host.clone(),
        port: profile.port,
        username: profile.username.clone(),
        auth,
    })
}

/// 从会话池取出已有连接；不存在则新建并缓存后返回，供并发操作以 `Arc` 共享。
pub(super) async fn ensure_session(
    sessions: &SessionPool,
    connection_id: i64,
    params: ConnectParams,
) -> Result<Arc<SftpConnection>, SftpError> {
    // 已有会话直接复用：命中即返回，避免重复建连与主机密钥再次校验。
    if let Some(existing) = sessions
        .lock()
        .ok()
        .and_then(|pool| pool.get(&connection_id).cloned())
    {
        return Ok(existing);
    }
    // 未命中则实际建连；建连在锁外进行，避免长时间持锁阻塞其它连接的操作。
    let connection = Arc::new(establish(params).await?);
    if let Ok(mut pool) = sessions.lock() {
        // 双重检查：并发建连时以先入者为准，丢弃本次多建的连接。
        let entry = pool
            .entry(connection_id)
            .or_insert_with(|| connection.clone());
        return Ok(entry.clone());
    }
    Ok(connection)
}

/// 实际建连：认证后打开会话通道、请求 `sftp` 子系统，并在其上初始化 SFTP 会话。
async fn establish(params: ConnectParams) -> Result<SftpConnection, SftpError> {
    let handle =
        connect_authenticated_coded(&params.host, params.port, &params.username, params.auth)
            .await
            .map_err(|failure| SftpError::new(failure.code, failure.message))?;
    // 打开一个会话通道并请求 sftp 子系统；want_reply=true 让服务端确认子系统可用。
    let channel = handle
        .channel_open_session()
        .await
        .map_err(|error| SftpError::lost(format!("打开 SFTP 通道失败：{error}")))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|error| SftpError::lost(format!("请求 sftp 子系统失败：{error}")))?;
    // 将通道转为字节流交给 russh-sftp 完成协议初始化，得到可用的 SFTP 会话。
    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(|error| SftpError::lost(format!("初始化 SFTP 会话失败：{error}")))?;
    // russh-sftp 默认每个请求只等 10 秒。传输是按块逐个发请求的，在跨境链路或
    // 家用上行带宽下，单块耗时超过 10 秒并不罕见——默认值会把一次本来能成功的
    // 传输判成超时。放宽到 60 秒仍能在服务端真的失联时收口。
    sftp.set_timeout(SFTP_REQUEST_TIMEOUT.as_secs());
    Ok(SftpConnection {
        _handle: handle,
        sftp,
    })
}

/// 从会话池剔除并释放某连接；释放句柄即断开底层连接，下次操作会自动重连。
pub(super) fn evict_session(sessions: &SessionPool, connection_id: i64) {
    if let Ok(mut pool) = sessions.lock() {
        pool.remove(&connection_id);
    }
}
