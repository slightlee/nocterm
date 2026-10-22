import { useEffect, useMemo, useState } from 'react';
import { LocalFilePane, RemoteFilePane } from './SftpFilePanes';
import { useSftpStore } from '../model/sftp-store';
import type { SortState } from '../types/sftp-types';
import { useSftpDirectoryBrowser } from '../model/useSftpDirectoryBrowser';
import { useSftpFeedback } from '../model/useSftpFeedback';
import { useSftpFileOperations } from '../model/useSftpFileOperations';
import { useSftpSelection } from '../model/useSftpSelection';
import { nextSortState, sortRows, toLocalRows, toRemoteRows } from '../model/sftp-view-model';
import styles from './SFTPView.module.css';

function SFTPView() {
  const sessions = useSftpStore((s) => s.sessions);
  const activeId = useSftpStore((s) => s.activeId);
  const setCurrentPath = useSftpStore((s) => s.setCurrentPath);
  const setSessionStatus = useSftpStore((s) => s.setSessionStatus);
  const setSelectionSummary = useSftpStore((s) => s.setSelectionSummary);
  const activeSession = sessions.find((session) => session.connectionId === activeId);
  const activeConnectionId = activeSession?.connectionId;
  const activeConnectionAttempt = activeSession?.connectionAttempt ?? 0;
  const activeRemotePath = activeSession?.currentPath || activeSession?.initialPath || '/';
  const [localSort, setLocalSort] = useState<SortState>({ key: 'name', direction: 'asc' });
  const [remoteSort, setRemoteSort] = useState<SortState>({ key: 'name', direction: 'asc' });

  const { toasts, confirmState, showToast, requestConfirm, closeConfirm } = useSftpFeedback();
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
            path={localPath || localListing?.path || '~'}
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
            onRefresh={() => void loadLocalDirectory(localPath || localListing?.path, true)}
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
