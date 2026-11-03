use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshTerminalOpenResponse {
    pub terminal_id: String,
    /// workspace 会话内的重连世代；AI 可见执行用它拒绝旧授权。
    pub terminal_generation: u64,
    /// bootstrap 成功时注入的 shell（bash/zsh/fish/PowerShell）；None 表示未注入。
    pub integration_shell: Option<String>,
    /// bootstrap 失败的降级原因；成功或未尝试时为 None。
    pub integration_fallback_reason: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshTerminalOutput {
    pub terminal_id: String,
    pub connection_id: i64,
    pub data: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshTerminalExit {
    pub terminal_id: String,
    pub connection_id: i64,
    pub reason: &'static str,
}
