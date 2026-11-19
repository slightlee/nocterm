//! 在内存 SFTP 服务端验证保存协议与失败路径，不访问用户服务器或磁盘文件。
use super::*;
use russh_sftp::{
    protocol::{Attrs, Handle, Status, StatusCode},
    server::Handler,
};
use std::{collections::HashMap, sync::Mutex};

#[derive(Clone)]
struct File {
    bytes: Vec<u8>,
    attrs: FileAttributes,
}
#[derive(Default)]
struct ServerState {
    files: HashMap<String, File>,
    opened_modes: Vec<Option<u32>>,
    exclusive_opens: Vec<bool>,
    fail_commit: bool,
    change_during_write: bool,
}
struct Server(Arc<Mutex<ServerState>>);
fn ok(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: String::new(),
        language_tag: String::new(),
    }
}

impl Handler for Server {
    type Error = StatusCode;
    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }
    async fn open(
        &mut self,
        id: u32,
        filename: String,
        flags: OpenFlags,
        attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        let mut state = self.0.lock().unwrap();
        if flags.contains(OpenFlags::EXCLUDE) && state.files.contains_key(&filename) {
            return Err(StatusCode::Failure);
        }
        state.opened_modes.push(attrs.permissions);
        state
            .exclusive_opens
            .push(flags.contains(OpenFlags::EXCLUDE));
        state.files.insert(
            filename.clone(),
            File {
                bytes: Vec::new(),
                attrs: FileAttributes {
                    size: Some(0),
                    mtime: Some(2),
                    ..attrs
                },
            },
        );
        Ok(Handle {
            id,
            handle: filename,
        })
    }
    async fn close(&mut self, id: u32, _: String) -> Result<Status, Self::Error> {
        Ok(ok(id))
    }
    async fn write(
        &mut self,
        id: u32,
        path: String,
        offset: u64,
        bytes: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        let mut state = self.0.lock().unwrap();
        let file = state.files.get_mut(&path).ok_or(StatusCode::NoSuchFile)?;
        file.bytes.resize(offset as usize + bytes.len(), 0);
        file.bytes[offset as usize..].copy_from_slice(&bytes);
        file.attrs.size = Some(file.bytes.len() as u64);
        if state.change_during_write {
            let target = state.files.get_mut("/config").unwrap();
            target.bytes = b"external update".to_vec();
            target.attrs.size = Some(target.bytes.len() as u64);
            target.attrs.mtime = Some(3);
        }
        Ok(ok(id))
    }
    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let state = self.0.lock().unwrap();
        Ok(Attrs {
            id,
            attrs: state
                .files
                .get(&path)
                .ok_or(StatusCode::NoSuchFile)?
                .attrs
                .clone(),
        })
    }
    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        self.stat(id, path).await
    }
    async fn setstat(
        &mut self,
        id: u32,
        path: String,
        attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        self.0
            .lock()
            .unwrap()
            .files
            .get_mut(&path)
            .ok_or(StatusCode::NoSuchFile)?
            .attrs
            .permissions = attrs.permissions;
        Ok(ok(id))
    }
    async fn rename(&mut self, id: u32, old: String, new: String) -> Result<Status, Self::Error> {
        let mut state = self.0.lock().unwrap();
        if state.fail_commit && old.ends_with(".part") && new == "/config" {
            return Err(StatusCode::PermissionDenied);
        }
        if state.files.contains_key(&new) {
            return Err(StatusCode::Failure);
        }
        let file = state.files.remove(&old).ok_or(StatusCode::NoSuchFile)?;
        state.files.insert(new, file);
        Ok(ok(id))
    }
    async fn remove(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
        self.0
            .lock()
            .unwrap()
            .files
            .remove(&path)
            .ok_or(StatusCode::NoSuchFile)?;
        Ok(ok(id))
    }
}

async fn server() -> (
    SftpSession,
    Arc<Mutex<ServerState>>,
    tokio::task::JoinHandle<()>,
) {
    let state = Arc::new(Mutex::new(ServerState::default()));
    state.lock().unwrap().files.insert(
        "/config".into(),
        File {
            bytes: b"old".to_vec(),
            attrs: FileAttributes {
                size: Some(3),
                mtime: Some(1),
                permissions: Some(0o100600),
                ..FileAttributes::default()
            },
        },
    );
    let (client, peer) = tokio::io::duplex(1024 * 1024);
    let task = tokio::spawn(russh_sftp::server::run(peer, Server(state.clone())));
    let session = SftpSession::new(client).await.unwrap();
    session.set_timeout(2);
    (session, state, task)
}
fn expectation() -> RemoteCommitExpectation {
    RemoteCommitExpectation {
        size: Some(3),
        modified_at: Some(1),
        force: false,
    }
}

#[tokio::test]
async fn saved_content_is_private_from_open_and_preserves_permissions() {
    let (session, state, task) = server().await;
    let result = save_text(
        &session,
        "/config".into(),
        b"updated".to_vec(),
        expectation(),
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap();
    assert_eq!(result.size, Some(7));
    let state = state.lock().unwrap();
    assert_eq!(state.opened_modes, vec![Some(0o600)]);
    assert_eq!(state.exclusive_opens, vec![true]);
    assert_eq!(state.files.len(), 1);
    assert_eq!(state.files["/config"].bytes, b"updated");
    assert_eq!(state.files["/config"].attrs.permissions, Some(0o600));
    task.abort();
}

#[tokio::test]
async fn rejected_commit_restores_the_original_and_cleans_the_staged_file() {
    let (session, state, task) = server().await;
    state.lock().unwrap().fail_commit = true;
    assert!(
        save_text(
            &session,
            "/config".into(),
            b"updated".to_vec(),
            expectation(),
            Arc::new(AtomicBool::new(false))
        )
        .await
        .is_err()
    );
    let state = state.lock().unwrap();
    assert_eq!(state.files.len(), 1);
    assert_eq!(state.files["/config"].bytes, b"old");
    task.abort();
}

#[tokio::test]
async fn changes_during_upload_and_cancellation_never_replace_the_original() {
    let (session, state, task) = server().await;
    state.lock().unwrap().change_during_write = true;
    let error = save_text(
        &session,
        "/config".into(),
        b"updated".to_vec(),
        expectation(),
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "remoteChanged");
    assert_eq!(
        state.lock().unwrap().files["/config"].bytes,
        b"external update"
    );
    assert_eq!(state.lock().unwrap().files.len(), 1);
    assert!(
        save_text(
            &session,
            "/config".into(),
            b"updated".to_vec(),
            expectation(),
            Arc::new(AtomicBool::new(true))
        )
        .await
        .is_err()
    );
    assert_eq!(state.lock().unwrap().files.len(), 1);
    task.abort();
}
