use std::{error::Error, fmt};

/// 审计目标只记录安全分类和本地连接主键，不复制主机地址、账号或终端输出。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiAuditTarget {
    Ssh { connection_id: i64 },
    Local,
}

impl AiAuditTarget {
    pub const fn kind(self) -> &'static str {
        match self {
            Self::Ssh { .. } => "ssh",
            Self::Local => "local",
        }
    }

    pub const fn connection_id(self) -> Option<i64> {
        match self {
            Self::Ssh { connection_id } => Some(connection_id),
            Self::Local => None,
        }
    }
}

/// 工具名使用封闭枚举，防止把任意命令或 Provider 输入误写进审计库。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiAuditTool {
    SessionContext,
    GetSystemInfo,
    ListProcesses,
    ListListeningPorts,
    GetServiceStatus,
    ReadServiceLogs,
    GetDiskUsage,
    GetMemoryUsage,
    ListDockerContainers,
    GetDockerInfo,
    GetDockerContainerStatus,
    ReadDockerLogs,
    SshExec,
    LocalTerminalExec,
    Unknown,
}

impl AiAuditTool {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SessionContext => "session_context",
            Self::GetSystemInfo => "get_system_info",
            Self::ListProcesses => "list_processes",
            Self::ListListeningPorts => "list_listening_ports",
            Self::GetServiceStatus => "get_service_status",
            Self::ReadServiceLogs => "read_service_logs",
            Self::GetDiskUsage => "get_disk_usage",
            Self::GetMemoryUsage => "get_memory_usage",
            Self::ListDockerContainers => "list_docker_containers",
            Self::GetDockerInfo => "get_docker_info",
            Self::GetDockerContainerStatus => "get_docker_container_status",
            Self::ReadDockerLogs => "read_docker_logs",
            Self::SshExec => "ssh_exec",
            Self::LocalTerminalExec => "local_terminal_exec",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiAuditApproval {
    NotRequested,
    NotRequired,
    Requested,
    Approved,
    Rejected,
    TimedOut,
}

impl AiAuditApproval {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotRequested => "not_requested",
            Self::NotRequired => "not_required",
            Self::Requested => "requested",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::TimedOut => "timed_out",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiAuditOutcome {
    Pending,
    Succeeded,
    Failed,
    Denied,
}

impl AiAuditOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Denied => "denied",
        }
    }
}

/// 单条不可变审计事件。参数、命令、stdout/stderr 和 Bridge token 不属于该模型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiToolAuditEvent {
    pub provider: String,
    pub session_id: String,
    pub target: AiAuditTarget,
    pub tool: AiAuditTool,
    pub approval: AiAuditApproval,
    pub outcome: AiAuditOutcome,
    pub duration_ms: Option<u64>,
    pub error_code: Option<&'static str>,
}

/// 审计写入是安全决策的一部分，因此 Port 不提供忽略失败的默认实现。
pub trait AiAuditRepository: Send + Sync {
    fn append(&self, event: &AiToolAuditEvent) -> Result<(), AiAuditRepositoryError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiAuditRepositoryError {
    message: String,
}

impl AiAuditRepositoryError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for AiAuditRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for AiAuditRepositoryError {}
