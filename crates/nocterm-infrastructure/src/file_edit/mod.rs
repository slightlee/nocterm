//! 文件编辑的平台 Adapter；敏感内容只写入创建时即受保护的暂存资源。
mod local;
mod remote;
pub use local::LocalTextFiles;
pub use remote::RemoteTextFiles;

use std::{
    fs::{File, OpenOptions},
    io,
    path::Path,
};

/// 随机且排他创建的名字避免其它进程预先布置路径；熵源失败时直接拒绝保存。
pub(crate) fn unique_suffix() -> io::Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(io::Error::other)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(crate) fn private_file(path: &Path) -> io::Result<File> {
    // 不对既存路径执行“创建或截断”；发生碰撞时失败，让调用方保留原文件。
    // 权限随创建一并设置，后续复制原 mode 只影响最终提交文件。
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    // Windows 新文件继承用户目录 ACL；Unix 从创建时起即限制为当前用户读写。
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn staging_is_exclusive_and_private_before_content_is_written() {
        let path =
            std::env::temp_dir().join(format!("nocterm-private-{}", unique_suffix().unwrap()));
        let file = private_file(&path).unwrap();
        assert!(private_file(&path).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o600);
        }
        drop(file);
        std::fs::remove_file(path).unwrap();
    }
}
