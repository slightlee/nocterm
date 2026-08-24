/* eslint-disable react-hooks/preserve-manual-memoization */
import { useCallback, useEffect, useRef, useState } from 'react';
import {
  createLocalDir,
  createRemoteDir,
  deleteLocalPath,
  deleteRemotePath,
  downloadRemoteToLocal,
  localPathExists,
  onFileTransferProgress,
  remotePathExists,
  renameLocalPath,
  renameRemotePath,
  uploadLocalToRemote,
  type LocalDirectoryListing,
  type RemoteDirectoryListing,
} from '../api/sftp-client';
import { getSftpErrorMessage } from './sftp-error';
import { useSftpStore } from './sftp-store';
import type { CreateEditState, FileRow, PathEditState, RenameEditState } from '../types/sftp-types';
import { getErrorMessage, joinPath, validateNameInput } from './sftp-view-model';

type RequestConfirm = (options: {
  title: string;
  message: string;
  confirmLabel?: string;
  danger?: boolean;
}) => Promise<boolean>;

type TransferTarget = {
  type: 'upload' | 'download';
  path: string;
  connectionId?: string;
  sourcePath: string;
  total?: number | null;
  isDir?: boolean;
};

interface UseSftpFileOperationsOptions {
  activeConnectionId?: string;
  activeRemotePath: string;
  localListing: LocalDirectoryListing | null;
  remoteListing: RemoteDirectoryListing | null;
  localPath?: string;
  remotePath?: string;
  loadLocalDirectory: (path?: string, force?: boolean) => Promise<void>;
  loadRemoteDirectory: (path?: string, force?: boolean) => Promise<void>;
  invalidateLocalPath: (path: string) => void;
  invalidateRemotePath: (connectionId: string, path: string) => void;
  requestConfirm: RequestConfirm;
  showToast: (message: string, tone?: 'error' | 'info' | 'success') => void;
}

export function useSftpFileOperations({
  activeConnectionId,
  activeRemotePath,
  localListing,
  remoteListing,
  localPath,
  remotePath,
  loadLocalDirectory,
  loadRemoteDirectory,
  invalidateLocalPath,
  invalidateRemotePath,
  requestConfirm,
  showToast,
}: UseSftpFileOperationsOptions) {
  const [createEdit, setCreateEdit] = useState<CreateEditState | null>(null);
  const [renameEdit, setRenameEdit] = useState<RenameEditState | null>(null);
  const [pathEdit, setPathEdit] = useState<PathEditState | null>(null);
  // 输入框 blur 和 Enter 可能同时触发提交，用锁避免重复创建或重命名。
  const editCommitRef = useRef(false);
  // 后端进度事件只带 taskId，这里记录任务目标，完成后刷新对应目录。
  const transferTargetsRef = useRef<Map<string, TransferTarget>>(new Map());
  const transferCleanupTimersRef = useRef<Map<string, ReturnType<typeof setTimeout>>>(new Map());
  const activeConnectionIdRef = useRef(activeConnectionId);

  useEffect(() => {
    activeConnectionIdRef.current = activeConnectionId;
  }, [activeConnectionId]);

  const upload = useCallback(
    async (rows: FileRow[]) => {
      if (!activeConnectionId || rows.length === 0) return;
      const targetRemoteDir = remoteListing?.path || remotePath || activeRemotePath;
      for (const row of rows) {
        if (!row.path) continue;
        try {
          const targetPath = joinPath(targetRemoteDir, row.name);
          const exists = await remotePathExists(activeConnectionId, targetPath);
          if (
            exists &&
            !(await requestConfirm({
              title: '覆盖远程项目',
              message: `远程已存在「${row.name}」，是否覆盖？`,
              confirmLabel: '覆盖',
            }))
          ) {
            continue;
          }
          const task = await uploadLocalToRemote(activeConnectionId, row.path, targetRemoteDir);
          transferTargetsRef.current.set(task.taskId, {
            type: 'upload',
            path: targetRemoteDir,
            connectionId: activeConnectionId,
            sourcePath: row.path,
          });
          useSftpStore.getState().trackTransfer(task.taskId, activeConnectionId);
        } catch (err) {
          showToast(`上传失败：${getSftpErrorMessage(err)}`);
        }
      }
    },
    [
      activeConnectionId,
      activeRemotePath,
      remoteListing?.path,
      remotePath,
      requestConfirm,
      showToast,
    ]
  );

  const download = useCallback(
    async (rows: FileRow[]) => {
      if (!activeConnectionId || rows.length === 0) return;
      const targetLocalDir = localListing?.path || localPath;
      if (!targetLocalDir) return;
      for (const row of rows) {
        if (!row.path) continue;
        try {
          const targetPath = joinPath(targetLocalDir, row.name);
          const exists = await localPathExists(targetPath);
          if (
            exists &&
            !(await requestConfirm({
              title: '覆盖本地项目',
              message: `本地已存在「${row.name}」，是否覆盖？`,
              confirmLabel: '覆盖',
            }))
          ) {
            continue;
          }
          const task = await downloadRemoteToLocal(
            activeConnectionId,
            row.path,
            targetLocalDir,
            row.sizeBytes,
            row.isDir
          );
          transferTargetsRef.current.set(task.taskId, {
            type: 'download',
            path: targetLocalDir,
            connectionId: activeConnectionId,
            sourcePath: row.path,
            total: row.sizeBytes,
            isDir: row.isDir,
          });
          useSftpStore.getState().trackTransfer(task.taskId, activeConnectionId);
        } catch (err) {
          showToast(`下载失败：${getSftpErrorMessage(err)}`);
        }
      }
    },
    [activeConnectionId, localListing?.path, localPath, requestConfirm, showToast]
  );

  const deleteLocal = useCallback(
    async (rows: FileRow[]) => {
      if (rows.length === 0) return;
      const confirmed = await requestConfirm({
        title: '删除本地项目',
        message:
          rows.length === 1
            ? `确定删除本地${rows[0].isDir ? '目录' : '文件'}「${rows[0].name}」吗？`
            : `确定删除选中的 ${rows.length} 个本地项目吗？`,
        confirmLabel: '删除',
        danger: true,
      });
      if (!confirmed) return;
      const currentPath = localListing?.path || localPath;
      for (const row of rows) {
        if (!row.path) continue;
        try {
          await deleteLocalPath(row.path);
        } catch (err) {
          showToast(`删除失败：${getErrorMessage(err)}`);
        }
      }
      if (currentPath) {
        invalidateLocalPath(currentPath);
        await loadLocalDirectory(currentPath, true);
      }
    },
    [
      invalidateLocalPath,
      loadLocalDirectory,
      localListing?.path,
      localPath,
      requestConfirm,
      showToast,
    ]
  );

  const deleteRemote = useCallback(
    async (rows: FileRow[]) => {
      if (!activeConnectionId || rows.length === 0) return;
      const confirmed = await requestConfirm({
        title: '删除远程项目',
        message:
          rows.length === 1
            ? `确定删除远程${rows[0].isDir ? '目录' : '文件'}「${rows[0].name}」吗？`
            : `确定删除选中的 ${rows.length} 个远程项目吗？`,
        confirmLabel: '删除',
        danger: true,
      });
      if (!confirmed) return;
      const currentPath = remoteListing?.path || remotePath || activeRemotePath;
      for (const row of rows) {
        if (!row.path) continue;
        try {
          await deleteRemotePath(activeConnectionId, row.path);
        } catch (err) {
          showToast(`删除失败：${getSftpErrorMessage(err)}`);
        }
      }
      invalidateRemotePath(activeConnectionId, currentPath);
      await loadRemoteDirectory(currentPath, true);
    },
    [
      activeConnectionId,
      activeRemotePath,
      invalidateRemotePath,
      loadRemoteDirectory,
      remoteListing?.path,
      remotePath,
      requestConfirm,
      showToast,
    ]
  );

  const startCreateLocalDir = useCallback(() => {
    setCreateEdit({ scope: 'local', value: '' });
  }, []);

  const startCreateRemoteDir = useCallback(() => {
    setCreateEdit({ scope: 'remote', value: '' });
  }, []);

  const startRenameLocal = useCallback((row: FileRow) => {
    if (!row.path) return;
    setRenameEdit({ scope: 'local', row, value: row.name });
  }, []);

  const startRenameRemote = useCallback(
    (row: FileRow) => {
      if (!activeConnectionId || !row.path) return;
      setRenameEdit({ scope: 'remote', row, value: row.name });
    },
    [activeConnectionId]
  );

  const startPathEditLocal = useCallback(() => {
    setPathEdit({ scope: 'local', value: localListing?.path || localPath || '' });
  }, [localListing?.path, localPath]);

  const startPathEditRemote = useCallback(() => {
    setPathEdit({
      scope: 'remote',
      value: remoteListing?.path || remotePath || activeRemotePath,
    });
  }, [activeRemotePath, remoteListing?.path, remotePath]);

  const updateCreateValue = useCallback((value: string) => {
    setCreateEdit((current) => (current ? { ...current, value } : current));
  }, []);

  const updateRenameValue = useCallback((value: string) => {
    setRenameEdit((current) => (current ? { ...current, value } : current));
  }, []);

  const updatePathEditValue = useCallback((value: string) => {
    setPathEdit((current) => (current ? { ...current, value } : current));
  }, []);

  const cancelCreate = useCallback(() => {
    setCreateEdit(null);
  }, []);

  const cancelRename = useCallback(() => {
    setRenameEdit(null);
  }, []);

  const cancelPathEdit = useCallback(() => {
    setPathEdit(null);
  }, []);

  const commitPathEdit = useCallback(async () => {
    if (editCommitRef.current) return;
    if (!pathEdit) return;
    const path = pathEdit.value.trim();
    if (!path) {
      setPathEdit(null);
      return;
    }

    editCommitRef.current = true;
    try {
      if (pathEdit.scope === 'local') {
        await loadLocalDirectory(path, true);
      } else {
        await loadRemoteDirectory(path, true);
      }
      setPathEdit(null);
    } catch (err) {
      showToast(
        `跳转目录失败：${pathEdit.scope === 'remote' ? getSftpErrorMessage(err) : getErrorMessage(err)}`
      );
    } finally {
      editCommitRef.current = false;
    }
  }, [loadLocalDirectory, loadRemoteDirectory, pathEdit, showToast]);

  const commitCreate = useCallback(async () => {
    if (editCommitRef.current) return;
    if (!createEdit) return;
    const { name, error } = validateNameInput(createEdit.value);
    if (error) {
      showToast(error);
      return;
    }
    if (!name) {
      setCreateEdit(null);
      return;
    }

    editCommitRef.current = true;
    try {
      if (createEdit.scope === 'local') {
        const parent = localListing?.path || localPath;
        if (!parent) return;
        await createLocalDir(parent, name);
        setCreateEdit(null);
        invalidateLocalPath(parent);
        await loadLocalDirectory(parent, true);
        return;
      }

      if (!activeConnectionId) return;
      const parent = remoteListing?.path || remotePath || activeRemotePath;
      await createRemoteDir(activeConnectionId, parent, name);
      setCreateEdit(null);
      invalidateRemotePath(activeConnectionId, parent);
      await loadRemoteDirectory(parent, true);
    } catch (err) {
      showToast(
        `新建文件夹失败：${
          createEdit.scope === 'remote' ? getSftpErrorMessage(err) : getErrorMessage(err)
        }`
      );
    } finally {
      editCommitRef.current = false;
    }
  }, [
    activeConnectionId,
    activeRemotePath,
    createEdit,
    invalidateLocalPath,
    invalidateRemotePath,
    loadLocalDirectory,
    loadRemoteDirectory,
    localListing?.path,
    localPath,
    remoteListing?.path,
    remotePath,
    showToast,
  ]);

  const commitRename = useCallback(async () => {
    if (editCommitRef.current) return;
    if (!renameEdit) return;
    const { name, error } = validateNameInput(renameEdit.value);
    if (error) {
      showToast(error);
      return;
    }
    if (!name || name === renameEdit.row.name) {
      setRenameEdit(null);
      return;
    }

    editCommitRef.current = true;
    try {
      if (renameEdit.scope === 'local') {
        if (!renameEdit.row.path) return;
        const currentPath = localListing?.path || localPath;
        await renameLocalPath(renameEdit.row.path, name);
        setRenameEdit(null);
        if (currentPath) {
          invalidateLocalPath(currentPath);
          await loadLocalDirectory(currentPath, true);
        }
        return;
      }

      if (!activeConnectionId || !renameEdit.row.path) return;
      const currentPath = remoteListing?.path || remotePath || activeRemotePath;
      await renameRemotePath(activeConnectionId, renameEdit.row.path, name);
      setRenameEdit(null);
      invalidateRemotePath(activeConnectionId, currentPath);
      await loadRemoteDirectory(currentPath, true);
    } catch (err) {
      showToast(
        `重命名失败：${
          renameEdit.scope === 'remote' ? getSftpErrorMessage(err) : getErrorMessage(err)
        }`
      );
    } finally {
      editCommitRef.current = false;
    }
  }, [
    activeConnectionId,
    activeRemotePath,
    invalidateLocalPath,
    invalidateRemotePath,
    loadLocalDirectory,
    loadRemoteDirectory,
    localListing?.path,
    localPath,
    remoteListing?.path,
    remotePath,
    renameEdit,
    showToast,
  ]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;

    // 上传/下载完成后只刷新受影响目录，避免整页重新加载。
    onFileTransferProgress((progress) => {
      if (progress.status !== 'running') useSftpStore.getState().finishTransfer(progress.taskId);
      const target = transferTargetsRef.current.get(progress.taskId);
      if (!target) return;
      if (progress.status === 'error' || progress.status === 'cancelled') {
        const previousTimer = transferCleanupTimersRef.current.get(progress.taskId);
        if (previousTimer) clearTimeout(previousTimer);
        const timer = setTimeout(() => {
          transferTargetsRef.current.delete(progress.taskId);
          transferCleanupTimersRef.current.delete(progress.taskId);
        }, 15_000);
        transferCleanupTimersRef.current.set(progress.taskId, timer);
        return;
      }
      if (progress.status !== 'completed') return;
      transferTargetsRef.current.delete(progress.taskId);
      if (target.type === 'upload' && target.connectionId === activeConnectionIdRef.current) {
        if (!target.connectionId) return;
        invalidateRemotePath(target.connectionId, target.path);
        void loadRemoteDirectory(target.path, true);
        return;
      }
      if (target.type === 'download') {
        invalidateLocalPath(target.path);
        void loadLocalDirectory(target.path, true);
      }
    }).then((listener) => {
      if (disposed) {
        listener();
        return;
      }
      unlisten = listener;
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [invalidateLocalPath, invalidateRemotePath, loadLocalDirectory, loadRemoteDirectory]);

  useEffect(() => {
    // 传输列表在共享组件内触发重试事件，SFTP 页面只负责按原目标重新发起任务。
    const handleRetry = (event: Event) => {
      const taskId = (event as CustomEvent<{ taskId?: string }>).detail?.taskId;
      if (!taskId) return;
      const target = transferTargetsRef.current.get(taskId);
      if (!target) return;
      const cleanupTimer = transferCleanupTimersRef.current.get(taskId);
      if (cleanupTimer) {
        clearTimeout(cleanupTimer);
        transferCleanupTimersRef.current.delete(taskId);
      }
      transferTargetsRef.current.delete(taskId);
      useSftpStore.getState().finishTransfer(taskId);

      if (target.type === 'upload') {
        const connectionId = target.connectionId || activeConnectionIdRef.current;
        if (!connectionId) return;
        void uploadLocalToRemote(connectionId, target.sourcePath, target.path)
          .then((task) => {
            transferTargetsRef.current.set(task.taskId, { ...target, connectionId });
            useSftpStore.getState().trackTransfer(task.taskId, connectionId);
          })
          .catch((err) => {
            showToast(`重新上传失败：${getSftpErrorMessage(err)}`);
          });
        return;
      }

      const connectionId = target.connectionId || activeConnectionIdRef.current;
      if (!connectionId) return;
      void downloadRemoteToLocal(
        connectionId,
        target.sourcePath,
        target.path,
        target.total,
        target.isDir
      )
        .then((task) => {
          transferTargetsRef.current.set(task.taskId, { ...target, connectionId });
          useSftpStore.getState().trackTransfer(task.taskId, connectionId);
        })
        .catch((err) => {
          showToast(`重新下载失败：${getSftpErrorMessage(err)}`);
        });
    };

    window.addEventListener('nocterm-sftp-retry', handleRetry);
    return () => window.removeEventListener('nocterm-sftp-retry', handleRetry);
  }, [showToast]);

  useEffect(
    () => () => {
      for (const timer of transferCleanupTimersRef.current.values()) clearTimeout(timer);
      transferCleanupTimersRef.current.clear();
    },
    []
  );

  return {
    createEdit,
    renameEdit,
    pathEdit,
    upload,
    download,
    deleteLocal,
    deleteRemote,
    startCreateLocalDir,
    startCreateRemoteDir,
    startRenameLocal,
    startRenameRemote,
    startPathEditLocal,
    startPathEditRemote,
    updateCreateValue,
    updateRenameValue,
    updatePathEditValue,
    commitCreate,
    commitRename,
    commitPathEdit,
    cancelCreate,
    cancelRename,
    cancelPathEdit,
  };
}
