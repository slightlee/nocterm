//! SFTP 远程路径拼接与本地写入安全校验。
//!
//! 远端服务端返回的条目名是**不可信输入**：拼进本地路径前必须校验，拼进远程
//! 路径时必须规避分隔符歧义。判定交给 `std::path` 的组件解析，按各平台真实
//! 语义判断，而不是手写分隔符黑名单。

use std::path::{Component, Path};

use super::error::SftpError;

/// 拼接远程路径：以 `/` 分隔，规避根目录下产生重复分隔符。
pub(super) fn join_remote(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// 校验远端目录条目名能否安全地拼进本地路径。
///
/// 远端 `read_dir` 返回的名字是**不可信输入**：`PathBuf::join` 见到 `..` 会向上跳级，
/// 在 Windows 上还会把 `\` 和盘符前缀当成路径语法，异常或恶意的服务端因此能让递归
/// 下载写到目标目录之外。OpenSSH 的 `sftp` 与 WinSCP 同样在客户端侧拦这类条目名。
///
/// 判定刻意交给 `std::path` 的组件解析而不是手写分隔符黑名单：`\` 在 Windows 上是
/// 分隔符、在 Unix 上只是普通文件名字符，按各自平台的真实语义判断才不会在 macOS 上
/// 误拒合法文件名。
pub(super) fn ensure_safe_child_name(name: &str) -> Result<(), SftpError> {
    let reject = |reason: &str| {
        Err(SftpError::new(
            "invalidPath",
            format!("远端条目名 {name} {reason}，已拒绝下载以免写出目标目录"),
        ))
    };
    // NUL 无法进入任何平台的文件名，提前拦下比让 OS 报一句晦涩的 io 错误清楚。
    if name.is_empty() || name.contains('\0') {
        return reject("为空或含有 NUL 字节");
    }
    let mut components = Path::new(name).components();
    // 合法条目名解析后必须恰好是一个普通组件：`..` 会解析成 ParentDir、`.` 解析成
    // CurDir、带分隔符或盘符的名字会解析出两个以上组件或 Prefix。
    if !matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(_)), None)
    ) {
        return reject("含有路径分隔符、盘符或上跳片段");
    }
    // Windows 的冒号是备用数据流语法：下载 `notes.txt:secret` 会静默写进
    // `notes.txt` 的隐藏流，资源管理器里看不到任何新文件。Unix 上冒号只是普通字符，
    // 因此这条检查按平台生效而不是一律拒绝。
    #[cfg(windows)]
    if name.contains(':') {
        return reject("含有冒号，Windows 会把它当作备用数据流");
    }
    Ok(())
}
