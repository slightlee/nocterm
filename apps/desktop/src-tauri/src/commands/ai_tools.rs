//! AI 可自动执行的结构化服务器检查能力。
//! 模型只能选择工具和受约束参数，实际 Shell 命令始终由 Nocterm 生成。

mod definitions;
mod inspection;

pub(crate) use definitions::server_tool_definitions;
pub(crate) use inspection::build_server_inspection;

/// 后端生成的固定检查计划；`command` 从不包含未经校验的自由文本。
pub(crate) struct ServerInspection {
    pub operation: &'static str,
    pub command: String,
}

const SERVER_TOOL_NAMES: &[&str] = &[
    "get_system_info",
    "list_processes",
    "list_listening_ports",
    "get_service_status",
    "read_service_logs",
    "get_disk_usage",
    "get_memory_usage",
    "list_docker_containers",
    "get_docker_info",
    "get_docker_container_status",
    "read_docker_logs",
];

pub(crate) fn is_server_tool(name: &str) -> bool {
    SERVER_TOOL_NAMES.contains(&name)
}
