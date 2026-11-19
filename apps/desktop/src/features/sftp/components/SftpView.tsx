import { lazy, Suspense, useEffect, useMemo, useState } from 'react';
import { LocalFilePane, RemoteFilePane } from './SftpFilePanes';
import { useSftpStore } from '../model/sftp-store';
import type { SortState } from '../types/sftp-types';
import { useSftpDirectoryBrowser } from '../model/useSftpDirectoryBrowser';
import { useSftpFeedback } from '../model/useSftpFeedback';
import { useSftpFileEditor } from '../model/useSftpFileEditor';
import { useSftpFileOperations } from '../model/useSftpFileOperations';
import { useSftpSelection } from '../model/useSftpSelection';
import { nextSortState, sortRows, toLocalRows, toRemoteRows } from '../model/sftp-view-model';
import styles from './SFTPView.module.css';

// CodeMirror 体积较大，只有真正打开文件时才加载编辑器 chunk；浏览/终端启动不承担成本。
const LazyFileEditorView = lazy(() =>
  import('./editor/RemoteFileEditor').then((module) => ({ default: module.FileEditorView }))
);

/** visible=false 时仍保持挂载：编辑器内容与浏览状态不因路由切换销毁。 */
function SFTPView({ visible = true }: { visible?: boolean }) {
  const sessions = useSftpStore((s) => s.sessions);
  const activeId = useSftpStore((s) => s.activeId);
  const initialLocalPath = useSftpStore((s) => s.localPath);
  const rememberLocalPath = useSftpStore((s) => s.setLocalPath);
  const setCurrentPath = useSftpStore((s) => s.setCurrentPath);
  const setSessionStatus = useSftpStore((s) => s.setSessionStatus);
  const markFileViewVisited = useSftpStore((s) => s.markFileViewVisited);
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
    onFileSaved,
  } = useSftpDirectoryBrowser({
    enabled: visible,
    activeConnectionId,
    activeConnectionAttempt,
    activeRemotePath,
    initialLocalPath,
    rememberLocalPath,
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

  // 在线编辑器状态机：打开/保存/冲突处理与目录刷新都经由文件操作同源的回调。
  const fileEditor = useSftpFileEditor({
    onFileSaved,
    showToast,
  });

  useEffect(() => {
    // 只在用户真正进入文件页后保留该工作区；隐藏挂载不算一次访问。
    if (visible) markFileViewVisited();
  }, [visible, markFileViewVisited]);

  // 激活编辑标签时主内容区切换为全尺寸编辑器；有会话时双栏管理器仅隐藏不销毁。
  // 本地编辑器在最后一个 SFTP 会话关闭后依然可用，优先于「选择远程连接」空态。
  const editorVisible = fileEditor.activeEditor !== null;

  return (
    <div className={`${styles.view} ${visible ? '' : styles.hiddenView}`}>
      {!editorVisible && !activeSession ? (
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
        <main
          className={`${styles.manager} ${editorVisible ? styles.managerHidden : ''}`}
          hidden={editorVisible}
        >
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
            onEditFile={(row) => void fileEditor.openFile({ kind: 'local', path: row.path ?? '' })}
            onEditBlocked={showToast}
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
            onEditFile={(row) => {
              if (!activeSession) return;
              void fileEditor.openFile({
                kind: 'remote',
                path: row.path ?? '',
                connectionId: activeSession.connectionId,
              });
            }}
            onEditBlocked={showToast}
          />
        </main>
      )}
      {fileEditor.activeEditor && (
        <Suspense fallback={<div className={styles.editorLoading}>加载编辑器中…</div>}>
          <LazyFileEditorView
            file={fileEditor.activeEditor}
            onChange={(value) => fileEditor.changeContent(fileEditor.activeEditor!.id, value)}
            onSave={() =>
              fileEditor.activeEditor && void fileEditor.saveFile(fileEditor.activeEditor.id)
            }
          />
        </Suspense>
      )}
      {fileEditor.conflictPendingEditor && (
        <div className={styles.confirmLayer}>
          <section className={styles.confirmDialog} role="dialog" aria-modal="true">
            <h3>文件已变更</h3>
            <p>
              文件 {fileEditor.conflictPendingEditor.name} 在编辑期间被修改或删除。覆盖将丢弃
              {fileEditor.conflictPendingEditor.source.kind === 'remote' ? '服务器' : '磁盘'}当前
              版本；加载最新版本将丢弃你的未保存修改。
            </p>
            <div className={styles.confirmActions}>
              <button
                type="button"
                onClick={() =>
                  void fileEditor.resolveConflict(fileEditor.conflictPendingEditor!.id, 'cancel')
                }
              >
                取消
              </button>
              <button
                type="button"
                onClick={() =>
                  void fileEditor.resolveConflict(fileEditor.conflictPendingEditor!.id, 'reload')
                }
              >
                加载最新版本
              </button>
              <button
                type="button"
                className={styles.dangerButton}
                onClick={() =>
                  void fileEditor.resolveConflict(fileEditor.conflictPendingEditor!.id, 'overwrite')
                }
              >
                覆盖保存
              </button>
            </div>
          </section>
        </div>
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
export { SFTPView as SftpView };
