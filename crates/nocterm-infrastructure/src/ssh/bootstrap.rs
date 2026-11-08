//! SSH shell integration bootstrap 的纯逻辑：登录 shell 探针解析、临时 rc 内容
//! 生成与注入命令构造。
//!
//! 本模块不含任何网络与 tokio 代码，全部可无环境单测（阶段 3 退出标准）。
//! 网络编排见 `super::terminal::connect_session`。
//!
//! ## 设计决策：source 注入而非包装 exec
//!
//! SSH `request_shell` 启动的是**登录 shell**（bash 走 /etc/profile →
//! ~/.bash_profile 链，zsh 走 .zprofile/.zshrc/.zlogin 链），用户的 PATH 与
//! 环境通常在 profile 链中。若用 `exec bash --rcfile` / 非 login zsh 替换
//! shell，登录链不会执行，用户环境会被破坏（真实环境验证发现）。
//!
//! 因此 bootstrap 保持 `request_shell` 原样，只做：
//! 探针识别 shell → 上传临时 rc（含 nonce 与集成钩子）→ 会话就绪后向 PTY
//! 敲入一条 `source '<path>'`。登录链 100% 保留；与 `initial_path` 的 cd
//! 使用同一注入机制。代价是命令行回显一行（以空格开头配合
//! `HISTCONTROL=ignorespace` 避免进入 history）。
//!
//! 安全约束（演进方案 §10.3）：
//! - 不修改用户持久 rc 文件；临时路径每次随机生成；
//! - 远端以 `umask 077` 创建临时文件（0600 权限）；
//! - source 返回后由注入命令立即删除临时 rc，上传失败另走兜底清理；
//! - 不把凭据、token、命令或完整环境写入临时脚本。

use crate::terminal::shell_single_quoted;
use nocterm_domain::terminal::{SshShellIntegrationPlan, SshShellKind};

/// POSIX 登录 shell 探针：一次性输出 bash/zsh/fish 版本变量的分隔串。
pub(super) const SHELL_PROBE_COMMAND: &str = "printf '%s|%s|%s|%s\\n' \"$BASH_VERSION\" \"$ZSH_VERSION\" \"$FISH_VERSION\" \"$POWERSHELL_VERSION\"";
/// Windows OpenSSH 使用系统配置的默认 Shell 解释 exec 请求；POSIX 探针未命中时
/// 再执行 PowerShell 语法，避免把错误输出误判成受支持 Shell。
pub(super) const POWERSHELL_PROBE_COMMAND: &str = "$v=$PSVersionTable.PSVersion; if($null -ne $v){Write-Output ('__NOCTERM_POWERSHELL__|' + $v.ToString())}";

/// 从探针输出解析受支持的 shell。PowerShell 使用带固定前缀的独立探针，
/// 不能仅凭错误文本或环境变量猜测远端平台。
pub(super) fn parse_shell_probe(output: &str) -> Option<SshShellKind> {
    if output.lines().any(|line| {
        line.trim()
            .strip_prefix("__NOCTERM_POWERSHELL__|")
            .is_some_and(|version| !version.trim().is_empty())
    }) {
        return Some(SshShellKind::PowerShell);
    }
    let first_line = output.lines().next()?.trim();
    let mut fields = first_line.split('|');
    let bash = fields.next()?.trim();
    let zsh = fields.next()?.trim();
    let fish = fields.next()?.trim();
    let _legacy_placeholder = fields.next()?.trim();
    if !bash.is_empty() {
        Some(SshShellKind::Bash)
    } else if !zsh.is_empty() {
        Some(SshShellKind::Zsh)
    } else if !fish.is_empty() {
        Some(SshShellKind::Fish)
    } else {
        None
    }
}

/// 生成远程临时 rc 路径：仅含 `/`、字母、数字、`_`、`-`、`.`，免单引号转义。
pub fn remote_rc_path(random_hex: &str) -> String {
    format!("/tmp/nocterm-ssh-{random_hex}.rc")
}

/// 上传临时 rc 的远端命令：umask 077 保证文件 0600；内容经 stdin 传入。
pub(super) fn rc_upload_command(path: &str) -> String {
    format!("umask 077 && cat > {}", shell_single_quoted(path))
}

/// 生成临时 rc 内容：nonce 导出 → 对应 shell 的集成钩子。
///
/// 不 source 用户 rc：login shell 已加载完整用户配置，这里只装钩子。
/// 自删除由 `source_command` 的注入命令负责（见其文档）——integration 脚本
/// 幂等守卫的 `return` 会跳过 rc 尾部的任何语句，删除不能放在 rc 内容里。
/// PowerShell 不经过临时 rc（见 `integration_command` 的内存注入，nonce 绑定
/// 在注入命令前缀完成），此处的 PowerShell 分支仅为匹配穷尽保留。
pub(super) fn build_rc_content(plan: &SshShellIntegrationPlan, _path: &str) -> String {
    let integration = match plan.shell {
        SshShellKind::Bash => include_str!("../terminal/integration_bash.sh"),
        SshShellKind::Zsh => include_str!("../terminal/integration_zsh.zsh"),
        SshShellKind::Fish => include_str!("../terminal/integration_fish.fish"),
        // run_bootstrap 在上传前已对 PowerShell 提前返回，生产路径不可达。
        SshShellKind::PowerShell => include_str!("../terminal/integration_pwsh.ps1"),
    };
    let nonce_assignment = match plan.shell {
        SshShellKind::Bash | SshShellKind::Zsh => format!(
            "export NOCTERM_OSC_NONCE={}",
            shell_single_quoted(&plan.nonce)
        ),
        // nonce 由本机生成且只含十六进制字符；仍统一单引号，避免未来格式
        // 扩展时把内容解释为 fish 命令替换或变量展开。
        SshShellKind::Fish => format!(
            "set -gx NOCTERM_OSC_NONCE {}",
            shell_single_quoted(&plan.nonce)
        ),
        SshShellKind::PowerShell => {
            format!("$env:NOCTERM_OSC_NONCE = '{}'", plan.nonce)
        }
    };
    format!("{nonce_assignment}\n{integration}\n")
}

/// 注入命令：登录 shell 就绪后向 PTY 敲入，加载临时 rc 并立即删除。
///
/// 删除放在注入命令里而不是 rc 内容尾部——integration 脚本开头的幂等守卫
/// `return` 会跳过其后的所有行，把删除责任交给 rc 内容会产生残留（真实环境
/// 验证发现的 5 个 /tmp 残留即此）。`source` 返回后 rm 必然执行；行首空格
/// 配合 `HISTCONTROL=ignorespace`（Ubuntu 默认）避免进入 history；bash 与
/// zsh 的 `source` 语义一致，无需按 shell 分支。
///
/// 尾部的 `printf` 清理序列擦除上一行残留的登录提示符：注入命令必然以一次
/// 回车结束，tty 回显的换行会让远端再打印一次提示符，用户因此看到两行
/// 提示符（beta.2 实测回归）。命令执行点光标位于回显换行后的空行，`ESC[A`
/// 上移到登录提示符行、`ESC[2K` 擦除、`\r` 归零列，随后的 integration
/// 钩子事件（被剥离）与 PS1 会在该行重新打印唯一提示符。执行时登录提示符
/// 必已渲染（探针与上传的往返远长于登录链），擦除目标行稳定存在。
pub(super) fn source_command(path: &str) -> String {
    let quoted = shell_single_quoted(path);
    format!(" source {quoted}; rm -f -- {quoted}; printf '\\033[1A\\033[2K\\r'\n")
}

/// 返回交互式 Shell 中实际注入的命令。PowerShell 直接在内存中解码脚本，
/// 避免依赖 Windows 远端临时目录、ACL 与 POSIX 文件命令。
pub(super) fn integration_command(plan: &SshShellIntegrationPlan) -> String {
    match plan.shell {
        SshShellKind::PowerShell => {
            let encoded =
                base64_encode(include_str!("../terminal/integration_pwsh.ps1").as_bytes());
            format!(
                " $env:NOCTERM_OSC_NONCE='{}'; . ([scriptblock]::Create([Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{}'))))\r\n",
                plan.nonce, encoded
            )
        }
        SshShellKind::Bash | SshShellKind::Zsh | SshShellKind::Fish => {
            source_command(&remote_rc_path(&plan.nonce))
        }
    }
}

/// 注入回显过滤使用的唯一片段。只在 bootstrap 写入到首个换行期间生效，
/// 不作为完成或 readiness 证据。
pub fn integration_echo_token(plan: &SshShellIntegrationPlan) -> String {
    match plan.shell {
        SshShellKind::PowerShell => "FromBase64String".to_string(),
        SshShellKind::Bash | SshShellKind::Zsh | SshShellKind::Fish => remote_rc_path(&plan.nonce),
    }
}

/// 初始目录切换必须由远端 Shell 自己解释，不能用 POSIX 引号拼接 Windows 路径。
pub(super) fn initial_directory_command(path: &str, shell: Option<SshShellKind>) -> String {
    match shell {
        Some(SshShellKind::PowerShell) => {
            format!(
                "Set-Location -LiteralPath '{}'\r\n",
                path.replace('\'', "''")
            )
        }
        _ => format!("cd {}\n", shell_single_quoted(path)),
    }
}

/// 阶段 1：在独立无 PTY channel 上执行登录 shell 探针。
///
/// 失败（超时、通道拒绝、输出不可读）统一返回 Err，由调用方降级。
pub(super) async fn exec_probe(
    handle: &russh::client::Handle<super::connection::ClientHandler>,
) -> Result<String, &'static str> {
    let posix_result = exec_probe_command(handle, SHELL_PROBE_COMMAND, "POSIX").await;
    if let Ok(output) = &posix_result
        && parse_shell_probe(output).is_some()
    {
        return posix_result;
    }
    exec_probe_command(handle, POWERSHELL_PROBE_COMMAND, "PowerShell").await
}

async fn exec_probe_command(
    handle: &russh::client::Handle<super::connection::ClientHandler>,
    command: &str,
    label: &str,
) -> Result<String, &'static str> {
    eprintln!("[nocterm-ssh-bootstrap] 阶段 1：探测 {label} 登录 shell …");
    let channel = handle.channel_open_session().await.map_err(|error| {
        eprintln!("[nocterm-ssh-bootstrap] 探针通道打开失败：{error}");
        "probe_channel_failed" as &'static str
    })?;
    channel
        .exec(false, command.as_bytes().to_vec())
        .await
        .map_err(|_| "probe_exec_failed" as &'static str)?;
    let (mut read_half, write_half) = channel.split();
    let _ = write_half.eof().await;
    let mut output = Vec::new();
    let deadline = std::time::Instant::now() + SHELL_PROBE_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err("probe_timeout" as &'static str);
        }
        match tokio::time::timeout(remaining, read_half.wait()).await {
            Err(_) => return Err("probe_timeout" as &'static str),
            Ok(None) => break,
            Ok(Some(russh::ChannelMsg::Data { data }))
            | Ok(Some(russh::ChannelMsg::ExtendedData { data, .. })) => {
                output.extend_from_slice(&data);
                // 探针输出只有一行，凑齐换行即可提前结束。
                if output.contains(&b'\n') {
                    break;
                }
            }
            Ok(Some(_)) => {}
        }
    }
    let _ = write_half.close().await;
    let text = String::from_utf8(output).map_err(|_| {
        eprintln!("[nocterm-ssh-bootstrap] 探针输出不是有效 UTF-8");
        "probe_output_invalid" as &'static str
    })?;
    eprintln!("[nocterm-ssh-bootstrap] {label} 探针输出：{text:?}");
    Ok(text)
}

/// 标准 Base64 编码用于把 PowerShell integration 以内存脚本注入远端。
/// 内容不是秘密；编码只解决跨 Shell 引号和换行传输，不承担加密职责。
/// 刻意不引入 `base64` 依赖：这是仓库中唯一的编码调用点，实现由标准向量
/// 测试锁定；新增生产依赖需按仓库规则评审，收益不足以抵消评审成本。
fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let first = u32::from(chunk[0]);
        let second = chunk.get(1).map_or(0, |value| u32::from(*value));
        let third = chunk.get(2).map_or(0, |value| u32::from(*value));
        let group = (first << 16) | (second << 8) | third;
        out.push(ALPHABET[((group >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((group >> 12) & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[((group >> 6) & 0x3f) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(group & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// 阶段 2：在独立 channel 上以 `umask 077 && cat > <path>` 上传 rc 内容。
///
/// 内容经 channel stdin 写入（免转义）；EOF 后等待远端收尾，任何失败都返回 Err。
pub(super) async fn upload_rc(
    handle: &russh::client::Handle<super::connection::ClientHandler>,
    path: &str,
    content: &[u8],
) -> Result<(), &'static str> {
    eprintln!(
        "[nocterm-ssh-bootstrap] 阶段 2：上传临时 rc（{path}，{} 字节）…",
        content.len()
    );
    let channel = handle.channel_open_session().await.map_err(|error| {
        eprintln!("[nocterm-ssh-bootstrap] 上传通道打开失败：{error}");
        "upload_channel_failed" as &'static str
    })?;
    channel
        .exec(false, rc_upload_command(path).into_bytes())
        .await
        .map_err(|_| "upload_exec_failed" as &'static str)?;
    let (mut read_half, write_half) = channel.split();
    write_half
        .data_bytes(content.to_vec())
        .await
        .map_err(|_| "upload_write_failed" as &'static str)?;
    let _ = write_half.eof().await;
    let deadline = std::time::Instant::now() + RC_UPLOAD_TIMEOUT;
    let mut exit_status = None;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err("upload_timeout" as &'static str);
        }
        match tokio::time::timeout(remaining, read_half.wait()).await {
            Err(_) => return Err("upload_timeout" as &'static str),
            Ok(None) | Ok(Some(russh::ChannelMsg::Close)) => break,
            Ok(Some(russh::ChannelMsg::ExitStatus {
                exit_status: status,
            })) => {
                exit_status = Some(status);
            }
            Ok(Some(_)) => {}
        }
    }
    match exit_status {
        Some(0) => {
            eprintln!("[nocterm-ssh-bootstrap] 上传完成：{path}");
            Ok(())
        }
        Some(_) => Err("upload_remote_failed"),
        None => Err("upload_status_missing"),
    }
}

/// 阶段 2 失败清理：尽力删除半途创建的临时 rc；失败静默（下一次
/// bootstrap 会生成全新随机路径，残留文件不含敏感内容且受 0600 保护）。
pub(super) async fn cleanup_rc(
    handle: &russh::client::Handle<super::connection::ClientHandler>,
    path: &str,
) {
    let channel = match handle.channel_open_session().await {
        Ok(channel) => channel,
        Err(_) => return,
    };
    let command = format!("rm -f -- {}", shell_single_quoted(path));
    if channel.exec(false, command.into_bytes()).await.is_ok() {
        let (mut read_half, write_half) = channel.split();
        let _ = write_half.eof().await;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, read_half.wait()).await {
                Err(_) => break,
                Ok(None) | Ok(Some(russh::ChannelMsg::Close)) => break,
                Ok(Some(_)) => {}
            }
        }
    }
}

/// 探针短窗超时；整体仍受 CONNECT_READY_TIMEOUT 兜底约束。
pub(super) const SHELL_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// 临时 rc 上传超时；umask 077 + 单文件写入，正常在 RTT 内完成。
pub(super) const RC_UPLOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(shell: SshShellKind) -> SshShellIntegrationPlan {
        SshShellIntegrationPlan {
            nonce: "abc123".to_string(),
            shell,
        }
    }

    /// 真实环境回归（首次 SSH 验证发现）：`builtin` 前缀只能调用 shell 内建命令，
    /// 对自定义函数使用会导致每轮提示符渲染报
    /// `bash: builtin: __nocterm_emit: not a shell builtin`。
    /// 生成的 rc 内容中，不得出现对 `__nocterm_` 函数的 builtin 调用。
    #[test]
    fn rc_content_never_invokes_nocterm_functions_through_builtin() {
        for (content, script) in [
            (
                build_rc_content(&plan(SshShellKind::Bash), &remote_rc_path("aabb")),
                "bash",
            ),
            (
                build_rc_content(&plan(SshShellKind::Zsh), &remote_rc_path("ccdd")),
                "zsh",
            ),
        ] {
            for line in content.lines() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("builtin __nocterm") {
                    panic!("{script} uses builtin prefix on a custom function: {line}");
                }
            }
        }
    }

    /// 真实环境回归（首次 SSH 验证发现）：旧 emit 实现在 nonce 模式下对 A 事件
    /// 输出 `633;A;;<nonce>`，空事件体占用第二字段——后端把三字段 A 事件按畸形
    /// 丢弃，Prompt Ready 永远不置位。生成的脚本必须满足字段契约：
    /// A 事件体为空时 nonce 直接顶上第二字段。
    #[test]
    fn emit_contract_keeps_nonce_in_second_field_for_empty_bodies() {
        for (content, script) in [
            (
                build_rc_content(&plan(SshShellKind::Bash), &remote_rc_path("aabb")),
                "bash",
            ),
            (
                build_rc_content(&plan(SshShellKind::Zsh), &remote_rc_path("ccdd")),
                "zsh",
            ),
        ] {
            // 每个脚本都必须先拼"事件名;事件体"再追加 nonce，而不是固定三参数。
            let emit_body = content
                .find("local fields=\"$1\"")
                .or_else(|| content.find("set -l fields $argv[1]"))
                .unwrap_or_else(|| panic!("{script} emit must build fields incrementally"));
            let three_arg_printf = content.find("printf '\\033]633;%s;%s;%s\\007'").is_some();
            assert!(
                !three_arg_printf,
                "{script} uses a fixed three-field printf that emits empty-body placeholders"
            );
            assert!(emit_body > 0);
        }
    }

    #[test]
    fn probe_output_parses_supported_shells_by_version_position() {
        assert_eq!(
            parse_shell_probe("5.1.16|0.0||\n"),
            Some(SshShellKind::Bash)
        );
        assert_eq!(parse_shell_probe("|5.9|0.0|\n"), Some(SshShellKind::Zsh));
        assert_eq!(parse_shell_probe("||3.7.1|\n"), Some(SshShellKind::Fish));
        assert_eq!(
            parse_shell_probe("__NOCTERM_POWERSHELL__|7.4.6\r\n"),
            Some(SshShellKind::PowerShell)
        );
        // 没有固定前缀的第四字段不能被错误文本或环境变量伪装成 PowerShell。
        assert_eq!(parse_shell_probe("|||7.4\n"), None);
        assert_eq!(parse_shell_probe("__NOCTERM_POWERSHELL__|\n"), None);
        assert_eq!(parse_shell_probe("|||\n"), None);
        assert_eq!(parse_shell_probe(""), None);
    }

    #[test]
    fn fish_rc_uses_fish_nonce_assignment_and_postexec_hook() {
        let content = build_rc_content(&plan(SshShellKind::Fish), &remote_rc_path("fish0011"));

        assert!(content.contains("set -gx NOCTERM_OSC_NONCE 'abc123'"));
        assert!(content.contains("--on-event fish_postexec"));
        assert!(!content.contains("export NOCTERM_OSC_NONCE"));
    }

    #[test]
    fn powershell_uses_memory_injection_with_nonce_bound_osc_hooks() {
        let plan = plan(SshShellKind::PowerShell);
        let content = build_rc_content(&plan, "unused");
        let command = integration_command(&plan);

        assert!(content.contains("$env:NOCTERM_OSC_NONCE = 'abc123'"));
        assert!(content.contains("function global:__nocterm_emit"));
        assert!(content.contains("$fields;$($env:NOCTERM_OSC_NONCE)"));
        assert!(content.contains("__NOCTERM_LAST_ERROR_RECORD"));
        assert!(content.contains("ReferenceEquals"));
        assert!(command.contains("$env:NOCTERM_OSC_NONCE='abc123'"));
        assert!(command.contains("[Convert]::FromBase64String("));
        assert!(!command.contains("/tmp/"));
        assert!(!command.contains("Remove-Item"));
        assert_eq!(integration_echo_token(&plan), "FromBase64String");
    }

    #[test]
    fn initial_directory_command_uses_shell_specific_literal_quoting() {
        assert_eq!(
            initial_directory_command("C:\\Users\\O'Brien", Some(SshShellKind::PowerShell)),
            "Set-Location -LiteralPath 'C:\\Users\\O''Brien'\r\n"
        );
        assert_eq!(
            initial_directory_command("/srv/it's", Some(SshShellKind::Bash)),
            "cd '/srv/it'\\''s'\n"
        );
    }

    #[test]
    fn bootstrap_base64_uses_the_standard_padded_alphabet() {
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"M"), "TQ==");
    }

    #[test]
    fn remote_paths_are_random_and_escape_free() {
        let path = remote_rc_path("aabbccdd00112233");
        assert!(path.starts_with("/tmp/nocterm-ssh-"));
        assert!(path.ends_with(".rc"));
        // 点号只出现在扩展名处；其余字符免单引号转义。
        assert!(
            path.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '.'))
        );
    }

    #[test]
    fn upload_command_uses_restrictive_umask_and_quoted_path() {
        let command = rc_upload_command("/tmp/nocterm-ssh-x.rc");
        assert!(command.starts_with("umask 077 && cat > "));
        assert!(command.ends_with("'/tmp/nocterm-ssh-x.rc'"));
    }

    #[test]
    fn rc_content_exports_nonce_and_installs_hooks_without_self_delete() {
        for (content, script) in [
            (
                build_rc_content(&plan(SshShellKind::Bash), &remote_rc_path("aabb")),
                "bash",
            ),
            (
                build_rc_content(&plan(SshShellKind::Zsh), &remote_rc_path("ccdd")),
                "zsh",
            ),
        ] {
            let nonce = content
                .find("export NOCTERM_OSC_NONCE='abc123'")
                .unwrap_or_else(|| panic!("{script} must export the nonce"));
            let hooks = content
                .find(if script == "bash" {
                    "PROMPT_COMMAND="
                } else {
                    "precmd_functions+=("
                })
                .unwrap_or_else(|| panic!("{script} must install hooks"));
            assert!(nonce < hooks);
            // 删除责任在注入命令（source; rm），rc 内容自身不含 rm——
            // integration 幂等守卫的 return 不会再跳过删除。
            assert!(!content.contains("rm -f"));
            // 不重建登录链：login shell 已加载用户全部配置。
            assert!(!content.contains(". \"$HOME/.bashrc\""));
            assert!(!content.contains(".zshrc\""));
        }
    }

    #[test]
    fn source_command_carries_the_delete_responsibility() {
        let command = source_command("/tmp/nocterm-ssh-x.rc");
        // 脚本守卫 return 也不能跳过删除：rm 必须在 source 之后同命令执行；
        // 尾部 printf 在远端擦除残留登录提示符，避免连接后出现两行提示符。
        assert_eq!(
            command,
            " source '/tmp/nocterm-ssh-x.rc'; rm -f -- '/tmp/nocterm-ssh-x.rc'; printf '\\033[1A\\033[2K\\r'\n"
        );
    }
}
