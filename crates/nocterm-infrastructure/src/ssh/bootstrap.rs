//! SSH shell integration bootstrap 的纯逻辑：登录 shell 探针解析、spawn 预载
//! 计划（启动命令 + 临时文件内容）生成与上传/清理命令构造。
//!
//! 本模块不含任何网络与 tokio 代码，全部可无环境单测。
//! 网络编排见 `super::terminal::connect_session`。
//!
//! ## 设计决策：pty+exec spawn 预载，绝不向可见 PTY 敲入机制性命令
//!
//! 集成必须在远端 shell **启动时**随参数加载（业界通行做法：VS Code Remote /
//! JetBrains 由 server spawn 带预载的 shell，本地终端 `terminal/shell.rs` 同样是
//! --rcfile/ZDOTDIR/XDG 的 spawn 注入），而不是向已启动的可见 shell 敲入
//! `source`：后者必然产生回显——字节先于 shell 就绪到达时由内核行规程回显，
//! 之后由 readline/ZLE 自绘回显并重绘待处理输入。客户端文本匹配抑制已被真实
//! 环境证明不可靠（终端折行、ANSI 重绘、跨块到达均会泄漏，beta.3 实测回归）。
//!
//! 历史上偏离 spawn 方案的原因（本模块 v1 注释）是"exec bash --rcfile 会跳过
//! 登录链，破坏用户 PATH"。本版以**登录链复刻**解决：bash 在 rc 内按登录顺序
//! 显式 source /etc/profile 与首个命中的用户登录文件；zsh 用 ZDOTDIR 目录链式
//! 加载用户四件套后恢复原值；fish 用 `-C` 在用户配置之后加载钩子。实测
//! （Ubuntu 22.04 + OpenSSH 8.9）与真实登录的 PATH 逐字符一致。
//!
//! ## 登录会话显示补偿
//!
//! sshd 只对 shell 请求打印 MOTD 与 "Last login"（exec 请求跳过两者）；POSIX
//! rc 内容补偿：cat 动态/静态 motd，lastlog 可读时补 Last login 行。普通用户
//! 因 lastlog 文件权限（root:utmp 0664）无法补齐该行，是已确认接受的行为差异
//! （`who`/`last`/`lastlog` 记录由 sshd 对 pty+exec 会话照常写入，实测无缺失）。
//!
//! ## 架构不变量（评审基线）
//!
//! 可见 PTY 的输入只允许两类字节：用户键盘输入、已授权 AI 可见执行的命令本体。
//! 一切机制性内容（bootstrap、初始目录切换、marker）都必须走 spawn 预载或独立
//! exec channel，不得进入 PTY 输入路径。
//!
//! 安全约束（演进方案 §10.3）：
//! - 不修改用户持久 rc 文件；临时路径每次随机生成；
//! - 远端以 `umask 077` 创建临时文件（0600 权限）；
//! - rc 读取完成后自删除，上传中断或会话异常关闭由独立 channel 兜底清理；
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

/// 生成远程临时目录路径（zsh ZDOTDIR / fish rc），字符约束同 `remote_rc_path`。
pub fn remote_rc_dir(random_hex: &str, suffix: &str) -> String {
    format!("/tmp/nocterm-ssh-{random_hex}.{suffix}")
}

/// 上传临时文件的远端命令：umask 077 保证文件 0600；内容经 stdin 传入。
/// `mkdir -p` 兼容目录内文件（zsh ZDOTDIR 目录），已存在目录幂等。
pub(super) fn rc_upload_command(path: &str) -> String {
    let parent = path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("/tmp");
    format!(
        "umask 077 && mkdir -p {} && cat > {}",
        shell_single_quoted(parent),
        shell_single_quoted(path)
    )
}

/// 一次 spawn 预载需要的远端文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnFile {
    pub path: String,
    pub content: String,
}

/// spawn 预载计划：把集成加载前移到远端 shell 启动参数所需的全部素材。
///
/// `command` 通过独立 exec 请求在 pty 通道上启动 shell（不是敲入可见 PTY 的
/// 内容）；`files` 必须先于 spawn 上传完成。PowerShell 无临时文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnPlan {
    /// 关联的 integration 计划（nonce + shell），命令层据此注册控制面。
    pub plan: SshShellIntegrationPlan,
    /// pty+exec 请求携带的启动命令。
    pub command: String,
    /// 先于 spawn 上传的远端文件（按上传顺序）。
    pub files: Vec<SpawnFile>,
}

impl SpawnPlan {
    /// 需要兜底清理的远端路径（目录以 `rm -rf` 递归删除，文件按普通删除）。
    pub fn file_paths(&self) -> Vec<String> {
        self.files.iter().map(|file| file.path.clone()).collect()
    }
}

/// 依据 integration 计划与初始目录构造完整的 spawn 预载计划。
///
/// 各 shell 的启动命令与文件布局见对应 builder；`initial_path` 的目录切换属于
/// 机制性内容，同样只允许出现在 rc 内（POSIX）或编码脚本内（PowerShell），
/// 绝不敲入可见 PTY。
pub fn prepare_spawn(plan: SshShellIntegrationPlan, initial_path: Option<&str>) -> SpawnPlan {
    match plan.shell {
        SshShellKind::Bash => {
            let path = remote_rc_path(&plan.nonce);
            SpawnPlan {
                command: format!("exec \"$SHELL\" --rcfile {} -i", shell_single_quoted(&path)),
                files: vec![SpawnFile {
                    path,
                    content: bash_rc_content(&plan.nonce, initial_path),
                }],
                plan,
            }
        }
        SshShellKind::Zsh => {
            let dir = remote_rc_dir(&plan.nonce, "zsh");
            SpawnPlan {
                // 原始 ZDOTDIR 必须在 spawn 命令里由外层登录 shell 展开：进入
                // 预载 .zshrc 时 ZDOTDIR 已被重定向，在文件内部取 ${ZDOTDIR:-$HOME}
                // 会取到本目录并递归 source 自己（真机验证发现的回归）。
                command: format!(
                    "exec env NOCTERM_ORIG_ZDOTDIR=\"${{ZDOTDIR:-$HOME}}\" ZDOTDIR={} \"$SHELL\" -l -i",
                    shell_single_quoted(&dir)
                ),
                files: zsh_dir_files(&plan.nonce, &dir, initial_path),
                plan,
            }
        }
        SshShellKind::Fish => {
            let path = remote_rc_path(&plan.nonce);
            SpawnPlan {
                command: format!(
                    "exec env NOCTERM_OSC_NONCE={} \"$SHELL\" -l -C {}",
                    shell_single_quoted(&plan.nonce),
                    // -C 参数整体单引号；路径字符集受 remote_rc_path 约束，免转义。
                    shell_single_quoted(&format!("source {path}"))
                ),
                files: vec![SpawnFile {
                    path,
                    content: fish_rc_content(&plan.nonce, initial_path),
                }],
                plan,
            }
        }
        SshShellKind::PowerShell => {
            let script = pwsh_script_content(&plan.nonce, initial_path);
            SpawnPlan {
                command: format!(
                    "powershell.exe -NoLogo -NoExit -EncodedCommand {}",
                    utf16le_base64(&script)
                ),
                files: Vec::new(),
                plan,
            }
        }
    }
}

/// 登录信息补偿（POSIX 语法，bash/zsh 共用）：补显示 sshd 在 exec 路径下不再
/// 打印的 MOTD 与 Last login。lastlog 不可读（普通用户）时整段静默跳过。
fn motd_compensation_posix() -> String {
    let mut block = String::new();
    block.push_str("# —— 登录信息补偿：sshd 仅对 shell 请求打印 MOTD 与 Last login ——\n");
    block.push_str("cat /var/run/motd.dynamic 2>/dev/null\n");
    block.push_str("cat /etc/motd 2>/dev/null\n");
    block.push_str("_nocterm_ll=$(lastlog -u \"${USER:-$(id -un)}\" 2>/dev/null | tail -n +2)\n");
    block.push_str(
        "if [ -n \"$_nocterm_ll\" ] && ! printf '%s' \"$_nocterm_ll\" | grep -q 'Never logged in'; then\n",
    );
    block.push_str("  printf '%s\\n' \"$_nocterm_ll\" | awk '{printf \"Last login: %s %s %s %s %s from %s\\n\", $4, $5, $6, $7, $8, $3}'\n");
    block.push_str("fi\nunset _nocterm_ll\n");
    block
}

/// bash 预载 rc：登录信息补偿 → 登录链复刻 → nonce → 集成钩子 → 初始目录 → 自删除。
///
/// 登录链复刻与 bash 登录 shell 的读取顺序一致（/etc/profile 后取
/// .bash_profile/.bash_login/.profile 首个命中）；钩子必须在用户配置之后安装，
/// 保证用户的 PROMPT_COMMAND 定制先加载、由集成组合而非覆盖。
/// 自删除放在 rc 尾部：spawn 时守卫（NOCTERM_SHELL_INTEGRATION 未设置）不可能
/// 触发提前 return；即使触发，会话关闭路径仍有兜底清理。
fn bash_rc_content(nonce: &str, initial_path: Option<&str>) -> String {
    let mut rc = String::new();
    rc.push_str("# nocterm SSH spawn 预载 rc（会话级临时文件，读取完成后自删除）。\n");
    rc.push_str("# exec 启动的交互 bash 不再由 sshd 以登录方式拉起，登录链在此复刻。\n");
    rc.push_str(&motd_compensation_posix());
    rc.push_str("# —— 登录链复刻：/etc/profile → 首个命中的用户登录文件 ——\n");
    rc.push_str("if [ -f /etc/profile ]; then . /etc/profile; fi\n");
    rc.push_str(
        "for _nocterm_f in \"$HOME/.bash_profile\" \"$HOME/.bash_login\" \"$HOME/.profile\"; do\n",
    );
    rc.push_str("  if [ -f \"$_nocterm_f\" ]; then . \"$_nocterm_f\"; break; fi\n");
    rc.push_str("done\nunset _nocterm_f\n");
    rc.push_str(&format!(
        "export NOCTERM_OSC_NONCE={}\n",
        shell_single_quoted(nonce)
    ));
    rc.push_str(include_str!("../terminal/integration_bash.sh"));
    rc.push('\n');
    if let Some(directory) = initial_path {
        rc.push_str(&initial_directory_command(directory, None));
    }
    // 自删除放在 rc 尾部：spawn 时守卫（NOCTERM_SHELL_INTEGRATION 未设置）不可能
    // 触发提前 return；即使触发，会话关闭路径仍有兜底清理。
    rc.push_str(&format!(
        "rm -f -- {}\n",
        shell_single_quoted(&remote_rc_path(nonce))
    ));
    rc
}
/// zsh 预载目录三件套：.zshenv / .zprofile / .zshrc。
///
/// ZDOTDIR 重定向后 zsh 依次读取本目录的同名文件；每个文件先链式加载用户原始
/// 配置再装 nocterm 内容。原始 ZDOTDIR 由 spawn 命令通过 NOCTERM_ORIG_ZDOTDIR
/// 环境变量传入（见 `prepare_spawn`）；.zshrc 用它恢复后，zsh 读取的 .zlogin
/// 自然回落到用户真实文件，因此本目录无需 .zlogin。
fn zsh_dir_files(nonce: &str, dir: &str, initial_path: Option<&str>) -> Vec<SpawnFile> {
    let zshenv = "# nocterm SSH spawn 预载 .zshenv：链式加载用户原始 .zshenv。\n\
                  if [ -f \"$HOME/.zshenv\" ]; then . \"$HOME/.zshenv\"; fi\n";
    let zprofile = format!(
        "{}# —— 链式加载用户原始 .zprofile ——\n\
         if [ -f \"$HOME/.zprofile\" ]; then . \"$HOME/.zprofile\"; fi\n",
        motd_compensation_posix()
    );
    let mut zshrc = String::new();
    zshrc.push_str("# nocterm SSH spawn 预载 .zshrc：先加载用户原始 .zshrc，再装 nocterm 钩子。\n");
    zshrc.push_str(
        "# NOCTERM_ORIG_ZDOTDIR 由 spawn 命令捕获（外层 shell 展开 ${ZDOTDIR:-$HOME}），\n",
    );
    zshrc.push_str("# 恢复后嵌套 zsh 与 .zlogin 都回落到用户真实文件。\n");
    // 原始值由 spawn 命令传入（此刻 ZDOTDIR 已指向本目录，文件内部取值会自指递归）；
    // 兜底 ${NOCTERM_ORIG_ZDOTDIR:-$HOME} 覆盖环境变量缺失的异常场景。
    zshrc.push_str("NOCTERM_ORIG_ZDOTDIR=\"${NOCTERM_ORIG_ZDOTDIR:-$HOME}\"\n");
    zshrc.push_str(
        "if [ -f \"$NOCTERM_ORIG_ZDOTDIR/.zshrc\" ]; then . \"$NOCTERM_ORIG_ZDOTDIR/.zshrc\"; fi\n",
    );
    zshrc.push_str("ZDOTDIR=\"$NOCTERM_ORIG_ZDOTDIR\"\n");
    zshrc.push_str("unset NOCTERM_ORIG_ZDOTDIR\n");
    zshrc.push_str(&format!(
        "export NOCTERM_OSC_NONCE={}\n",
        shell_single_quoted(nonce)
    ));
    zshrc.push_str(include_str!("../terminal/integration_zsh.zsh"));
    zshrc.push('\n');
    if let Some(directory) = initial_path {
        zshrc.push_str(&initial_directory_command(directory, None));
    }
    // 自删除放 .zshrc 尾部：此后 zsh 不再读取本目录（.zlogin 已回落 $HOME）。
    zshrc.push_str(&format!("rm -rf -- {}\n", shell_single_quoted(dir)));
    vec![
        SpawnFile {
            path: format!("{dir}/.zshenv"),
            content: zshenv.to_string(),
        },
        SpawnFile {
            path: format!("{dir}/.zprofile"),
            content: zprofile,
        },
        SpawnFile {
            path: format!("{dir}/.zshrc"),
            content: zshrc,
        },
    ]
}

/// fish 预载 rc：借 `fish -C 'source <rc>'` 在用户配置之后加载，无需 XDG 重定向
/// 与登录链复刻（fish 自身配置链不受 exec 启动影响）。
///
/// rc 内仅承载：登录信息补偿 → nonce → 集成钩子 → 初始目录 → 自删除。
/// `-C` 相对 config.fish 的确切时序是真机验收项；钩子为事件处理器，用户配置
/// 覆盖风险低，失败时按无集成降级。
fn fish_rc_content(nonce: &str, initial_path: Option<&str>) -> String {
    let mut rc = String::new();
    rc.push_str("# nocterm SSH spawn 预载 rc（fish -C 加载，会话级临时文件，读取后自删除）。\n");
    rc.push_str("# —— 登录信息补偿：sshd 仅对 shell 请求打印 MOTD 与 Last login ——\n");
    rc.push_str("cat /var/run/motd.dynamic 2>/dev/null\n");
    rc.push_str("cat /etc/motd 2>/dev/null\n");
    rc.push_str("set -l _nocterm_ll (lastlog -u (id -un) 2>/dev/null | tail -n +2)\n");
    rc.push_str("if test -n \"$_nocterm_ll\"; and not string match -q -- '*Never logged in*' $_nocterm_ll\n");
    rc.push_str("    echo $_nocterm_ll | awk '{printf \"Last login: %s %s %s %s %s from %s\\n\", $4, $5, $6, $7, $8, $3}'\n");
    rc.push_str("end\n");
    rc.push_str(&format!(
        "set -gx NOCTERM_OSC_NONCE {}\n",
        shell_single_quoted(nonce)
    ));
    rc.push_str(include_str!("../terminal/integration_fish.fish"));
    rc.push('\n');
    if let Some(directory) = initial_path {
        rc.push_str(&initial_directory_command(directory, None));
    }
    rc
}

/// PowerShell 预载脚本：nonce → 集成钩子 → 初始目录。整体经 UTF-16LE + Base64
/// 编码为 `-EncodedCommand`，不创建远端临时文件。
fn pwsh_script_content(nonce: &str, initial_path: Option<&str>) -> String {
    let mut script = format!("$env:NOCTERM_OSC_NONCE = '{}'\n", nonce);
    script.push_str(include_str!("../terminal/integration_pwsh.ps1"));
    if let Some(directory) = initial_path {
        script.push_str(&initial_directory_command(
            directory,
            Some(SshShellKind::PowerShell),
        ));
    }
    script
}

/// 初始目录切换必须由远端 Shell 自己解释，不能用 POSIX 引号拼接 Windows 路径。
/// 仅用于 spawn 预载 rc 与降级路径的 activate 写入，不进入其他输入通道。
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

/// 阶段 2：在独立 channel 上以 `umask 077 && mkdir -p && cat > <path>` 上传文件。
///
/// 内容经 channel stdin 写入（免转义）；EOF 后等待远端收尾，任何失败都返回 Err。
pub(super) async fn upload_file(
    handle: &russh::client::Handle<super::connection::ClientHandler>,
    path: &str,
    content: &[u8],
) -> Result<(), &'static str> {
    eprintln!(
        "[nocterm-ssh-bootstrap] 阶段 2：上传临时文件（{path}，{} 字节）…",
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

/// 上传或 spawn 失败、会话异常关闭时的兜底清理：尽力删除预载临时文件
/// （目录递归）。全部是本会话随机生成的 /tmp/nocterm-ssh-* 路径，不含用户数据；
/// 正常路径下 rc 读取后已自删除，这里的 rm 对不存在的路径是幂等 no-op。
pub(super) async fn cleanup_files(
    handle: &russh::client::Handle<super::connection::ClientHandler>,
    paths: &[String],
) {
    for path in paths {
        let channel = match handle.channel_open_session().await {
            Ok(channel) => channel,
            Err(_) => return,
        };
        let command = format!("rm -rf -- {}", shell_single_quoted(path));
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
}

/// 探针短窗超时；整体仍受 CONNECT_READY_TIMEOUT 兜底约束。
pub(super) const SHELL_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// 临时文件上传超时；umask 077 + 单文件写入，正常在 RTT 内完成。
pub(super) const RC_UPLOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// PowerShell `-EncodedCommand` 只接受 UTF-16LE；脚本先转 UTF-16LE 字节序列
/// 再走标准 Base64。内容不是秘密，编码只解决跨 Shell 的引号与换行传输。
fn utf16le_base64(script: &str) -> String {
    let mut utf16 = Vec::with_capacity(script.len() * 2);
    for unit in script.encode_utf16() {
        utf16.extend_from_slice(&unit.to_le_bytes());
    }
    base64_encode(&utf16)
}

/// 标准 Base64 编码用于把 PowerShell integration 以内存脚本注入远端。
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
        let bash = prepare_spawn(plan(SshShellKind::Bash), None);
        let zsh = prepare_spawn(plan(SshShellKind::Zsh), None);
        for spawn in [bash, zsh] {
            for file in &spawn.files {
                for line in file.content.lines() {
                    let trimmed = line.trim_start();
                    if trimmed.starts_with("builtin __nocterm") {
                        panic!(
                            "{} uses builtin prefix on a custom function: {line}",
                            spawn.plan.shell.as_str()
                        );
                    }
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
        for spawn in [
            prepare_spawn(plan(SshShellKind::Bash), None),
            prepare_spawn(plan(SshShellKind::Zsh), None),
            prepare_spawn(plan(SshShellKind::Fish), None),
        ] {
            // zsh 计划含 .zshenv/.zprofile 等无 emit 的链式文件；契约只约束
            // 携带集成脚本的文件。
            for file in spawn
                .files
                .iter()
                .filter(|file| file.content.contains("__nocterm_emit"))
            {
                // 每个脚本都必须先拼"事件名;事件体"再追加 nonce，而不是固定三参数。
                let emit_body = file
                    .content
                    .find("local fields=\"$1\"")
                    .or_else(|| file.content.find("set -l fields $argv[1]"))
                    .unwrap_or_else(|| {
                        panic!(
                            "{} emit must build fields incrementally",
                            spawn.plan.shell.as_str()
                        )
                    });
                let three_arg_printf = file.content.contains("printf '\\033]633;%s;%s;%s\\007'");
                assert!(
                    !three_arg_printf,
                    "{} uses a fixed three-field printf that emits empty-body placeholders",
                    spawn.plan.shell.as_str()
                );
                assert!(emit_body > 0);
            }
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
    fn fish_rc_exports_nonce_and_installs_hooks_without_user_config_chaining() {
        let spawn = prepare_spawn(plan(SshShellKind::Fish), None);
        assert_eq!(spawn.files.len(), 1);
        let content = &spawn.files[0].content;

        assert!(content.contains("set -gx NOCTERM_OSC_NONCE 'abc123'"));
        assert!(content.contains("--on-event fish_postexec"));
        assert!(!content.contains("export NOCTERM_OSC_NONCE"));
        // fish 走 -C 加载：用户配置链由 fish 自己执行，rc 不做 XDG 重定向与链式 source。
        assert!(!content.contains("XDG_CONFIG_HOME"));
        assert!(!content.contains(".config/fish/config.fish"));
    }

    #[test]
    fn fish_spawn_uses_init_command_and_never_touches_visible_pty() {
        let spawn = prepare_spawn(plan(SshShellKind::Fish), Some("/srv/it's"));
        assert_eq!(
            spawn.command,
            "exec env NOCTERM_OSC_NONCE='abc123' \"$SHELL\" -l -C 'source /tmp/nocterm-ssh-abc123.rc'"
        );
        // 初始目录在 rc 内部执行，绝不追加到可见 PTY 输入。
        assert!(spawn.files[0].content.contains("cd '/srv/it'\\''s'"));
    }

    #[test]
    fn powershell_uses_encoded_command_without_temp_files() {
        let spawn = prepare_spawn(plan(SshShellKind::PowerShell), Some("C:\\Users\\O'Brien"));
        assert!(spawn.files.is_empty());
        assert!(
            spawn
                .command
                .starts_with("powershell.exe -NoLogo -NoExit -EncodedCommand ")
        );
        assert!(!spawn.command.contains("/tmp/"));
        // Base64 字母表不含路径与引号字符；nonce 与目录只出现在编码内容里。
        assert!(
            spawn
                .command
                .chars()
                .skip("powershell.exe -NoLogo -NoExit -EncodedCommand ".len())
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
        );
    }

    #[test]
    fn initial_directory_command_uses_shell_specific_literal_quoting() {
        assert_eq!(
            initial_directory_command("C:\\Users\\O'Brien", Some(SshShellKind::PowerShell)),
            "Set-Location -LiteralPath 'C:\\Users\\O''Brien'\r\n"
        );
        assert_eq!(
            initial_directory_command("/srv/it's", None),
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
        let file = remote_rc_path("aabbccdd00112233");
        let dir = remote_rc_dir("aabbccdd00112233", "zsh");
        assert!(file.starts_with("/tmp/nocterm-ssh-"));
        assert!(file.ends_with(".rc"));
        assert!(dir.starts_with("/tmp/nocterm-ssh-"));
        assert!(dir.ends_with(".zsh"));
        for path in [file, dir] {
            assert!(
                path.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '.'))
            );
        }
    }

    #[test]
    fn upload_command_creates_parent_with_restrictive_umask() {
        let command = rc_upload_command("/tmp/nocterm-ssh-x.zsh/.zshrc");
        assert!(command.starts_with("umask 077 && mkdir -p '/tmp/nocterm-ssh-x.zsh' && cat > "));
        assert!(command.ends_with("'/tmp/nocterm-ssh-x.zsh/.zshrc'"));
    }

    /// bash 预载 rc 的顺序契约：MOTD 补偿 → 登录链复刻 → nonce → 钩子 → 初始
    /// 目录 →（尾部由测试断言 rm）。钩子必须在用户配置之后安装。
    #[test]
    fn bash_rc_orders_compensation_chain_nonce_hooks_and_directory() {
        let spawn = prepare_spawn(plan(SshShellKind::Bash), Some("/srv/it's"));
        let rc = &spawn.files[0].content;

        let motd = rc.find("登录信息补偿").expect("motd compensation");
        let chain = rc.find("/etc/profile").expect("login chain");
        let nonce = rc
            .find("export NOCTERM_OSC_NONCE='abc123'")
            .expect("nonce export");
        let hooks = rc.find("PROMPT_COMMAND=").expect("bash hooks");
        let cd = rc.find("cd '/srv/it'\\''s'").expect("initial directory");
        assert!(motd < chain && chain < nonce && nonce < hooks && hooks < cd);
        // 自删除责任回到 rc 尾部（spawn 时守卫不可能提前 return）。
        assert!(rc.ends_with(&format!("rm -f -- '{}'\n", remote_rc_path("abc123"))));
    }

    /// zsh 三件套契约：.zshenv 链式加载用户配置；.zprofile 补偿 + 链式；
    /// .zshrc 先链式再恢复 ZDOTDIR 再装钩子，尾部自删除目录。
    #[test]
    fn zsh_dir_files_chain_user_config_and_restore_zdotdir() {
        let spawn = prepare_spawn(plan(SshShellKind::Zsh), None);
        assert_eq!(spawn.files.len(), 3);
        // 原始 ZDOTDIR 由外层 shell 展开（文件内部取值会自指递归，真机回归）。
        assert!(
            spawn
                .command
                .contains("exec env NOCTERM_ORIG_ZDOTDIR=\"${ZDOTDIR:-$HOME}\" ZDOTDIR='/tmp/nocterm-ssh-abc123.zsh'")
        );
        assert!(spawn.command.contains("\"$SHELL\" -l -i"));

        let zshenv = &spawn.files[0];
        assert!(zshenv.path.ends_with("/.zshenv"));
        assert!(zshenv.content.contains(". \"$HOME/.zshenv\""));

        let zprofile = &spawn.files[1];
        assert!(zprofile.path.ends_with("/.zprofile"));
        let motd = zprofile.content.find("登录信息补偿").expect("motd");
        let chain = zprofile
            .content
            .find(". \"$HOME/.zprofile\"")
            .expect("chain");
        assert!(motd < chain);

        let zshrc = &spawn.files[2];
        assert!(zshrc.path.ends_with("/.zshrc"));
        let chain = zshrc
            .content
            .find("$NOCTERM_ORIG_ZDOTDIR/.zshrc")
            .expect("chain");
        let restore = zshrc
            .content
            .find("ZDOTDIR=\"$NOCTERM_ORIG_ZDOTDIR\"")
            .expect("restore");
        let nonce = zshrc
            .content
            .find("export NOCTERM_OSC_NONCE='abc123'")
            .expect("nonce");
        let hooks = zshrc.content.find("precmd_functions+=(").expect("hooks");
        let rm = zshrc
            .content
            .find("rm -rf -- '/tmp/nocterm-ssh-abc123.zsh'")
            .expect("self delete");
        assert!(chain < restore && restore < nonce && nonce < hooks && hooks < rm);
        // 不重建登录链之外的用户文件；.zlogin 靠 ZDOTDIR 恢复自然回落，
        // 本目录不放置也不 source 它。
        assert!(!zshrc.content.contains(". \"$HOME/.zlogin\""));
    }

    #[test]
    fn spawn_plan_paths_cover_all_files_for_cleanup() {
        let zsh = prepare_spawn(plan(SshShellKind::Zsh), None);
        let paths = zsh.file_paths();
        assert_eq!(paths.len(), 3);
        assert!(
            paths
                .iter()
                .all(|path| path.starts_with("/tmp/nocterm-ssh-abc123.zsh/"))
        );

        let pwsh = prepare_spawn(plan(SshShellKind::PowerShell), None);
        assert!(pwsh.file_paths().is_empty());
    }
}
