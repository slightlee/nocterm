//! 本地终端的默认 Shell 选择：本文件集中承载这一步的**全部**平台差异。
//!
//! 从 `terminal/mod.rs` 拆出来的理由是规模而非风格：Windows 侧要自己走一遍 `PATH`
//! 与 `PATHEXT` 做可执行文件查找，连测试算上超过一百行，且在 macOS 上一行都不编译。
//! 混在 `LocalTerminalManager` 的 PTY 生命周期代码里，会让 macOS 读者需要跳过整整
//! 三分之一的文件；而 PTY 本身的平台差异已经由 `portable-pty` 消化，`mod.rs` 因此
//! 可以做到零 `cfg`。
//!
//! Unix 侧只有一行（交给 `portable-pty` 读 `SHELL`），差异不对称是事实而不是遗漏：
//! macOS 的默认 Shell 由系统与用户配置决定，没有"该挑哪个"的问题。

use portable_pty::CommandBuilder;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LocalShellKind {
    Posix,
    Fish,
    PowerShell,
    Cmd,
}

/// Windows 的系统默认 Shell 通常是 CMD，但 PowerShell 更适合作为客户端默认终端。
/// 候选按现代 PowerShell、系统 PowerShell、用户配置和最后兜底的顺序选择。
pub(super) fn local_shell_command() -> (CommandBuilder, LocalShellKind) {
    #[cfg(windows)]
    {
        for shell in windows_shell_candidates(std::env::var("COMSPEC").ok()).iter() {
            if windows_command_available(shell) {
                return (
                    apply_terminal_identity(CommandBuilder::new(shell)),
                    detect_shell_kind(shell),
                );
            }
        }
        (
            apply_terminal_identity(CommandBuilder::new("cmd.exe")),
            LocalShellKind::Cmd,
        )
    }

    // macOS 与其它 Unix 走 portable-pty 的默认程序解析（读 `SHELL`，回落到 passwd 项）。
    #[cfg(not(windows))]
    {
        let command =
            apply_default_term(apply_terminal_identity(CommandBuilder::new_default_prog()));
        let kind = detect_shell_kind(&command.get_shell());
        (command, kind)
    }
}

/// PTY 内 Shell 的默认 TERM 值，与前端 xterm.js 的能力对齐。
#[cfg(not(windows))]
pub(super) const DEFAULT_TERM: &str = "xterm-256color";

/// 终端身份声明：`TERM_PROGRAM` 必须覆写为 Nocterm 自己，而不是继承启动方。
/// 从 Apple Terminal 里启动 Nocterm 时若继承 `Apple_Terminal`，`/etc/zshrc`
/// 会 source `/etc/zshrc_Apple_Terminal` 并重放 `~/.zsh_sessions/*.session`，
/// 把上一个会话的命令当作新终端的启动输出（典型报错：`command not found: Saving`）。
/// iTerm2 / VS Code 等同类终端均无条件声明自己的 `TERM_PROGRAM`。
pub(super) const TERM_PROGRAM_VALUE: &str = "Nocterm";

fn apply_terminal_identity(mut command: CommandBuilder) -> CommandBuilder {
    command.env("TERM_PROGRAM", TERM_PROGRAM_VALUE);
    command
}

/// 从 Finder/Dock 启动的 GUI 进程只继承 launchd 最小环境，常没有 `TERM`。
/// zsh 的 ZLE 在 TERM 缺失时会在提示符的多字节字符（如 ➜、✗）前输出字面 `?`，
/// 因此仅在进程环境未提供 TERM 时补默认值；显式设置的值原样保留。
#[cfg(not(windows))]
fn apply_default_term(mut command: CommandBuilder) -> CommandBuilder {
    if command.get_env("TERM").is_none() {
        command.env("TERM", DEFAULT_TERM);
    }
    command
}

#[cfg(all(test, not(windows)))]
mod term_default_tests {
    use super::{DEFAULT_TERM, apply_default_term};
    use portable_pty::CommandBuilder;
    use std::ffi::OsStr;

    #[test]
    fn fills_term_only_when_the_process_env_does_not_provide_one() {
        let mut command = CommandBuilder::new("zsh");
        command.env_clear();
        let command = apply_default_term(command);
        assert_eq!(command.get_env("TERM"), Some(OsStr::new(DEFAULT_TERM)));
    }

    #[test]
    fn keeps_an_explicit_term_from_the_parent_environment() {
        let mut command = CommandBuilder::new("zsh");
        command.env_clear();
        command.env("TERM", "screen-256color");
        let command = apply_default_term(command);
        assert_eq!(command.get_env("TERM"), Some(OsStr::new("screen-256color")));
    }
}

fn detect_shell_kind(program: &str) -> LocalShellKind {
    // 同一构建也要能识别另一平台的路径文本，不能依赖宿主 `Path` 的分隔符规则。
    let name = program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program)
        .to_ascii_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    match name {
        "fish" => LocalShellKind::Fish,
        "powershell" | "pwsh" => LocalShellKind::PowerShell,
        "cmd" => LocalShellKind::Cmd,
        _ => LocalShellKind::Posix,
    }
}

/// 标记只由 Nocterm 生成且仅含 ASCII 字母、数字和下划线，可安全嵌入各 Shell。
pub(super) fn completion_command(kind: LocalShellKind, marker: &str) -> String {
    match kind {
        LocalShellKind::Posix => format!("printf '\\n{marker}%s\\n' \"$?\""),
        LocalShellKind::Fish => format!("printf '\\n{marker}%s\\n' $status"),
        LocalShellKind::PowerShell => format!(
            "$__nocterm_ok = $?; $__nocterm_code = if ($__nocterm_ok) {{ 0 }} elseif ($null -ne $LASTEXITCODE) {{ $LASTEXITCODE }} else {{ 1 }}; Write-Output \"{marker}$__nocterm_code\""
        ),
        LocalShellKind::Cmd => format!("echo {marker}%ERRORLEVEL%"),
    }
}

#[cfg(test)]
mod terminal_identity_tests {
    use super::{TERM_PROGRAM_VALUE, apply_terminal_identity};
    use portable_pty::CommandBuilder;
    use std::ffi::OsStr;

    #[test]
    fn overrides_an_inherited_term_program_from_the_launching_terminal() {
        let mut command = CommandBuilder::new("zsh");
        command.env("TERM_PROGRAM", "Apple_Terminal");
        let command = apply_terminal_identity(command);
        assert_eq!(
            command.get_env("TERM_PROGRAM"),
            Some(OsStr::new(TERM_PROGRAM_VALUE))
        );
    }

    #[test]
    fn sets_identity_even_when_the_launch_environment_has_none() {
        let command = apply_terminal_identity(CommandBuilder::new("zsh"));
        assert_eq!(
            command.get_env("TERM_PROGRAM"),
            Some(OsStr::new(TERM_PROGRAM_VALUE))
        );
    }
}

#[cfg(test)]
mod completion_tests {
    use super::{LocalShellKind, completion_command, detect_shell_kind};

    const MARKER: &str = "__NOCTERM_LOCAL_AI_DONE_test__";

    #[test]
    fn emits_status_probe_for_each_supported_shell_family() {
        assert_eq!(
            completion_command(LocalShellKind::Posix, MARKER),
            format!("printf '\\n{MARKER}%s\\n' \"$?\"")
        );
        assert_eq!(
            completion_command(LocalShellKind::Fish, MARKER),
            format!("printf '\\n{MARKER}%s\\n' $status")
        );
        assert!(
            completion_command(LocalShellKind::PowerShell, MARKER)
                .contains(&format!("Write-Output \"{MARKER}$__nocterm_code\""))
        );
        assert_eq!(
            completion_command(LocalShellKind::Cmd, MARKER),
            format!("echo {MARKER}%ERRORLEVEL%")
        );
    }

    #[test]
    fn detects_shell_family_from_executable_name() {
        assert_eq!(detect_shell_kind("/bin/zsh"), LocalShellKind::Posix);
        assert_eq!(
            detect_shell_kind("/opt/homebrew/bin/fish"),
            LocalShellKind::Fish
        );
        assert_eq!(detect_shell_kind("pwsh.exe"), LocalShellKind::PowerShell);
        assert_eq!(
            detect_shell_kind(r"C:\\Windows\\System32\\cmd.exe"),
            LocalShellKind::Cmd
        );
    }
}

#[cfg(windows)]
fn windows_shell_candidates(comspec: Option<String>) -> Vec<String> {
    let mut candidates = vec!["pwsh.exe".to_string(), "powershell.exe".to_string()];
    if let Some(comspec) = comspec.filter(|value| !value.trim().is_empty()) {
        candidates.push(comspec);
    } else {
        candidates.push("cmd.exe".to_string());
    }
    candidates
}

/// 判断某个 Shell 可执行文件是否存在。
///
/// 刻意不 spawn `where.exe`：Tauri 是 GUI 子系统进程，从它启动控制台程序会闪出一个
/// 黑色窗口（每个候选闪一次），而 `status()` 还会同步等子进程退出，把"打开本地终端"
/// 卡在两次进程创建上。自己走一遍 `PATH` 既没有窗口也没有等待。
#[cfg(windows)]
fn windows_command_available(program: &str) -> bool {
    let path = std::path::Path::new(program);
    // COMSPEC 通常是绝对路径，直接落地检查。
    if path.is_absolute() {
        return path.is_file();
    }
    // 带分隔符的相对路径按当前目录解析，与 CreateProcess 的查找规则保持一致。
    if path.components().count() > 1 {
        return path.is_file();
    }
    let Some(search_path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&search_path).any(|dir| {
        // 候选名自带 `.exe`；只有没写扩展名时才需要按 PATHEXT 逐个拼。
        if path.extension().is_some() {
            return dir.join(program).is_file();
        }
        windows_path_extensions()
            .iter()
            .any(|extension| dir.join(format!("{program}{extension}")).is_file())
    })
}

/// `PATHEXT` 决定无扩展名命令的可执行后缀，缺失时退回 Windows 的出厂值。
#[cfg(windows)]
fn windows_path_extensions() -> Vec<String> {
    std::env::var("PATHEXT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| {
            value
                .split(';')
                .filter(|item| !item.trim().is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_else(|| {
            [".COM", ".EXE", ".BAT", ".CMD"]
                .iter()
                .map(|extension| extension.to_string())
                .collect()
        })
}

#[cfg(all(test, windows))]
mod tests {
    use super::{windows_command_available, windows_path_extensions, windows_shell_candidates};

    #[test]
    fn prefers_powershell_before_comspec() {
        assert_eq!(
            windows_shell_candidates(Some(r"C:\Windows\System32\cmd.exe".to_string())),
            vec!["pwsh.exe", "powershell.exe", r"C:\Windows\System32\cmd.exe"]
        );
    }

    #[test]
    fn falls_back_to_cmd_when_comspec_is_missing() {
        assert_eq!(
            windows_shell_candidates(None),
            vec!["pwsh.exe", "powershell.exe", "cmd.exe"]
        );
    }

    /// PATH 查找取代了 `where.exe`，因此必须自证：Windows 上必然存在的 `cmd.exe`
    /// 要能找到，而明显不存在的名字不能误报。
    #[test]
    fn finds_a_command_on_path_without_spawning_a_process() {
        assert!(windows_command_available("cmd.exe"));
        assert!(!windows_command_available(
            "nocterm-definitely-not-a-real-shell.exe"
        ));
    }

    #[test]
    fn resolves_a_command_without_an_extension_via_pathext() {
        // 不写扩展名时要靠 PATHEXT 补全，否则 COMSPEC 被设成 `cmd` 就会判定不可用。
        assert!(windows_command_available("cmd"));
        assert!(
            windows_path_extensions()
                .iter()
                .any(|extension| extension.eq_ignore_ascii_case(".exe"))
        );
    }

    #[test]
    fn rejects_an_absolute_path_that_does_not_exist() {
        assert!(!windows_command_available(
            r"C:\nocterm-missing\powershell.exe"
        ));
    }
}
