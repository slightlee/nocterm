use super::*;

fn fixture() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "nocterm-local-edit-{}",
        super::super::unique_suffix().unwrap()
    ));
    fs::create_dir(&root).unwrap();
    root
}

#[test]
fn local_conflicts_keep_external_content_and_force_only_skips_the_conflict() {
    let root = fixture();
    let target = root.join("config");
    fs::write(&target, b"old").unwrap();
    let adapter = LocalTextFiles;
    let original = adapter.read(target.to_str().unwrap(), 1024).unwrap();
    fs::write(&target, b"external version").unwrap();
    let error = adapter
        .write(target.to_str().unwrap(), b"new", original.version, false)
        .unwrap_err();
    assert_eq!(error.code, "SFTP_LOCAL_FILE_CHANGED");
    assert_eq!(fs::read(&target).unwrap(), b"external version");
    let saved = adapter
        .write(target.to_str().unwrap(), b"new", original.version, true)
        .unwrap();
    assert_eq!(saved.size, Some(3));
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn changed_backup_is_restored_before_commit_is_rejected() {
    let root = fixture();
    let target = root.join("config");
    let backup = root.join("backup");
    fs::write(&target, b"old").unwrap();
    let version = LocalTextFiles
        .read(target.to_str().unwrap(), 1024)
        .unwrap()
        .version;
    fs::rename(&target, &backup).unwrap();
    fs::write(&backup, b"external version").unwrap();
    let error = verify_local_backup_snapshot(
        &backup,
        &target,
        LocalCommitExpectation {
            size: version.size,
            modified_seconds: version.modified_seconds,
            modified_nanos: version.modified_nanos,
            force: false,
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
    assert_eq!(fs::read(&target).unwrap(), b"external version");
    assert!(!backup.exists());
    fs::remove_dir_all(root).unwrap();
}
