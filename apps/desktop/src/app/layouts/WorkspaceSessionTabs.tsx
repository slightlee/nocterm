import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type WheelEvent,
} from 'react';
import { useNavigate } from 'react-router-dom';

import { AiPanelToggle } from '../../features/ai';
import { useSettings } from '../../features/settings';
import {
  disconnectSftpSession,
  beginFileEditorClose,
  FileIcon,
  getFileKindForName,
  getSftpErrorMessage,
  useFileEditorStore,
  useSftpFeedback,
  useSftpStore,
} from '../../features/sftp';
import { useTerminalStore, type TerminalSessionId } from '../../features/terminal';
import { PanelSideIcon } from '../../shared/components/PanelSideIcon';
import { TabContextMenu } from '../../shared/components/TabContextMenu';
import {
  createWorkspaceSessionTabKeys,
  editorWorkspaceTabKey,
  expandWorkspaceTabCloseKeys,
  preserveWorkspaceEditorParent,
  resolveWorkspaceTabCloseFallback,
  formatWorkspaceSessionNumber,
  resolveWorkspaceTabOverflow,
  sftpWorkspaceTabKey,
  terminalWorkspaceTabKey,
  type WorkspaceSessionTabKey,
} from '../model/workspace-session-tabs';
import { onAppMenuAction } from '../platform/app-menu-client';
import styles from './TerminalWorkspace.module.css';
import { WorkspaceSessionIndex, type WorkspaceSessionIndexEntry } from './WorkspaceSessionIndex';
import { WorkspaceSessionTypeIcon } from './WorkspaceSessionTypeIcon';

interface WorkspaceSessionTabsProps {
  activeView: 'terminal' | 'sftp';
  aiOpen: boolean;
  compactTop: boolean;
  onCreateLocalTerminal: () => void;
  onToggleAi: () => void;
  sidebarCollapsed: boolean;
  onToggleSidebar: () => void;
}

function terminalStatus(
  sessionId: TerminalSessionId,
  activeId: TerminalSessionId | null,
  statuses: ReturnType<typeof useTerminalStore.getState>['statuses'],
  activeStatus: ReturnType<typeof useTerminalStore.getState>['status']
) {
  return statuses[String(sessionId)] ?? (sessionId === activeId ? activeStatus : 'connected');
}

/** 应用层统一编排终端与文件标签；各 Feature 仍独立持有会话和关闭生命周期。 */
export function WorkspaceSessionTabs({
  activeView,
  aiOpen,
  compactTop,
  onCreateLocalTerminal,
  onToggleAi,
  sidebarCollapsed,
  onToggleSidebar,
}: WorkspaceSessionTabsProps) {
  const navigate = useNavigate();
  const { terminalThemeId } = useSettings();
  const terminalSessions = useTerminalStore((state) => state.sessions);
  const activeTerminalId = useTerminalStore((state) => state.activeId);
  const terminalStatuses = useTerminalStore((state) => state.statuses);
  const activeTerminalStatus = useTerminalStore((state) => state.status);
  const activateTerminal = useTerminalStore((state) => state.activateConnection);
  const closeTerminal = useTerminalStore((state) => state.closeConnection);
  const sftpSessions = useSftpStore((state) => state.sessions);
  const activeSftpId = useSftpStore((state) => state.activeId);
  const activateSftp = useSftpStore((state) => state.setActive);
  const closeSftp = useSftpStore((state) => state.closeSession);
  const hasRunningTransfers = useSftpStore((state) => state.hasRunningTransfers);
  const editors = useFileEditorStore((state) => state.editors);
  const activeEditorId = useFileEditorStore((state) => state.activeEditorId);
  const activateEditor = useFileEditorStore((state) => state.activateEditor);
  const closeEditors = useFileEditorStore((state) => state.closeEditors);
  // 关闭确认与异步断开期间禁止叠加另一批关闭请求，避免确认状态互相覆盖。
  const closingRef = useRef(false);
  const { toasts, confirmState, showToast, requestConfirm, closeConfirm } = useSftpFeedback();
  const tabViewportRef = useRef<HTMLDivElement>(null);
  const [tabOverflow, setTabOverflow] = useState({ left: false, right: false });
  const tabKeys = createWorkspaceSessionTabKeys(
    terminalSessions.map((session) => session.id),
    sftpSessions.map((session) => session.connectionId),
    editors.map((editor) => editor.id)
  );
  const activeKey =
    activeEditorId !== null && activeView === 'sftp'
      ? editorWorkspaceTabKey(activeEditorId)
      : activeEditorId === null && activeView === 'terminal' && activeTerminalId !== null
        ? terminalWorkspaceTabKey(activeTerminalId)
        : activeEditorId === null && activeView === 'sftp' && activeSftpId !== null
          ? sftpWorkspaceTabKey(activeSftpId)
          : null;
  const editorsByKey = useMemo(
    () => new Map(editors.map((editor) => [editorWorkspaceTabKey(editor.id), editor])),
    [editors]
  );
  const terminalSessionsByKey = useMemo(
    () =>
      new Map(terminalSessions.map((session) => [terminalWorkspaceTabKey(session.id), session])),
    [terminalSessions]
  );
  const sftpSessionsByKey = useMemo(
    () =>
      new Map(sftpSessions.map((session) => [sftpWorkspaceTabKey(session.connectionId), session])),
    [sftpSessions]
  );
  // 弹层只消费统一的展示模型；终端与 SFTP Store 继续各自负责会话生命周期。
  const sessionIndexEntries: WorkspaceSessionIndexEntry[] = [
    ...terminalSessions.map((session) => {
      const status = terminalStatus(
        session.id,
        activeTerminalId,
        terminalStatuses,
        activeTerminalStatus
      );
      if (session.kind === 'remote') {
        const number = formatWorkspaceSessionNumber(session.terminalNumber);
        return {
          key: terminalWorkspaceTabKey(session.id),
          groupId: `remote:${session.connectionId}`,
          groupLabel: session.name,
          kind: 'terminal' as const,
          label: `终端 ${number}`,
          searchText: `${session.username} ${session.host} ${session.port}`,
          status,
          current: activeKey === terminalWorkspaceTabKey(session.id),
          title: `${session.name} · 终端 ${number} · ${session.username}@${session.host}:${session.port}`,
        };
      }

      return {
        key: terminalWorkspaceTabKey(session.id),
        groupId: 'local',
        groupLabel: '本地',
        kind: 'terminal' as const,
        label: session.name,
        searchText: 'local shell 本地终端',
        status,
        current: activeKey === terminalWorkspaceTabKey(session.id),
        title: session.name,
      };
    }),
    ...sftpSessions.map((session) => ({
      key: sftpWorkspaceTabKey(session.connectionId),
      groupId: `remote:${session.connectionId}`,
      groupLabel: session.connectionName,
      kind: 'file' as const,
      label: '文件',
      searchText: `${session.username} ${session.host} ${session.port} ${session.currentPath}`,
      status: session.status,
      current: activeKey === sftpWorkspaceTabKey(session.connectionId),
      title: `${session.connectionName} · 文件 · ${session.username}@${session.host}:${session.port}`,
    })),
    ...editors.map((editor): WorkspaceSessionIndexEntry => ({
      key: editorWorkspaceTabKey(editor.id),
      groupId: editor.source.kind === 'remote' ? `remote:${editor.source.connectionId}` : 'local',
      groupLabel:
        editor.source.kind === 'remote'
          ? (sftpSessionsByKey.get(sftpWorkspaceTabKey(editor.source.connectionId))
              ?.connectionName ?? '远程')
          : '本地',
      kind: 'file',
      label: `${editor.source.kind === 'local' ? '本地' : '远程'} · ${editor.name}${editor.dirty ? ' · 未保存' : ''}`,
      searchText: editor.source.path,
      // 编辑器不扩展连接状态协议；加载映射为 connecting，其余为可用。
      status: editor.status === 'loading' ? 'connecting' : 'connected',
      current: activeKey === editorWorkspaceTabKey(editor.id),
      title: `${editor.source.kind === 'local' ? '本地' : '远程'} · ${editor.source.path}`,
    })),
  ];

  const updateTabOverflow = useCallback(() => {
    const viewport = tabViewportRef.current;
    if (!viewport) return;
    const next = resolveWorkspaceTabOverflow(viewport);
    setTabOverflow((current) =>
      current.left === next.left && current.right === next.right ? current : next
    );
  }, []);

  useLayoutEffect(() => {
    const viewport = tabViewportRef.current;
    if (!viewport) return;

    updateTabOverflow();
    // 同时观察视口和内容轨道，覆盖窗口缩放、标签增删与名称变化。
    const observer = new ResizeObserver(updateTabOverflow);
    observer.observe(viewport);
    if (viewport.firstElementChild instanceof HTMLElement) {
      observer.observe(viewport.firstElementChild);
    }
    return () => observer.disconnect();
  }, [editors.length, sftpSessions.length, terminalSessions.length, updateTabOverflow]);

  useEffect(() => {
    // 新建或从侧栏切换会话时，活动标签必须始终回到横向可视区域。
    const activeTab = tabViewportRef.current?.querySelector<HTMLElement>(
      '[role="tab"][aria-selected="true"]'
    );
    const tabContainer = activeTab?.parentElement;
    if (typeof tabContainer?.scrollIntoView === 'function') {
      tabContainer.scrollIntoView({ block: 'nearest', inline: 'nearest' });
    }
    const frame = window.requestAnimationFrame(updateTabOverflow);
    return () => window.cancelAnimationFrame(frame);
  }, [activeEditorId, activeSftpId, activeTerminalId, activeView, updateTabOverflow]);

  const handleTabWheel = (event: WheelEvent<HTMLDivElement>) => {
    if (Math.abs(event.deltaY) <= Math.abs(event.deltaX)) return;
    event.currentTarget.scrollLeft += event.deltaY;
    event.preventDefault();
  };

  const scrollTabs = (direction: -1 | 1) => {
    const viewport = tabViewportRef.current;
    if (!viewport) return;
    // 与原型一致：小窗口至少翻 160px，大窗口翻动约三分之二屏。
    viewport.scrollBy({
      left: direction * Math.max(160, viewport.clientWidth * 0.65),
      behavior: 'smooth',
    });
  };

  const activateSession = useCallback(
    (key: WorkspaceSessionTabKey) => {
      // 所有入口（标签、索引、关闭回退）共享同一路由与 Store 激活顺序。
      const editor = editorsByKey.get(key);
      if (editor) {
        // setActive 会清除编辑器选择，必须先选父连接再恢复目标编辑器。
        if (editor.source.kind === 'remote') activateSftp(editor.source.connectionId);
        activateEditor(editor.id);
        navigate('/sftp');
        return;
      }
      const terminalSession = terminalSessionsByKey.get(key);
      if (terminalSession) {
        activateEditor(null);
        activateTerminal(terminalSession.id);
        navigate('/connections');
        return;
      }
      const sftpSession = sftpSessionsByKey.get(key);
      if (sftpSession) {
        activateSftp(sftpSession.connectionId);
        activateEditor(null);
        navigate('/sftp');
      }
    },
    [
      activateEditor,
      activateSftp,
      activateTerminal,
      editorsByKey,
      navigate,
      sftpSessionsByKey,
      terminalSessionsByKey,
    ]
  );

  const createLocalTerminal = () => {
    activateEditor(null);
    onCreateLocalTerminal();
  };

  const closeFromContextMenu = (keys: WorkspaceSessionTabKey[], target: WorkspaceSessionTabKey) => {
    void handleCloseTabs(preserveWorkspaceEditorParent(keys, target, editors));
  };

  const handleCloseTabs = useCallback(
    async (keys: WorkspaceSessionTabKey[]) => {
      if (closingRef.current) return;
      closingRef.current = true;
      let releaseEditors: (() => void) | null = null;
      try {
        const expandedKeys = expandWorkspaceTabCloseKeys(
          keys,
          useFileEditorStore.getState().editors
        );
        const terminalIds = expandedKeys
          .map((key) => terminalSessionsByKey.get(key)?.id)
          .filter((id): id is string => id !== undefined);
        const sftpIds = expandedKeys
          .map((key) => sftpSessionsByKey.get(key)?.connectionId)
          .filter((id): id is string => id !== undefined);
        const closingEditors = () =>
          useFileEditorStore
            .getState()
            .editors.filter((editor) => expandedKeys.includes(editorWorkspaceTabKey(editor.id)));
        if (closingEditors().some((editor) => editor.status === 'saving')) {
          showToast('文件正在保存，请等待保存完成后再关闭。');
          return;
        }

        if (sftpIds.length > 0 && hasRunningTransfers(sftpIds)) {
          const confirmed = await requestConfirm({
            title: '关闭文件标签',
            message:
              sftpIds.length === 1
                ? '该标签仍有文件正在传输，关闭会取消传输并断开连接。是否继续？'
                : '待关闭标签中仍有文件正在传输，关闭会取消相关传输并断开连接。是否继续？',
            confirmLabel: '关闭标签',
            danger: true,
          });
          if (!confirmed) return;
        }

        releaseEditors = await beginFileEditorClose(
          closingEditors().map((editor) => editor.id),
          requestConfirm,
          showToast
        );
        if (!releaseEditors) return;
        const closedKeys: WorkspaceSessionTabKey[] = [];
        const independentEditorIds = closingEditors()
          .filter(
            (editor) =>
              editor.source.kind === 'local' || !sftpIds.includes(editor.source.connectionId)
          )
          .map((editor) => editor.id);
        closeEditors(independentEditorIds);
        closedKeys.push(...independentEditorIds.map(editorWorkspaceTabKey));
        terminalIds.forEach((id) => closeTerminal(id));
        closedKeys.push(...terminalIds.map(terminalWorkspaceTabKey));
        let failureCount = 0;
        for (const connectionId of sftpIds) {
          try {
            await disconnectSftpSession(connectionId);
            // 断开失败必须保留整个父子组；只在后端确认成功后释放编辑内容。
            const dependentIds = closingEditors()
              .filter(
                (editor) =>
                  editor.source.kind === 'remote' && editor.source.connectionId === connectionId
              )
              .map((editor) => editor.id);
            closeEditors(dependentIds);
            closeSftp(connectionId);
            closedKeys.push(
              sftpWorkspaceTabKey(connectionId),
              ...dependentIds.map(editorWorkspaceTabKey)
            );
          } catch (error) {
            failureCount += 1;
            if (sftpIds.length === 1) {
              showToast(`断开 SFTP 失败：${getSftpErrorMessage(error)}`);
            }
          }
        }
        if (sftpIds.length > 1 && failureCount > 0) {
          showToast(`有 ${failureCount} 个文件标签关闭失败，请重试。`);
        }
        const fallback = resolveWorkspaceTabCloseFallback(tabKeys, activeKey, closedKeys);
        if (activeKey && closedKeys.includes(activeKey)) {
          if (fallback) activateSession(fallback);
          else activateEditor(null);
        }
      } finally {
        releaseEditors?.();
        closingRef.current = false;
      }
    },
    [
      activateEditor,
      activateSession,
      activeKey,
      closeEditors,
      closeSftp,
      closeTerminal,
      hasRunningTransfers,
      requestConfirm,
      sftpSessionsByKey,
      showToast,
      tabKeys,
      terminalSessionsByKey,
    ]
  );

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    void onAppMenuAction((action) => {
      if (action !== 'close-current-tab') return;
      if (activeKey) void handleCloseTabs([activeKey]);
    })
      .then((dispose) => {
        if (disposed) dispose();
        else unlisten = dispose;
      })
      .catch(() => {
        console.error('Workspace menu subscription failed');
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [activeKey, handleCloseTabs]);

  return (
    <div className={styles.tabThemeScope} data-terminal-theme={terminalThemeId}>
      <header
        className={`${styles.tabBar} nocterm-terminal-surface ${
          sidebarCollapsed && !compactTop ? styles.tabBarWindowControlsInset : ''
        }`}
      >
        <div
          aria-hidden={!sidebarCollapsed}
          className={`${styles.collapsedControls} ${
            sidebarCollapsed ? styles.collapsedControlsVisible : ''
          }`}
          inert={!sidebarCollapsed}
        >
          {/* Windows 的展开入口固定在标题栏品牌右侧，此处仅 macOS 需要。 */}
          {!compactTop ? (
            <button
              aria-label="显示连接面板"
              className={styles.panelToggle}
              onClick={onToggleSidebar}
              title="显示连接面板"
              type="button"
            >
              <PanelSideIcon className={styles.toggleIcon} side="left" />
            </button>
          ) : null}
          <button
            aria-label="新建本地终端"
            className={styles.panelToggle}
            onClick={createLocalTerminal}
            title="新建本地终端"
            type="button"
          >
            <svg
              aria-hidden="true"
              className={styles.collapsedLocalTerminalIcon}
              viewBox="0 0 24 24"
            >
              <path d="m4 7 5 5-5 5M11 17h5M19 4v7M15.5 7.5h7" />
            </svg>
          </button>
        </div>

        <div className={styles.tabNavigation}>
          <button
            aria-label="向左浏览会话"
            className={`${styles.tabScrollButton} ${styles.tabScrollPrevious} ${tabOverflow.left ? styles.tabScrollVisible : ''}`}
            disabled={!tabOverflow.left}
            onClick={() => scrollTabs(-1)}
            title="向左浏览会话"
            type="button"
          >
            <svg aria-hidden="true" viewBox="0 0 20 20">
              <path d="m12 5-5 5 5 5" />
            </svg>
          </button>

          <div
            aria-label="终端与文件会话"
            className={styles.remoteTabViewport}
            onScroll={updateTabOverflow}
            onWheel={handleTabWheel}
            ref={tabViewportRef}
          >
            <div className={styles.remoteTabTrack}>
              <div aria-label="终端与文件会话" className={styles.sessionTabList} role="tablist">
                {terminalSessions.map((session) => {
                  const selected = activeKey === terminalWorkspaceTabKey(session.id);
                  const currentStatus = terminalStatus(
                    session.id,
                    activeTerminalId,
                    terminalStatuses,
                    activeTerminalStatus
                  );
                  const production = session.kind === 'remote' && /prod|生产/i.test(session.name);
                  const sessionNumber =
                    session.kind === 'remote'
                      ? formatWorkspaceSessionNumber(session.terminalNumber)
                      : null;
                  const tabTitle = sessionNumber
                    ? `${sessionNumber} · ${session.name}`
                    : session.name;
                  const accessibleName = sessionNumber
                    ? `${session.name} 的终端 ${sessionNumber}`
                    : session.name;
                  const key = terminalWorkspaceTabKey(session.id);
                  return (
                    <TabContextMenu
                      ids={tabKeys}
                      key={key}
                      targetId={key}
                      onClose={(keys) => closeFromContextMenu(keys, key)}
                    >
                      <div
                        className={`${styles.tab} ${selected ? styles.active : ''}`}
                        data-no-window-drag="true"
                      >
                        <button
                          aria-label={`切换到 ${accessibleName}`}
                          aria-selected={selected}
                          className={styles.tabSelect}
                          onClick={() => activateSession(key)}
                          role="tab"
                          title={tabTitle}
                          type="button"
                        >
                          <span className={styles.tabDot} data-status={currentStatus} />
                          <WorkspaceSessionTypeIcon
                            className={styles.terminalIcon}
                            kind="terminal"
                          />
                          {sessionNumber ? (
                            <span aria-hidden="true" className={styles.sessionNumber}>
                              {sessionNumber}
                            </span>
                          ) : null}
                          <span className={styles.tabName}>{session.name}</span>
                          {production ? <span className={styles.tabBadge}>PROD</span> : null}
                        </button>
                        <button
                          aria-label={`关闭 ${accessibleName}`}
                          className={styles.tabClose}
                          onClick={() => void handleCloseTabs([key])}
                          title="关闭终端"
                          type="button"
                        >
                          ×
                        </button>
                      </div>
                    </TabContextMenu>
                  );
                })}

                {sftpSessions.map((session) => {
                  const selected = activeKey === sftpWorkspaceTabKey(session.connectionId);
                  const key = sftpWorkspaceTabKey(session.connectionId);
                  const title = `${session.connectionName} · 文件 · ${session.username}@${session.host}:${session.port}`;
                  return (
                    <TabContextMenu
                      ids={tabKeys}
                      key={key}
                      targetId={key}
                      onClose={(keys) => closeFromContextMenu(keys, key)}
                    >
                      <div
                        className={`${styles.tab} ${styles.fileTab} ${selected ? styles.active : ''}`}
                        data-no-window-drag="true"
                      >
                        <button
                          aria-label={`切换到 ${session.connectionName} 的文件管理`}
                          aria-selected={selected}
                          className={styles.tabSelect}
                          onClick={() => activateSession(key)}
                          role="tab"
                          title={title}
                          type="button"
                        >
                          <span className={styles.tabDot} data-status={session.status} />
                          <WorkspaceSessionTypeIcon className={styles.fileIcon} kind="file" />
                          <span className={styles.tabName}>{session.connectionName}</span>
                        </button>
                        <button
                          aria-label={`关闭 ${session.connectionName} 的文件管理`}
                          className={styles.tabClose}
                          onClick={() => void handleCloseTabs([key])}
                          title="关闭文件管理"
                          type="button"
                        >
                          ×
                        </button>
                      </div>
                    </TabContextMenu>
                  );
                })}
                {editors.map((editor) => {
                  const key = editorWorkspaceTabKey(editor.id);
                  const selected = activeKey === key;
                  const local = editor.source.kind === 'local';
                  return (
                    <TabContextMenu
                      ids={tabKeys}
                      key={key}
                      targetId={key}
                      onClose={(keys) => closeFromContextMenu(keys, key)}
                    >
                      <div
                        className={`${styles.tab} ${styles.fileTab} ${selected ? styles.active : ''}`}
                        data-no-window-drag="true"
                      >
                        <button
                          aria-label={`切换到 ${local ? '本地' : '远程'}文件 ${editor.name}${editor.dirty ? '，未保存' : ''}`}
                          aria-selected={selected}
                          className={styles.tabSelect}
                          onClick={() => activateSession(key)}
                          role="tab"
                          title={`${local ? '本地' : '远程'} · ${editor.source.path}`}
                          type="button"
                        >
                          {/* 标签图标表达文件类型（与文件面板同一套分类），来源由悬浮提示说明。 */}
                          <span aria-hidden="true" className={styles.editorSourceBadge}>
                            <FileIcon kind={getFileKindForName(editor.name)} />
                          </span>
                          <span className={styles.tabName}>{editor.name}</span>
                          {editor.status === 'saving' ? (
                            <span aria-label="正在保存" className={styles.editorSavingIndicator} />
                          ) : editor.dirty ? (
                            <span aria-label="未保存" className={styles.editorDirtyDot} />
                          ) : null}
                        </button>
                        <button
                          aria-label={`关闭 ${editor.name}`}
                          className={styles.tabClose}
                          disabled={editor.status === 'saving'}
                          onClick={() => void handleCloseTabs([key])}
                          title={editor.status === 'saving' ? '正在保存' : '关闭编辑器'}
                          type="button"
                        >
                          ×
                        </button>
                      </div>
                    </TabContextMenu>
                  );
                })}
              </div>

              {!sidebarCollapsed && tabKeys.length > 0 ? (
                <button
                  aria-label="新建本地终端"
                  className={styles.newLocalTabButton}
                  onClick={createLocalTerminal}
                  title="新建本地终端"
                  type="button"
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path d="M12 5v14M5 12h14" />
                  </svg>
                </button>
              ) : null}
            </div>
          </div>

          <button
            aria-label="向右浏览会话"
            className={`${styles.tabScrollButton} ${styles.tabScrollNext} ${tabOverflow.right ? styles.tabScrollVisible : ''}`}
            disabled={!tabOverflow.right}
            onClick={() => scrollTabs(1)}
            title="向右浏览会话"
            type="button"
          >
            <svg aria-hidden="true" viewBox="0 0 20 20">
              <path d="m8 5 5 5-5 5" />
            </svg>
          </button>
        </div>

        {sessionIndexEntries.length > 0 ? (
          <WorkspaceSessionIndex entries={sessionIndexEntries} onActivate={activateSession} />
        ) : null}

        {!aiOpen ? <AiPanelToggle expanded={false} onToggle={onToggleAi} /> : null}
      </header>

      {confirmState ? (
        <div className={styles.confirmLayer}>
          <section aria-modal="true" className={styles.confirmDialog} role="dialog">
            <h3>{confirmState.title}</h3>
            <p>{confirmState.message}</p>
            <div className={styles.confirmActions}>
              <button onClick={() => closeConfirm(false)} type="button">
                取消
              </button>
              <button
                className={confirmState.danger ? styles.dangerButton : styles.primaryButton}
                onClick={() => closeConfirm(true)}
                type="button"
              >
                {confirmState.confirmLabel || '确认'}
              </button>
            </div>
          </section>
        </div>
      ) : null}
      {toasts.length > 0 ? (
        <div className={styles.toastStack}>
          {toasts.map((toast) => (
            <div
              className={`${styles.toast} ${
                toast.tone === 'error'
                  ? styles.toastError
                  : toast.tone === 'success'
                    ? styles.toastSuccess
                    : ''
              }`}
              key={toast.id}
            >
              {toast.message}
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}
