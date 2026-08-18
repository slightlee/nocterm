use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshTerminalOpenResponse {
    pub terminal_id: String,
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
