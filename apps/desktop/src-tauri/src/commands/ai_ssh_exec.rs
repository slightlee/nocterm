//! 已停用的旧 AI SSH IPC，仅保留明确失败的兼容 ABI。
//! 实际工具调用统一经过结构化 Gateway 或逐次审批入口。

use serde_json::Value;

#[tauri::command]
pub fn ai_ssh_exec_readonly(_request: Value) -> Result<(), String> {
    Err("任意只读命令入口已停用，请使用结构化检查能力或逐次确认执行".to_string())
}

#[tauri::command]
pub fn ai_ssh_exec_stop(_execution_id: String) -> Result<(), String> {
    Err("旧 AI SSH 执行入口已停用".to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::ai_ssh_exec_readonly;

    #[test]
    fn legacy_readonly_abi_never_executes_a_command() {
        for command in [
            "hostname",
            "journalctl --vacuum-time=1s",
            "ss -K dst 127.0.0.1",
        ] {
            let request = json!({"connectionId": 1, "command": command});
            assert!(ai_ssh_exec_readonly(request).is_err());
        }
    }
}
