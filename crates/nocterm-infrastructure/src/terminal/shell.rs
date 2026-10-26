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
                let kind = detect_shell_kind(shell);
                let mut command = apply_terminal_identity(CommandBuilder::new(shell));
                if matches!(kind, LocalShellKind::PowerShell) {
                    // PowerShell 家族注入集成脚本：用户 Profile 先于 -EncodedCommand
                    // 加载，脚本包装 prompt 以 OSC 633 上报命令完成与退出码；
                    // 序列属于控制平面，不受终端折行影响（#82）。
                    command.arg("-NoLogo");
                    command.arg("-NoExit");
                    command.arg("-EncodedCommand");
                    command.arg(encoded_integration_command());
                }
                return (command, kind);
            }
        }
        (
            apply_terminal_identity(CommandBuilder::new("cmd.exe")),
            LocalShellKind::Cmd,
        )
    }

    // macOS 与其它 Unix 走 portable-pty 的默认程序解析（读 `SHELL`，回落到 passwd 项）。
    // 各 shell 按自身机制注入集成脚本：zsh 临时 ZDOTDIR、bash --rcfile、fish conf.d。
    // 注入失败或未覆盖的 shell 静默降级到既有探针路径（#82）。
    #[cfg(not(windows))]
    {
        let command =
            apply_default_term(apply_terminal_identity(CommandBuilder::new_default_prog()));
        let shell_path = command.get_shell();
        let kind = detect_shell_kind(&shell_path);
        let mut command = command;
        match kind {
            LocalShellKind::Posix if shell_path.ends_with("zsh") => {
                apply_zsh_integration(&mut command);
            }
            LocalShellKind::Posix if shell_path.ends_with("bash") => {
                apply_bash_integration(&mut command);
            }
            LocalShellKind::Fish => {
                apply_fish_integration(&mut command);
            }
            _ => {}
        }
        (command, kind)
    }
}

/// zsh 集成注入（#82）：构造临时 ZDOTDIR，其中的 `.zshrc` 先 source 用户原始
/// 配置（`$ZDOTDIR/.zshrc` 或 `~/.zshrc`），再加载 Nocterm 的 precmd 钩子脚本。
/// zsh 按 `$ZDOTDIR/.zshrc` 优先于 `~/.zshrc` 的规则加载包装，用户配置与环境
/// 不被破坏。目录常驻 `std::env::temp_dir()`，脚本内容以 `include_str!` 编译期
/// 固化；ZDOTDIR 原值保存进包装脚本，zsh 启动时恢复使用。
#[cfg(not(windows))]
fn apply_zsh_integration(command: &mut CommandBuilder) {
    let dir = std::env::temp_dir().join("nocterm-zsh-integration");
    // 并发/重复启动幂等：目录与文件内容固定，失败（多用户临时目录冲突等）时
    // 静默放弃注入，zsh 以无集成方式启动，回退探针路径接管。
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let wrapper = build_zsh_wrapper(
        command
            .get_env("ZDOTDIR")
            .map(|value| value.to_string_lossy().into_owned()),
    );
    if std::fs::write(dir.join(".zshrc"), wrapper).is_err() {
        return;
    }
    command.env("ZDOTDIR", dir);
}

/// bash 集成注入（#82）：构造临时 `--rcfile` 包装脚本，先 source 用户原始
/// `.bashrc`（若存在），再加载 Nocterm 的 PROMPT_COMMAND 钩子。文件常驻临时目录，
/// 内容固定，幂等；写入失败静默降级到无集成启动。
#[cfg(not(windows))]
fn apply_bash_integration(command: &mut CommandBuilder) {
    let path = std::env::temp_dir().join("nocterm-bash-integration");
    if std::fs::create_dir_all(path.parent().unwrap_or(&std::env::temp_dir())).is_err() {
        return;
    }
    let user_bashrc = command
        .get_env("HOME")
        .map(|home| format!("{}/.bashrc", home.to_string_lossy()))
        .unwrap_or_else(|| "$HOME/.bashrc".to_string());
    let integration = include_str!("integration_bash.sh");
    let wrapper =
        format!("if [ -f \"{user_bashrc}\" ]; then\n  source \"{user_bashrc}\"\nfi\n{integration}");
    if std::fs::write(&path, wrapper).is_err() {
        return;
    }
    command.arg("--rcfile");
    command.arg(path);
}

/// fish 集成注入（#82）：fish 按 `XDG_CONFIG_HOME/fish/conf.d/*.fish` 顺序加载
/// 启动脚本；构造临时 conf.d 目录放置 Nocterm 钩子，再以 `XDG_CONFIG_HOME`
/// 指向临时配置根。用户原配置（若存在）先于本目录被 fish 加载。
#[cfg(not(windows))]
fn apply_fish_integration(command: &mut CommandBuilder) {
    let root = std::env::temp_dir().join("nocterm-fish-integration");
    let conf_d = root.join("fish").join("conf.d");
    if std::fs::create_dir_all(&conf_d).is_err() {
        return;
    }
    let integration = include_str!("integration_fish.fish");
    if std::fs::write(conf_d.join("nocterm_integration.fish"), integration).is_err() {
        return;
    }
    command.env("XDG_CONFIG_HOME", root);
}

/// 生成 ZDOTDIR 包装 `.zshrc`：先加载用户原始配置，再加载 Nocterm 集成钩子。
#[cfg(not(windows))]
fn build_zsh_wrapper(original_zdotdir: Option<String>) -> String {
    // 用户原始 ZDOTDIR 需要在包装生效期间恢复：zsh 读包装 .zshrc 时 ZDOTDIR 仍
    // 指向临时目录，source 用户配置前必须改回原值（未设置则回到 $HOME）。
    let restore = match original_zdotdir {
        Some(dir) => format!("NOCTERM_ORIG_ZDOTDIR={}", shell_single_quoted(&dir)),
        None => "NOCTERM_ORIG_ZDOTDIR=".to_string(),
    };
    let integration = include_str!("integration_zsh.zsh");
    format!(
        "{restore}\n\
         if [[ -n \"$NOCTERM_ORIG_ZDOTDIR\" ]] && [[ -f \"$NOCTERM_ORIG_ZDOTDIR/.zshrc\" ]]; then\n\
         \x20 source \"$NOCTERM_ORIG_ZDOTDIR/.zshrc\"\n\
         elif [[ -f \"$HOME/.zshrc\" ]]; then\n\
         \x20 source \"$HOME/.zshrc\"\n\
         fi\n\
         ZDOTDIR=\"$NOCTERM_ORIG_ZDOTDIR\"\n\
         unset NOCTERM_ORIG_ZDOTDIR\n\
         {integration}"
    )
}

/// POSIX 单引号转义：内容中的单引号按 `'\''` 规则闭合再开。
#[cfg(not(windows))]
fn shell_single_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(all(test, not(windows)))]
mod zsh_integration_tests {
    use super::{build_zsh_wrapper, shell_single_quoted};

    #[test]
    fn wrapper_sources_user_config_before_integration_hooks() {
        let wrapper = build_zsh_wrapper(Some("/custom/zdotdir".to_string()));
        // 用户配置先于 Nocterm 钩子：保证原 prompt/插件完整加载后再包装。
        let user_source = wrapper
            .find("source \"$NOCTERM_ORIG_ZDOTDIR/.zshrc\"")
            .expect("wrapper must source the original ZDOTDIR config");
        let hooks = wrapper
            .find("precmd()")
            .expect("wrapper must install hooks");
        assert!(user_source < hooks);
        // 原始 ZDOTDIR 在 source 前被单引号转义保存。
        assert!(wrapper.contains("NOCTERM_ORIG_ZDOTDIR='/custom/zdotdir'"));
    }

    #[test]
    fn wrapper_falls_back_to_home_config_without_original_zdotdir() {
        let wrapper = build_zsh_wrapper(None);
        assert!(wrapper.contains("source \"$HOME/.zshrc\""));
        assert!(!wrapper.contains("NOCTERM_ORIG_ZDOTDIR='."));
    }

    #[test]
    fn single_quotes_escape_embedded_quotes() {
        assert_eq!(shell_single_quoted("it's"), "'it'\\''s'");
        assert_eq!(shell_single_quoted("plain"), "'plain'");
    }
}

/// 集成脚本按 PowerShell 约定编码为 UTF-16LE + Base64（`-EncodedCommand` 只接受该形式）。
#[cfg(windows)]
fn encoded_integration_command() -> String {
    let script = include_str!("integration_pwsh.ps1");
    let mut utf16 = Vec::with_capacity(script.len() * 2);
    for unit in script.encode_utf16() {
        utf16.extend_from_slice(&unit.to_le_bytes());
    }
    base64_encode(&utf16)
}

/// 标准字母表 Base64 编码（含填充）。仅为注入脚本而存在，不值得引入依赖。
#[cfg(windows)]
fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let byte = |index: usize| -> u32 { u32::from(chunk[index]) };
        let group = (byte(0) << 16)
            | (chunk.get(1).map_or(0, |_| byte(1)) << 8)
            | chunk.get(2).map_or(0, |_| byte(2));
        out.push(ALPHABET[(group >> 18) as usize & 0x3f] as char);
        out.push(ALPHABET[(group >> 12) as usize & 0x3f] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(group >> 6) as usize & 0x3f] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[group as usize & 0x3f] as char
        } else {
            '='
        });
    }
    out
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

#[cfg(all(test, windows))]
mod integration_encoding_tests {
    use super::{base64_encode, encoded_integration_command};

    #[test]
    fn base64_matches_standard_alphabet_with_padding() {
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"M"), "TQ==");
        assert_eq!(base64_encode(&[0x41, 0x00]), "QQA=");
    }

    #[test]
    fn encoded_integration_command_uses_base64_alphabet() {
        // 解码验证由真实 Shell 注入路径覆盖；此处守住非空与字母表约束，
        // 防止编码器悄悄混入其它字节。
        let encoded = encoded_integration_command();
        assert!(!encoded.is_empty());
        assert!(
            encoded
                .chars()
                .all(|character| character.is_ascii_alphanumeric()
                    || character == '+'
                    || character == '/'
                    || character == '=')
        );
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
