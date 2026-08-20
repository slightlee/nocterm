import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { isDesktopRuntime } from '../../../shared/lib/tauri-runtime';

const noopUnlisten: UnlistenFn = () => undefined;
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
  return invoke<LocalDirectoryListing>('list_local_dir', { path: path ?? null });
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

export function onFileTransferProgress(
  handler: (progress: FileTransferProgress) => void
): Promise<UnlistenFn> {
  if (!isLocalFileApiAvailable()) return Promise.resolve(noopUnlisten);
  return listen<FileTransferProgress>('nocterm://sftp-transfer-progress', (event) => {
    handler(event.payload);
  });
}
