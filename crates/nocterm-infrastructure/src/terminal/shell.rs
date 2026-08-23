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

/// Windows 的系统默认 Shell 通常是 CMD，但 PowerShell 更适合作为客户端默认终端。
/// 候选按现代 PowerShell、系统 PowerShell、用户配置和最后兜底的顺序选择。
pub(super) fn local_shell_command() -> CommandBuilder {
    #[cfg(windows)]
    {
        for shell in windows_shell_candidates(std::env::var("COMSPEC").ok()).iter() {
            if windows_command_available(shell) {
                return CommandBuilder::new(shell);
            }
        }
        CommandBuilder::new("cmd.exe")
    }

    // macOS 与其它 Unix 走 portable-pty 的默认程序解析（读 `SHELL`，回落到 passwd 项）。
    #[cfg(not(windows))]
    {
        CommandBuilder::new_default_prog()
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
