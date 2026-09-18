//! Provider 可见的服务器检查工具契约。

use serde_json::{Value, json};

use crate::commands::ai_tool_contract::execution_context_schema;

/// 每个工具都声明封闭的输入 Schema，避免 Provider 猜测额外参数或传入完整命令。
pub(crate) fn server_tool_definitions() -> Vec<Value> {
    vec![
        tool(
            "get_system_info",
            "读取当前服务器的操作系统主机名、内核、架构和运行时间；不返回当前工作目录",
            empty_schema(),
        ),
        tool(
            "list_processes",
            "按 CPU、内存或 PID 查看当前服务器进程；返回数量有硬上限",
            json!({
                "type":"object",
                "properties":{
                    "limit":{"type":"integer","minimum":1,"maximum":200,"default":50},
                    "sortBy":{"type":"string","enum":["cpu","memory","pid"],"default":"cpu"}
                },
                "additionalProperties":false
            }),
        ),
        tool(
            "list_listening_ports",
            "查看当前服务器正在监听的 TCP 和 UDP 端口",
            json!({
                "type":"object",
                "properties":{"includeProcesses":{"type":"boolean","default":true}},
                "additionalProperties":false
            }),
        ),
        tool(
            "get_service_status",
            "读取一个 systemd 服务的活动状态、子状态、PID 和最近状态变化",
            json!({
                "type":"object",
                "properties":{"service":{"type":"string","minLength":1,"maxLength":128}},
                "required":["service"],
                "additionalProperties":false
            }),
        ),
        tool(
            "read_service_logs",
            "读取一个 systemd 服务最近的日志；只允许受限行数、时间范围和级别",
            json!({
                "type":"object",
                "properties":{
                    "service":{"type":"string","minLength":1,"maxLength":128},
                    "lines":{"type":"integer","minimum":1,"maximum":500,"default":100},
                    "sinceMinutes":{"type":"integer","minimum":1,"maximum":10080},
                    "priority":{"type":"string","enum":["error","warning","info","debug"],"default":"info"}
                },
                "required":["service"],
                "additionalProperties":false
            }),
        ),
        tool(
            "get_disk_usage",
            "读取当前服务器各文件系统的容量和使用率",
            empty_schema(),
        ),
        tool(
            "get_memory_usage",
            "读取当前服务器的内存和交换空间使用量",
            empty_schema(),
        ),
        tool(
            "list_docker_containers",
            "读取 Docker 服务版本，并列出容器名称、镜像、状态和端口",
            json!({
                "type":"object",
                "properties":{"includeStopped":{"type":"boolean","default":false}},
                "additionalProperties":false
            }),
        ),
        tool(
            "get_docker_info",
            "读取 Docker 守护进程、存储驱动、资源和镜像加速器配置，不返回容器环境变量",
            empty_schema(),
        ),
        tool(
            "get_docker_container_status",
            "读取一个 Docker 容器的运行、退出、重启和健康状态，不返回环境变量",
            json!({
                "type":"object",
                "properties":{"container":{"type":"string","minLength":1,"maxLength":128}},
                "required":["container"],
                "additionalProperties":false
            }),
        ),
        tool(
            "read_docker_logs",
            "读取一个 Docker 容器最近的日志；只允许受限行数和时间范围",
            json!({
                "type":"object",
                "properties":{
                    "container":{"type":"string","minLength":1,"maxLength":128},
                    "lines":{"type":"integer","minimum":1,"maximum":500,"default":100},
                    "sinceMinutes":{"type":"integer","minimum":1,"maximum":10080}
                },
                "required":["container"],
                "additionalProperties":false
            }),
        ),
    ]
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    let execution_schema = execution_context_schema();
    json!({
        "name":name,
        "description":description,
        "inputSchema":input_schema,
        "outputSchema":{
            "type":"object",
            "properties":{
                "operation":{"type":"string"},
                "output":{"type":"string"},
                "succeeded":{"type":"boolean"},
                "execution":execution_schema
            },
            "required":["operation","output","succeeded","execution"],
            "additionalProperties":false
        },
        "annotations":{
            "readOnlyHint":true,
            "destructiveHint":false,
            "idempotentHint":true,
            "openWorldHint":false
        }
    })
}

fn empty_schema() -> Value {
    json!({"type":"object","properties":{},"additionalProperties":false})
}

#[cfg(test)]
mod tests {
    use super::server_tool_definitions;

    #[test]
    fn definitions_expose_closed_schemas_and_structured_outputs() {
        let tools = server_tool_definitions();
        assert_eq!(tools.len(), 11);
        assert!(
            tools
                .iter()
                .all(|tool| tool["inputSchema"]["additionalProperties"] == false)
        );
        assert!(
            tools
                .iter()
                .all(|tool| tool["annotations"]["readOnlyHint"] == true)
        );
        assert!(tools.iter().all(|tool| tool.get("outputSchema").is_some()));
        assert!(tools.iter().all(|tool| {
            tool["outputSchema"]["properties"]["execution"]["properties"]
                ["visibleTerminalStateInherited"]["type"]
                == "boolean"
        }));
        assert!(tools[0]["description"].as_str().is_some_and(|value| {
            value.contains("操作系统主机名") && value.contains("不返回当前工作目录")
        }));
    }
}
