import { useEffect, useMemo, useState, type WheelEvent } from 'react';
import { useOutletContext } from 'react-router-dom';
import { LocalFilePane, RemoteFilePane } from './SftpFilePanes';
import { disconnectSftpSession } from '../api/sftp-client';
import { useSftpStore } from '../model/sftp-store';
import type { SortState } from '../types/sftp-types';
import { useSftpDirectoryBrowser } from '../model/useSftpDirectoryBrowser';
import { useSftpFeedback } from '../model/useSftpFeedback';
import { getSftpErrorMessage } from '../model/sftp-error';
import { useSftpFileOperations } from '../model/useSftpFileOperations';
import { useSftpSelection } from '../model/useSftpSelection';
import { nextSortState, sortRows, toLocalRows, toRemoteRows } from '../model/sftp-view-model';
import styles from './SFTPView.module.css';
import type { AppShellOutletContext } from '../../../shared/types/app-shell-context';

function SFTPView() {
  const sessions = useSftpStore((s) => s.sessions);
  const activeId = useSftpStore((s) => s.activeId);
  const setActive = useSftpStore((s) => s.setActive);
  const closeSession = useSftpStore((s) => s.closeSession);
  const setCurrentPath = useSftpStore((s) => s.setCurrentPath);
  const setSessionStatus = useSftpStore((s) => s.setSessionStatus);
  const setSelectionSummary = useSftpStore((s) => s.setSelectionSummary);
  const { sidebarCollapsed, toggleSidebar } = useOutletContext<AppShellOutletContext>();
  const activeSession = sessions.find((session) => session.connectionId === activeId);
  const activeConnectionId = activeSession?.connectionId;
  const activeConnectionAttempt = activeSession?.connectionAttempt ?? 0;
  const activeRemotePath = activeSession?.currentPath || activeSession?.initialPath || '/';
  const [localSort, setLocalSort] = useState<SortState>({ key: 'name', direction: 'asc' });
  const [remoteSort, setRemoteSort] = useState<SortState>({ key: 'name', direction: 'asc' });

  /** 垂直滚轮映射为横向滚动，保持多连接标签在窄窗口下可访问。 */
  const handleTabWheel = (event: WheelEvent<HTMLDivElement>) => {
    if (Math.abs(event.deltaY) <= Math.abs(event.deltaX)) return;
    event.currentTarget.scrollLeft += event.deltaY;
    event.preventDefault();
  };

  const { toasts, confirmState, showToast, requestConfirm, closeConfirm } = useSftpFeedback();
  const handleCloseSession = async (connectionId: string) => {
    try {
      await disconnectSftpSession(connectionId);
      closeSession(connectionId);
    } catch (error) {
      showToast(`断开 SFTP 失败：${getSftpErrorMessage(error)}`);
    }
  };
  const {
    localListing,
    remoteListing,
    localPath,
    remotePath,
    localLoading,
    remoteLoading,
    localError,
    remoteError,
    loadLocalDirectory,
    loadRemoteDirectory,
    invalidateLocalPath,
    invalidateRemotePath,
  } = useSftpDirectoryBrowser({
    activeConnectionId,
    activeConnectionAttempt,
    activeRemotePath,
    setCurrentPath,
    setSessionStatus,
  });
  const localRows = useMemo(
    () => sortRows(toLocalRows(localListing), localSort),
    [localListing, localSort]
  );
  const remoteRows = useMemo(
    () => sortRows(toRemoteRows(remoteListing), remoteSort),
    [remoteListing, remoteSort]
  );
  const {
    localSelectedNames,
    remoteSelectedNames,
    selectLocal,
    selectRemote,
    selectAllLocal,
    selectAllRemote,
    resetLocalSelection,
    resetRemoteSelection,
  } = useSftpSelection({
    localRows,
    remoteRows,
    setSelectionSummary,
  });

  useEffect(() => {
    resetLocalSelection();
  }, [localListing?.path, resetLocalSelection]);

  useEffect(() => {
    resetRemoteSelection();
  }, [activeConnectionId, remoteListing?.path, resetRemoteSelection]);

  const fileOperations = useSftpFileOperations({
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
  });

  return (
    <div className={styles.view}>
      <div className={styles.topBar}>
        <button
          className={`${styles.panelToggle} ${!sidebarCollapsed ? styles.on : ''}`}
          title={sidebarCollapsed ? '显示连接面板' : '隐藏连接面板'}
          onClick={toggleSidebar}
        >
          <span
            className={`${styles.toggleIcon} ${
              sidebarCollapsed ? styles.toggleIconLeft : styles.toggleIconRight
            }`}
            aria-hidden="true"
          />
        </button>

        <div
          aria-label="SFTP 连接会话"
          className={styles.sessionTabViewport}
          data-no-window-drag="true"
          onWheel={handleTabWheel}
          role="tablist"
        >
          <div className={styles.sessionTabTrack}>
            {sessions.map((session) => {
              const selected = session.connectionId === activeId;
              const connectionTitle = `${session.connectionName} — ${session.username}@${session.host}:${session.port}`;

              return (
                <div
                  className={`${styles.sessionTab} ${selected ? styles.active : ''}`}
                  key={session.connectionId}
                  title={connectionTitle}
                >
                  <button
                    aria-selected={selected}
                    className={styles.sessionTabSelect}
                    onClick={() => setActive(session.connectionId)}
                    role="tab"
                    type="button"
                  >
                    <span className={styles.statusDot} data-status={session.status} />
                    <span className={styles.sessionName}>{session.connectionName}</span>
                  </button>
                  <button
                    aria-label={`关闭 ${session.connectionName}`}
                    className={styles.sessionCloseButton}
                    onClick={() => void handleCloseSession(session.connectionId)}
                    title="关闭 SFTP"
                    type="button"
                  >
                    ×
                  </button>
                </div>
              );
            })}
          </div>
        </div>
      </div>

      {!activeSession ? (
        <div className={styles.empty}>
          <div className={styles.emptyState}>
            <svg
              className={styles.emptyIcon}
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.8"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <path d="M3 6a2 2 0 0 1 2-2h5l2 2h7a2 2 0 0 1 2 2v9a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3z" />
              <path d="M8 12h8" />
            </svg>
            <h2 className={styles.emptyTitle}>选择远程连接</h2>
            <p className={styles.emptyHint}>连接后可浏览本地和远程文件，并执行上传、下载等操作。</p>
          </div>
        </div>
      ) : (
        <main className={styles.manager}>
          <LocalFilePane
            path={localListing?.path || localPath || '~'}
            parent={localListing?.parent ?? null}
            rows={localRows}
            loading={localLoading}
            error={localError}
            selectedNames={localSelectedNames}
            sort={localSort}
            createEdit={fileOperations.createEdit}
            renameEdit={fileOperations.renameEdit}
            pathEdit={fileOperations.pathEdit}
            onSelect={selectLocal}
            onSelectAll={selectAllLocal}
            onClearSelection={resetLocalSelection}
            onOpenDir={(path) => void loadLocalDirectory(path)}
            onRefresh={() => void loadLocalDirectory(localListing?.path || localPath, true)}
            onSort={(key) => setLocalSort((current) => nextSortState(current, key))}
            onUpload={(rows) => void fileOperations.upload(rows)}
            onCreateDir={fileOperations.startCreateLocalDir}
            onRename={fileOperations.startRenameLocal}
            onDelete={(rows) => void fileOperations.deleteLocal(rows)}
            onCreateValueChange={fileOperations.updateCreateValue}
            onCreateCommit={() => void fileOperations.commitCreate()}
            onCreateCancel={fileOperations.cancelCreate}
            onRenameValueChange={fileOperations.updateRenameValue}
            onRenameCommit={() => void fileOperations.commitRename()}
            onRenameCancel={fileOperations.cancelRename}
            onPathEditStart={fileOperations.startPathEditLocal}
            onPathEditValueChange={fileOperations.updatePathEditValue}
            onPathEditCommit={() => void fileOperations.commitPathEdit()}
            onPathEditCancel={fileOperations.cancelPathEdit}
            onOpenParent={() => {
              if (localListing?.parent) void loadLocalDirectory(localListing.parent);
            }}
          />
          <RemoteFilePane
            path={remoteListing?.path || remotePath || activeRemotePath}
            parent={remoteListing?.parent ?? null}
            rows={remoteRows}
            loading={remoteLoading}
            error={remoteError}
            selectedNames={remoteSelectedNames}
            sort={remoteSort}
            createEdit={fileOperations.createEdit}
            renameEdit={fileOperations.renameEdit}
            pathEdit={fileOperations.pathEdit}
            onSelect={selectRemote}
            onSelectAll={selectAllRemote}
            onClearSelection={resetRemoteSelection}
            onOpenDir={(path) => void loadRemoteDirectory(path)}
            onRefresh={() =>
              void loadRemoteDirectory(remoteListing?.path || remotePath || activeRemotePath, true)
            }
            onSort={(key) => setRemoteSort((current) => nextSortState(current, key))}
            onDownload={(rows) => void fileOperations.download(rows)}
            onCreateDir={fileOperations.startCreateRemoteDir}
            onRename={fileOperations.startRenameRemote}
            onDelete={(rows) => void fileOperations.deleteRemote(rows)}
            onCreateValueChange={fileOperations.updateCreateValue}
            onCreateCommit={() => void fileOperations.commitCreate()}
            onCreateCancel={fileOperations.cancelCreate}
            onRenameValueChange={fileOperations.updateRenameValue}
            onRenameCommit={() => void fileOperations.commitRename()}
            onRenameCancel={fileOperations.cancelRename}
            onPathEditStart={fileOperations.startPathEditRemote}
            onPathEditValueChange={fileOperations.updatePathEditValue}
            onPathEditCommit={() => void fileOperations.commitPathEdit()}
            onPathEditCancel={fileOperations.cancelPathEdit}
            onOpenParent={() => {
              if (remoteListing?.parent) void loadRemoteDirectory(remoteListing.parent);
            }}
          />
        </main>
      )}
      {confirmState && (
        <div className={styles.confirmLayer}>
          <section className={styles.confirmDialog} role="dialog" aria-modal="true">
            <h3>{confirmState.title}</h3>
            <p>{confirmState.message}</p>
            <div className={styles.confirmActions}>
              <button type="button" onClick={() => closeConfirm(false)}>
                取消
              </button>
              <button
                type="button"
                className={confirmState.danger ? styles.dangerButton : styles.primaryButton}
                onClick={() => closeConfirm(true)}
              >
                {confirmState.confirmLabel || '确认'}
              </button>
            </div>
          </section>
        </div>
      )}
      {toasts.length > 0 && (
        <div className={styles.toastStack}>
          {toasts.map((toast) => (
            <div
              key={toast.id}
              className={`${styles.toast} ${
                toast.tone === 'error'
                  ? styles.toastError
                  : toast.tone === 'success'
                    ? styles.toastSuccess
                    : ''
              }`}
            >
              {toast.message}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export default SFTPView;
