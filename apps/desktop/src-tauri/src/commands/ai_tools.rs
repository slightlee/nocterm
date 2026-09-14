//! AI 可自动执行的结构化服务器检查能力。
//! 模型只能选择工具和受约束参数，实际 Shell 命令始终由 Nocterm 生成。

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

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

/// 每个工具都声明封闭的输入 Schema，避免 Provider 猜测额外参数或传入完整命令。
pub(crate) fn server_tool_definitions() -> Vec<Value> {
    vec![
        tool(
            "get_system_info",
            "读取当前服务器的主机名、内核、架构和运行时间",
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
    json!({
        "name":name,
        "description":description,
        "inputSchema":input_schema,
        "outputSchema":{
            "type":"object",
            "properties":{
                "operation":{"type":"string"},
                "output":{"type":"string"}
            },
            "required":["operation","output"],
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

pub(crate) fn build_server_inspection(
    name: &str,
    arguments: &Value,
) -> Result<ServerInspection, String> {
    let object = arguments
        .as_object()
        .ok_or_else(|| "工具参数必须是 JSON 对象".to_string())?;
    match name {
        "get_system_info" => {
            reject_unknown(object, &[])?;
            Ok(ServerInspection {
                operation: "system_info",
                // 三段信息属于一个结构化结果，任一步失败都不能被最后一个成功退出码掩盖。
                command: "uname -n && uname -srm && uptime".into(),
            })
        }
        "list_processes" => process_plan(object),
        "list_listening_ports" => port_plan(object),
        "get_service_status" => service_status_plan(object),
        "read_service_logs" => service_logs_plan(object),
        "get_disk_usage" => {
            reject_unknown(object, &[])?;
            Ok(ServerInspection {
                operation: "disk_usage",
                command: "df -P -h".into(),
            })
        }
        "get_memory_usage" => {
            reject_unknown(object, &[])?;
            Ok(ServerInspection {
                operation: "memory_usage",
                command: "free -h".into(),
            })
        }
        "list_docker_containers" => docker_plan(object),
        "get_docker_info" => docker_info_plan(object),
        "get_docker_container_status" => docker_container_status_plan(object),
        "read_docker_logs" => docker_logs_plan(object),
        _ => Err("未知的服务器检查工具".to_string()),
    }
}

fn process_plan(object: &Map<String, Value>) -> Result<ServerInspection, String> {
    reject_unknown(object, &["limit", "sortBy"])?;
    let limit = bounded_integer(object, "limit", 50, 1, 200)?;
    let sort = optional_string(object, "sortBy")?.unwrap_or("cpu");
    let sort_key = match sort {
        "cpu" => "-pcpu",
        "memory" => "-pmem",
        "pid" => "pid",
        _ => return Err("sortBy 只支持 cpu、memory 或 pid".to_string()),
    };
    Ok(ServerInspection {
        operation: "process_list",
        command: format!(
            "ps -eo pid=,ppid=,user=,pcpu=,pmem=,stat=,etime=,comm= --sort={sort_key} | head -n {}",
            limit + 1
        ),
    })
}

fn port_plan(object: &Map<String, Value>) -> Result<ServerInspection, String> {
    reject_unknown(object, &["includeProcesses"])?;
    let include_processes = optional_bool(object, "includeProcesses")?.unwrap_or(true);
    Ok(ServerInspection {
        operation: "listening_ports",
        command: if include_processes {
            "ss -H -lntup".into()
        } else {
            "ss -H -lntu".into()
        },
    })
}

fn service_status_plan(object: &Map<String, Value>) -> Result<ServerInspection, String> {
    reject_unknown(object, &["service"])?;
    let service = service_name(required_string(object, "service")?)?;
    Ok(ServerInspection {
        operation: "service_status",
        command: format!(
            "systemctl show --no-pager --property=Id,LoadState,ActiveState,SubState,MainPID,Result,ExecMainStatus,StateChangeTimestamp {service}"
        ),
    })
}

fn service_logs_plan(object: &Map<String, Value>) -> Result<ServerInspection, String> {
    reject_unknown(object, &["service", "lines", "sinceMinutes", "priority"])?;
    let service = service_name(required_string(object, "service")?)?;
    let lines = bounded_integer(object, "lines", 100, 1, 500)?;
    let priority = optional_string(object, "priority")?.unwrap_or("info");
    let priority_range = match priority {
        "error" => "emerg..err",
        "warning" => "emerg..warning",
        "info" => "emerg..info",
        "debug" => "emerg..debug",
        _ => return Err("priority 只支持 error、warning、info 或 debug".to_string()),
    };
    let since = match object.get("sinceMinutes") {
        Some(_) => format!(
            " --since=-{}min",
            bounded_integer(object, "sinceMinutes", 60, 1, 10_080)?
        ),
        None => String::new(),
    };
    Ok(ServerInspection {
        operation: "service_logs",
        command: format!(
            "journalctl --no-pager --output=short-iso --unit={service} --lines={lines} --priority={priority_range}{since}"
        ),
    })
}

fn docker_plan(object: &Map<String, Value>) -> Result<ServerInspection, String> {
    reject_unknown(object, &["includeStopped"])?;
    let all = if optional_bool(object, "includeStopped")?.unwrap_or(false) {
        " --all"
    } else {
        ""
    };
    Ok(ServerInspection {
        operation: "docker_containers",
        command: format!(
            "docker version --format 'Server={{{{.Server.Version}}}}' && docker ps{all} --format '{{{{.Names}}}}\\t{{{{.Image}}}}\\t{{{{.Status}}}}\\t{{{{.Ports}}}}'"
        ),
    })
}

fn docker_info_plan(object: &Map<String, Value>) -> Result<ServerInspection, String> {
    reject_unknown(object, &[])?;
    Ok(ServerInspection {
        operation: "docker_info",
        command: "docker info --format 'ServerVersion={{.ServerVersion}}\nOperatingSystem={{.OperatingSystem}}\nOSType={{.OSType}}\nArchitecture={{.Architecture}}\nStorageDriver={{.Driver}}\nCgroupDriver={{.CgroupDriver}}\nCPUs={{.NCPU}}\nMemoryBytes={{.MemTotal}}\nDockerRootDir={{.DockerRootDir}}\nRegistryMirrors={{json .RegistryConfig.Mirrors}}'".into(),
    })
}

fn docker_container_status_plan(object: &Map<String, Value>) -> Result<ServerInspection, String> {
    reject_unknown(object, &["container"])?;
    let container = docker_container_name(required_string(object, "container")?)?;
    Ok(ServerInspection {
        operation: "docker_container_status",
        command: format!(
            "docker container inspect --format 'Name={{{{.Name}}}}\nImage={{{{.Config.Image}}}}\nStatus={{{{.State.Status}}}}\nRunning={{{{.State.Running}}}}\nRestarting={{{{.State.Restarting}}}}\nExitCode={{{{.State.ExitCode}}}}\nError={{{{json .State.Error}}}}\nStartedAt={{{{.State.StartedAt}}}}\nFinishedAt={{{{.State.FinishedAt}}}}\nRestartCount={{{{.RestartCount}}}}\nHealth={{{{if .State.Health}}}}{{{{.State.Health.Status}}}}{{{{else}}}}none{{{{end}}}}' -- {container}"
        ),
    })
}

fn docker_logs_plan(object: &Map<String, Value>) -> Result<ServerInspection, String> {
    reject_unknown(object, &["container", "lines", "sinceMinutes"])?;
    let container = docker_container_name(required_string(object, "container")?)?;
    let lines = bounded_integer(object, "lines", 100, 1, 500)?;
    let since = match object.get("sinceMinutes") {
        Some(_) => format!(
            " --since {}m",
            bounded_integer(object, "sinceMinutes", 60, 1, 10_080)?
        ),
        None => String::new(),
    };
    Ok(ServerInspection {
        operation: "docker_logs",
        command: format!("docker logs --timestamps --tail {lines}{since} -- {container}"),
    })
}

fn reject_unknown(object: &Map<String, Value>, allowed: &[&str]) -> Result<(), String> {
    let allowed = allowed.iter().copied().collect::<BTreeSet<_>>();
    if let Some(name) = object.keys().find(|name| !allowed.contains(name.as_str())) {
        return Err(format!("不支持的工具参数：{name}"));
    }
    Ok(())
}

fn required_string<'a>(object: &'a Map<String, Value>, name: &str) -> Result<&'a str, String> {
    object
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{name} 必须是字符串"))
}

fn optional_string<'a>(
    object: &'a Map<String, Value>,
    name: &str,
) -> Result<Option<&'a str>, String> {
    object
        .get(name)
        .map(|value| value.as_str().ok_or_else(|| format!("{name} 必须是字符串")))
        .transpose()
}

fn optional_bool(object: &Map<String, Value>, name: &str) -> Result<Option<bool>, String> {
    object
        .get(name)
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| format!("{name} 必须是布尔值"))
        })
        .transpose()
}

fn bounded_integer(
    object: &Map<String, Value>,
    name: &str,
    default: u64,
    minimum: u64,
    maximum: u64,
) -> Result<u64, String> {
    let value = match object.get(name) {
        Some(value) => value
            .as_u64()
            .ok_or_else(|| format!("{name} 必须是正整数"))?,
        None => default,
    };
    (minimum..=maximum)
        .contains(&value)
        .then_some(value)
        .ok_or_else(|| format!("{name} 必须在 {minimum} 到 {maximum} 之间"))
}

/// systemd unit 名使用受限字符集；禁止首字符为 `-`，避免被命令解释成选项。
fn service_name(raw: &str) -> Result<&str, String> {
    let value = raw.trim();
    let valid = !value.is_empty()
        && value.len() <= 128
        && value.chars().next().is_some_and(char::is_alphanumeric)
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".@_:-".contains(character));
    valid
        .then_some(value)
        .ok_or_else(|| "service 不是有效的 systemd unit 名".to_string())
}

/// Docker 名称和短/长 ID 都落在这个字符集内；禁止前导 `-` 以封死选项注入。
fn docker_container_name(raw: &str) -> Result<&str, String> {
    let value = raw.trim();
    let valid = !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_.-".contains(character));
    valid
        .then_some(value)
        .ok_or_else(|| "container 不是有效的 Docker 容器名称或 ID".to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{build_server_inspection, server_tool_definitions};

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
    }

    #[test]
    fn system_and_docker_commands_are_generated_without_free_form_input() {
        let system = build_server_inspection("get_system_info", &json!({})).unwrap();
        assert_eq!(system.command, "uname -n && uname -srm && uptime");

        let docker =
            build_server_inspection("list_docker_containers", &json!({"includeStopped":true}))
                .unwrap();
        assert!(docker.command.contains("docker ps --all"));
        assert!(docker.command.contains("{{.Names}}"));
    }

    #[test]
    fn service_tools_reject_option_and_shell_injection() {
        for service in ["--all", "docker;reboot", "docker service", "$(reboot)"] {
            assert!(
                build_server_inspection("get_service_status", &json!({"service":service})).is_err()
            );
        }
        let plan = build_server_inspection(
            "read_service_logs",
            &json!({"service":"docker.service","lines":50,"priority":"warning"}),
        )
        .unwrap();
        assert!(plan.command.contains("--unit=docker.service"));
        assert!(plan.command.contains("--lines=50"));
    }

    #[test]
    fn docker_diagnostics_are_bounded_and_do_not_expose_environment_variables() {
        let info = build_server_inspection("get_docker_info", &json!({})).unwrap();
        assert!(info.command.contains("RegistryMirrors"));

        let status = build_server_inspection(
            "get_docker_container_status",
            &json!({"container":"new-api"}),
        )
        .unwrap();
        assert!(status.command.contains("RestartCount"));
        assert!(!status.command.contains(".Config.Env"));

        let logs = build_server_inspection(
            "read_docker_logs",
            &json!({"container":"6b28ba412b61","lines":100,"sinceMinutes":30}),
        )
        .unwrap();
        assert!(
            logs.command
                .contains("--tail 100 --since 30m -- 6b28ba412b61")
        );
        for container in ["--help", "new-api;reboot", "$(reboot)", "new api"] {
            assert!(
                build_server_inspection("read_docker_logs", &json!({"container":container}))
                    .is_err()
            );
        }
    }

    #[test]
    fn bounded_parameters_and_unknown_fields_fail_closed() {
        assert!(build_server_inspection("list_processes", &json!({"limit":201})).is_err());
        assert!(
            build_server_inspection("read_service_logs", &json!({"service":"docker","lines":0}))
                .is_err()
        );
        assert!(build_server_inspection("get_disk_usage", &json!({"command":"rm -rf /"})).is_err());
    }
}
