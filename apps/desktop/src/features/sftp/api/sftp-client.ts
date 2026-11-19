import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { isDesktopRuntime } from '../../../shared/lib/tauri-runtime';

const noopUnlisten: UnlistenFn = () => undefined;
const localDirectoryRequests = new Map<string, Promise<LocalDirectoryListing>>();
const remoteDirectoryRequests = new Map<string, Promise<RemoteDirectoryListing>>();

export interface LocalFileEntry {
  name: string;
  path: string;
  isDir: boolean;
  size: number | null;
  modifiedAt: number | null;
}

export interface LocalDirectoryListing {
  path: string;
  parent: string | null;
  entries: LocalFileEntry[];
}

export interface RemoteFileEntry {
  name: string;
  isDir: boolean;
  size: number | null;
  modifiedAt: number | null;
}

export interface RemoteDirectoryListing {
  path: string;
  parent: string | null;
  entries: RemoteFileEntry[];
}

export interface FileTransferStartResponse {
  taskId: string;
}

export interface FileTransferProgress {
  taskId: string;
  direction: 'upload' | 'download';
  fileName: string;
  transferred: number;
  total: number;
  status: 'running' | 'completed' | 'error' | 'cancelled';
  error: string | null;
}

export function isLocalFileApiAvailable(): boolean {
  return isDesktopRuntime();
}

export function listLocalDir(path?: string): Promise<LocalDirectoryListing> {
  // 未传路径与 ~ 均由后端解析为主目录，应共享同一个在途请求。
  const requestKey = path ?? '~';
  const pending = localDirectoryRequests.get(requestKey);
  if (pending) return pending;

  const request = invoke<LocalDirectoryListing>('list_local_dir', { path: path ?? null });
  localDirectoryRequests.set(requestKey, request);

  // StrictMode、路由重挂载或连续刷新可能触发相同读取；只清理当前 Promise。
  const clearRequest = () => {
    if (localDirectoryRequests.get(requestKey) === request) {
      localDirectoryRequests.delete(requestKey);
    }
  };
  void request.then(clearRequest, clearRequest);
  return request;
}

export function listRemoteDir(
  connectionId: string,
  path?: string
): Promise<RemoteDirectoryListing> {
  const requestKey = `${connectionId}\0${path ?? ''}`;
  const pending = remoteDirectoryRequests.get(requestKey);
  if (pending) return pending;

  const request = invoke<RemoteDirectoryListing>('list_remote_dir', {
    connectionId: Number(connectionId),
    path: path ?? null,
  });
  remoteDirectoryRequests.set(requestKey, request);

  // StrictMode 或连续点击可能触发相同请求；完成后只清理由当前 Promise 占用的槽位。
  const clearRequest = () => {
    if (remoteDirectoryRequests.get(requestKey) === request) {
      remoteDirectoryRequests.delete(requestKey);
    }
  };
  void request.then(clearRequest, clearRequest);
  return request;
}

export function uploadLocalToRemote(
  connectionId: string,
  localPath: string,
  remoteDir: string
): Promise<FileTransferStartResponse> {
  return invoke<FileTransferStartResponse>('upload_local_to_remote', {
    connectionId: Number(connectionId),
    localPath,
    remoteDir,
  });
}

export function downloadRemoteToLocal(
  connectionId: string,
  remotePath: string,
  localDir: string,
  total?: number | null,
  isDir?: boolean
): Promise<FileTransferStartResponse> {
  return invoke<FileTransferStartResponse>('download_remote_to_local', {
    connectionId: Number(connectionId),
    remotePath,
    localDir,
    total: total ?? null,
    isDir: isDir ?? false,
  });
}

export function localPathExists(path: string): Promise<boolean> {
  return invoke<boolean>('local_path_exists', { path });
}

export function remotePathExists(connectionId: string, remotePath: string): Promise<boolean> {
  return invoke<boolean>('remote_path_exists', { connectionId: Number(connectionId), remotePath });
}

export function createLocalDir(parent: string, name: string): Promise<void> {
  return invoke<void>('create_local_dir', { parent, name });
}

export function createRemoteDir(connectionId: string, parent: string, name: string): Promise<void> {
  return invoke<void>('create_remote_dir', { connectionId: Number(connectionId), parent, name });
}

export function renameLocalPath(path: string, newName: string): Promise<void> {
  return invoke<void>('rename_local_path', { path, newName });
}

export function renameRemotePath(
  connectionId: string,
  remotePath: string,
  newName: string
): Promise<void> {
  return invoke<void>('rename_remote_path', {
    connectionId: Number(connectionId),
    remotePath,
    newName,
  });
}

export function cancelFileTransfer(taskId: string): Promise<void> {
  return invoke<void>('cancel_file_transfer', { taskId });
}

/** 关闭标签前终止该连接的传输和 OpenSSH 复用会话。 */
export function disconnectSftpSession(connectionId: string): Promise<void> {
  return invoke<void>('close_sftp_session', { connectionId: Number(connectionId) });
}

export function deleteLocalPath(path: string): Promise<void> {
  return invoke<void>('delete_local_path', { path });
}

export function deleteRemotePath(connectionId: string, remotePath: string): Promise<void> {
  return invoke<void>('delete_remote_path', { connectionId: Number(connectionId), remotePath });
}

/** 在线编辑读取结果：内容已由后端完成大小上限、二进制与 UTF-8 校验。 */
export interface RemoteFileContent {
  content: string;
  size: number | null;
  modifiedAt: number | null;
  targetRevision: string;
}

/** 在线编辑保存结果：提交后的远端 stat，用作下一次保存的冲突基线。 */
export interface RemoteSaveResult {
  size: number | null;
  modifiedAt: number | null;
  targetRevision: string;
}

/** 读取远程文本文件供在线编辑；后端拒绝超限、二进制与非 UTF-8 内容。 */
export function readRemoteFile(
  connectionId: string,
  remotePath: string
): Promise<RemoteFileContent> {
  return invoke<RemoteFileContent>('sftp_read_remote_file', {
    connectionId: Number(connectionId),
    remotePath,
  });
}

/**
 * 保存在线编辑内容：默认以打开/上次保存时的 stat 基线做冲突校验，远端已被
 * 改动时拒绝写入并返回稳定错误码 `remoteChanged`；`force` 为 true 时跳过校验
 * 强制覆盖（对应冲突弹窗中的"覆盖远端"选择）。
 */
export function saveRemoteFile(
  connectionId: string,
  remotePath: string,
  content: string,
  baseline: RemoteSaveResult,
  force = false
): Promise<RemoteSaveResult> {
  return invoke<RemoteSaveResult>('sftp_write_remote_file', {
    connectionId: Number(connectionId),
    remotePath,
    content,
    baselineSize: baseline.size,
    baselineModifiedAt: baseline.modifiedAt,
    targetRevision: baseline.targetRevision,
    force,
  });
}

/** 本地编辑基线：纳秒独立成字段，避免 epoch 纳秒超出 JS 安全整数。 */
export interface LocalFileBaseline {
  size: number;
  modifiedSeconds: number;
  modifiedNanos: number;
}

/** 本地编辑读取结果：identityPath 为后端规范化路径，作为编辑标签身份。 */
export interface LocalFileContent {
  content: string;
  identityPath: string;
  baseline: LocalFileBaseline;
}

/** 读取本地文本文件供在线编辑；后端校验类型、符号链接与文本规则。 */
export function readLocalFile(path: string): Promise<LocalFileContent> {
  return invoke<LocalFileContent>('sftp_read_local_file', { path });
}

/**
 * 保存本地文本文件：与远程一致，默认按基线做冲突校验，本地文件已被外部
 * 修改时拒绝写入（稳定错误码 `localFileChanged`）；`force` 只跳过冲突。
 */
export function saveLocalFile(
  path: string,
  content: string,
  baseline: LocalFileBaseline,
  force = false
): Promise<LocalFileBaseline> {
  return invoke<LocalFileBaseline>('sftp_write_local_file', {
    path,
    content,
    baselineSize: baseline.size,
    baselineModifiedSeconds: baseline.modifiedSeconds,
    baselineModifiedNanos: baseline.modifiedNanos,
    force,
  });
}

export function onFileTransferProgress(
  handler: (progress: FileTransferProgress) => void
): Promise<UnlistenFn> {
  if (!isLocalFileApiAvailable()) return Promise.resolve(noopUnlisten);
  return listen<FileTransferProgress>('nocterm://sftp-transfer-progress', (event) => {
    handler(event.payload);
  });
}
