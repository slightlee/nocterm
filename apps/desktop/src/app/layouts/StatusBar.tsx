/* eslint-disable react-hooks/set-state-in-effect */
import { useEffect, useMemo, useRef, useState } from 'react';
import { useLocation } from 'react-router-dom';
import {
  cancelFileTransfer,
  getSftpErrorMessage,
  onFileTransferProgress,
  type FileTransferProgress,
  useSftpStore,
} from '../../features/sftp';
import { useTerminalStore } from '../../features/terminal';
import styles from './StatusBar.module.css';

type TransferTask = FileTransferProgress & {
  createdAt: number;
  updatedAt: number;
  order: number;
};

function truncatePath(path: string, maxLen = 64): string {
  if (path.length <= maxLen) return path;
  const half = Math.floor((maxLen - 3) / 2);
  return `${path.slice(0, half)}...${path.slice(-half)}`;
}

function formatBytes(size: number): string {
  if (size < 1024) return `${size} B`;
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(size < 10 * 1024 ? 1 : 0)} KB`;
  if (size < 1024 * 1024 * 1024) return `${(size / 1024 / 1024).toFixed(1)} MB`;
  return `${(size / 1024 / 1024 / 1024).toFixed(1)} GB`;
}

export function StatusBar() {
  const location = useLocation();
  const sessions = useTerminalStore((s) => s.sessions);
  const activeId = useTerminalStore((s) => s.activeId);
  const terminalStatuses = useTerminalStore((s) => s.statuses);
  const sftpSessions = useSftpStore((s) => s.sessions);
  const activeSftpId = useSftpStore((s) => s.activeId);
  const sftpSelectionSummary = useSftpStore((s) => s.selectionSummary);

  const activeSession = sessions.find((session) => session.id === activeId);
  const activeSftpSession = sftpSessions.find((session) => session.connectionId === activeSftpId);
  const [transferTasks, setTransferTasks] = useState<Map<string, TransferTask>>(new Map());
  const [transferPanelOpen, setTransferPanelOpen] = useState(false);
  const [activeTransferId, setActiveTransferId] = useState<string | null>(null);
  const transferOrderRef = useRef(0);
  const transferCleanupTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;

    onFileTransferProgress((progress) => {
      const now = Date.now();

      setTransferTasks((current) => {
        const next = new Map(current);
        const previous = next.get(progress.taskId);
        next.set(progress.taskId, {
          ...progress,
          createdAt: previous?.createdAt ?? now,
          updatedAt: now,
          order: previous?.order ?? transferOrderRef.current++,
        });
        return next;
      });
    }).then((listener) => {
      if (disposed) {
        listener();
        return;
      }
      unlisten = listener;
    });

    return () => {
      disposed = true;
      if (transferCleanupTimerRef.current) clearTimeout(transferCleanupTimerRef.current);
      unlisten?.();
    };
  }, []);

  const tasks = useMemo(() => Array.from(transferTasks.values()), [transferTasks]);
  const completedTasks = useMemo(
    () => tasks.filter((task) => task.status === 'completed'),
    [tasks]
  );
  const failedTasks = useMemo(() => tasks.filter((task) => task.status === 'error'), [tasks]);
  const cancelledTasks = useMemo(
    () => tasks.filter((task) => task.status === 'cancelled'),
    [tasks]
  );
  const sortedTasks = useMemo(() => [...tasks].sort((a, b) => a.order - b.order), [tasks]);

  const clearTransferTasks = (status: 'completed' | 'error' | 'cancelled') => {
    setTransferTasks((current) => {
      const next = new Map(current);
      for (const [taskId, task] of next) {
        if (task.status === status) next.delete(taskId);
      }
      return next;
    });
  };

  useEffect(() => {
    setActiveTransferId((currentId) => {
      const currentTask = currentId ? transferTasks.get(currentId) : null;
      const nextRunningTask = [...transferTasks.values()]
        .filter((task) => task.status === 'running')
        .sort((a, b) => a.order - b.order)[0];

      if (currentTask?.status === 'running') return currentId;
      if (nextRunningTask) return nextRunningTask.taskId;
      if (currentTask) return currentId;

      const latestTask = [...transferTasks.values()].sort((a, b) => b.updatedAt - a.updatedAt)[0];
      return latestTask?.taskId ?? null;
    });
  }, [transferTasks]);

  useEffect(() => {
    if (tasks.length === 0) setTransferPanelOpen(false);
  }, [tasks.length]);

  useEffect(() => {
    if (transferCleanupTimerRef.current) {
      clearTimeout(transferCleanupTimerRef.current);
      transferCleanupTimerRef.current = null;
    }

    if (tasks.length === 0 || tasks.some((task) => task.status === 'running')) return;

    const hasCompletedTask = tasks.some((task) => task.status === 'completed');
    transferCleanupTimerRef.current = setTimeout(
      () => {
        setTransferTasks((current) => {
          const next = new Map(current);
          for (const [taskId, task] of next) {
            if (
              hasCompletedTask
                ? task.status === 'completed'
                : task.status === 'error' || task.status === 'cancelled'
            ) {
              next.delete(taskId);
            }
          }
          return next;
        });
        transferCleanupTimerRef.current = null;
      },
      hasCompletedTask ? 5000 : 10000
    );

    return () => {
      if (transferCleanupTimerRef.current) {
        clearTimeout(transferCleanupTimerRef.current);
        transferCleanupTimerRef.current = null;
      }
    };
  }, [tasks]);

  const activeTerminalStatus = activeId === null ? 'idle' : terminalStatuses[String(activeId)];
  const statusClass =
    activeTerminalStatus === 'connected'
      ? styles.green
      : activeTerminalStatus === 'connecting'
        ? styles.orange
        : styles.red;
  const statusTitle = activeSession
    ? `${activeSession.name} ${activeSession.kind === 'remote' ? activeSession.remoteInitialPath || '~' : '~'}`
    : '';

  if (location.pathname.startsWith('/sftp')) {
    const remotePath = activeSftpSession?.currentPath || activeSftpSession?.initialPath || '/';
    const activeTransfer = activeTransferId ? (transferTasks.get(activeTransferId) ?? null) : null;
    const transferPercent =
      activeTransfer && activeTransfer.total > 0
        ? Math.min(100, Math.round((activeTransfer.transferred / activeTransfer.total) * 100))
        : null;
    const transferLabel =
      activeTransfer?.direction === 'upload'
        ? '上传'
        : activeTransfer?.direction === 'download'
          ? '下载'
          : '';

    return (
      <footer className={styles.statusbar}>
        {transferPanelOpen && tasks.length > 0 && (
          <div className={styles.transferPanel}>
            <div className={styles.transferPanelHeader}>
              <span>传输队列</span>
              <div className={styles.transferPanelActions}>
                {completedTasks.length > 0 && (
                  <button type="button" onClick={() => clearTransferTasks('completed')}>
                    清除完成
                  </button>
                )}
                {failedTasks.length > 0 && (
                  <button type="button" onClick={() => clearTransferTasks('error')}>
                    清除失败
                  </button>
                )}
                {cancelledTasks.length > 0 && (
                  <button type="button" onClick={() => clearTransferTasks('cancelled')}>
                    清除取消
                  </button>
                )}
                <button
                  type="button"
                  className={styles.transferPanelClose}
                  onClick={() => setTransferPanelOpen(false)}
                >
                  ×
                </button>
              </div>
            </div>
            <div className={styles.transferList}>
              {sortedTasks.map((task) => {
                const percent =
                  task.total > 0
                    ? Math.min(100, Math.round((task.transferred / task.total) * 100))
                    : null;
                const label = task.direction === 'upload' ? '上传' : '下载';
                const taskError = task.error ? getSftpErrorMessage(task.error) : null;
                const displayName =
                  task.status === 'error' ? taskError || task.fileName : task.fileName;
                const displayTitle = taskError ? `${task.fileName}：${taskError}` : task.fileName;
                return (
                  <div key={task.taskId} className={styles.transferItem}>
                    <div className={styles.transferItemTop}>
                      <span
                        className={`${styles.transferBadge} ${
                          task.status === 'error'
                            ? styles.transferError
                            : task.status === 'cancelled'
                              ? styles.transferCancelled
                              : ''
                        }`}
                      >
                        {task.status === 'completed'
                          ? `${label}完成`
                          : task.status === 'error'
                            ? `${label}失败`
                            : task.status === 'cancelled'
                              ? `${label}已取消`
                              : `${label}中`}
                      </span>
                      <span className={styles.transferItemName} title={displayTitle}>
                        {displayName}
                      </span>
                      {percent != null && (
                        <span className={styles.transferItemMeta}>{percent}%</span>
                      )}
                      {percent == null && task.transferred > 0 && (
                        <span className={styles.transferItemMeta}>
                          {formatBytes(task.transferred)}
                        </span>
                      )}
                      {task.status === 'running' && (
                        <button
                          type="button"
                          className={styles.transferItemAction}
                          onClick={() => void cancelFileTransfer(task.taskId)}
                        >
                          取消
                        </button>
                      )}
                      {(task.status === 'error' || task.status === 'cancelled') && (
                        <button
                          type="button"
                          className={styles.transferItemAction}
                          onClick={() => {
                            setTransferTasks((current) => {
                              const next = new Map(current);
                              next.delete(task.taskId);
                              return next;
                            });
                            window.dispatchEvent(
                              new CustomEvent('nocterm-sftp-retry', {
                                detail: { taskId: task.taskId },
                              })
                            );
                          }}
                        >
                          {task.direction === 'upload' ? '重新上传' : '重新下载'}
                        </button>
                      )}
                    </div>
                    <span className={styles.transferItemTrack}>
                      <span
                        className={`${styles.progressFill} ${
                          percent == null ? styles.progressFillUnknown : ''
                        }`}
                        style={{ width: percent == null ? undefined : `${percent}%` }}
                      />
                    </span>
                  </div>
                );
              })}
            </div>
          </div>
        )}
        {activeTransfer ? (
          <button
            type="button"
            className={styles.transferStatus}
            onClick={() => setTransferPanelOpen((open) => !open)}
          >
            <span
              className={`${styles.transferBadge} ${
                activeTransfer.status === 'error'
                  ? styles.transferError
                  : activeTransfer.status === 'cancelled'
                    ? styles.transferCancelled
                    : ''
              }`}
            >
              {activeTransfer.status === 'completed'
                ? `${transferLabel}完成`
                : activeTransfer.status === 'error'
                  ? `${transferLabel}失败`
                  : activeTransfer.status === 'cancelled'
                    ? `${transferLabel}已取消`
                    : `${transferLabel}中`}
            </span>
            <span
              className={styles.transferName}
              title={
                activeTransfer.error
                  ? `${activeTransfer.fileName}：${getSftpErrorMessage(activeTransfer.error)}`
                  : activeTransfer.fileName
              }
            >
              {activeTransfer.status === 'error'
                ? activeTransfer.error
                  ? getSftpErrorMessage(activeTransfer.error)
                  : activeTransfer.fileName
                : activeTransfer.fileName}
            </span>
            {transferPercent != null ? (
              <>
                <span className={styles.progressTrack}>
                  <span className={styles.progressFill} style={{ width: `${transferPercent}%` }} />
                </span>
                <span className={styles.transferMeta}>{transferPercent}%</span>
              </>
            ) : activeTransfer.transferred > 0 ? (
              <>
                <span className={styles.progressTrack}>
                  <span className={`${styles.progressFill} ${styles.progressFillUnknown}`} />
                </span>
                <span className={styles.transferMeta}>
                  {formatBytes(activeTransfer.transferred)}
                </span>
              </>
            ) : null}
            <span className={styles.transferQueue}>传输 {tasks.length}</span>
            {failedTasks.length > 0 && (
              <span className={styles.transferQueue}>失败 {failedTasks.length}</span>
            )}
            {cancelledTasks.length > 0 && (
              <span className={styles.transferQueue}>取消 {cancelledTasks.length}</span>
            )}
          </button>
        ) : activeSftpSession ? (
          <div className={styles.statusLeft}>
            <div className={`${styles.statusDot} ${styles.green}`} />
            <span className={styles.sessionName}>{activeSftpSession.connectionName}</span>
            <span className={styles.sessionPath}>{truncatePath(remotePath)}</span>
          </div>
        ) : (
          <div />
        )}
        <div className={styles.spacer} />
        <div className={styles.statusRight}>
          {sftpSelectionSummary.count > 0 && (
            <span className={styles.statusItem}>
              {sftpSelectionSummary.scope === 'remote' ? '远程' : '本地'}已选{' '}
              {sftpSelectionSummary.count} 项
              {sftpSelectionSummary.totalSize != null
                ? ` · ${formatBytes(sftpSelectionSummary.totalSize)}`
                : ''}
            </span>
          )}
          <span className={styles.statusItem}>UTF-8</span>
          <span className={styles.statusItem}>LF</span>
        </div>
      </footer>
    );
  }

  return (
    <footer className={styles.statusbar}>
      {activeSession && (
        <div className={styles.statusLeft} title={statusTitle}>
          <div className={`${styles.statusDot} ${statusClass}`} />
          <>
            <span className={styles.sessionName}>{activeSession.name}</span>
            <span className={styles.sessionPath}>
              {truncatePath(
                activeSession.kind === 'remote' ? activeSession.remoteInitialPath || '~' : '~'
              )}
            </span>
          </>
        </div>
      )}
      <div className={styles.spacer} />
      <div className={styles.statusRight}>
        <span className={styles.statusItem}>UTF-8</span>
        <span className={styles.statusItem}>LF</span>
      </div>
    </footer>
  );
}

export default StatusBar;
