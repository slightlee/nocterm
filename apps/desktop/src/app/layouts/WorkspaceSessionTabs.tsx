import { useCallback, useEffect, useLayoutEffect, useRef, useState, type WheelEvent } from 'react';
import { useNavigate } from 'react-router-dom';

import { AiPanelToggle } from '../../features/ai';
import { useSettings } from '../../features/settings';
import {
  disconnectSftpSession,
  getSftpErrorMessage,
  useSftpFeedback,
  useSftpStore,
} from '../../features/sftp';
import { useTerminalStore, type TerminalSessionId } from '../../features/terminal';
import { TabContextMenu } from '../../shared/components/TabContextMenu';
import {
  createWorkspaceSessionTabKeys,
  formatWorkspaceSessionNumber,
  resolveWorkspaceTabOverflow,
  sftpWorkspaceTabKey,
  terminalWorkspaceTabKey,
  type WorkspaceSessionTabKey,
} from '../model/workspace-session-tabs';
import styles from './TerminalWorkspace.module.css';
import { WorkspaceSessionIndex, type WorkspaceSessionIndexEntry } from './WorkspaceSessionIndex';
import { WorkspaceSessionTypeIcon } from './WorkspaceSessionTypeIcon';

const EXTRA_LOCAL_TERMINAL_PREFIX = 'local:extra:';

interface WorkspaceSessionTabsProps {
  activeView: 'terminal' | 'sftp';
  aiOpen: boolean;
  compactTop: boolean;
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
  const openLocalSession = useTerminalStore((state) => state.openLocalSession);
  const sftpSessions = useSftpStore((state) => state.sessions);
  const activeSftpId = useSftpStore((state) => state.activeId);
  const activateSftp = useSftpStore((state) => state.setActive);
  const closeSftp = useSftpStore((state) => state.closeSession);
  const hasRunningTransfers = useSftpStore((state) => state.hasRunningTransfers);
  const { toasts, confirmState, showToast, requestConfirm, closeConfirm } = useSftpFeedback();
  const tabViewportRef = useRef<HTMLDivElement>(null);
  const [tabOverflow, setTabOverflow] = useState({ left: false, right: false });
  const tabKeys = createWorkspaceSessionTabKeys(
    terminalSessions.map((session) => session.id),
    sftpSessions.map((session) => session.connectionId)
  );
  const terminalSessionsByKey = new Map(
    terminalSessions.map((session) => [terminalWorkspaceTabKey(session.id), session])
  );
  const sftpSessionsByKey = new Map(
    sftpSessions.map((session) => [sftpWorkspaceTabKey(session.connectionId), session])
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
          current: activeView === 'terminal' && session.id === activeTerminalId,
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
        current: activeView === 'terminal' && session.id === activeTerminalId,
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
      current: activeView === 'sftp' && session.connectionId === activeSftpId,
      title: `${session.connectionName} · 文件 · ${session.username}@${session.host}:${session.port}`,
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
  }, [sftpSessions.length, terminalSessions.length, updateTabOverflow]);

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
  }, [activeSftpId, activeTerminalId, activeView, updateTabOverflow]);

  const handleTabWheel = (event: WheelEvent<HTMLDivElement>) => {
    if (Math.abs(event.deltaY) <= Math.abs(event.deltaX)) return;
    event.currentTarget.scrollLeft += event.deltaY;
    event.preventDefault();
  };

  const createLocalTerminal = () => {
    const localCount = terminalSessions.filter((session) => session.kind === 'local').length;
    const name = localCount === 0 ? '本地终端' : `本地终端 ${localCount + 1}`;
    openLocalSession(`${EXTRA_LOCAL_TERMINAL_PREFIX}${crypto.randomUUID()}`, name);
    navigate('/connections');
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

  const activateSession = (key: WorkspaceSessionTabKey) => {
    // 带类型前缀的 key 避免数据库连接 ID 在两个 Store 之间发生碰撞。
    const terminalSession = terminalSessionsByKey.get(key);
    if (terminalSession) {
      activateTerminal(terminalSession.id);
      navigate('/connections');
      return;
    }

    const sftpSession = sftpSessionsByKey.get(key);
    if (sftpSession) {
      activateSftp(sftpSession.connectionId);
      navigate('/sftp');
    }
  };

  const handleCloseTabs = async (keys: WorkspaceSessionTabKey[]) => {
    const terminalIds = keys
      .map((key) => terminalSessionsByKey.get(key)?.id)
      .filter((id): id is string => id !== undefined);
    const sftpIds = keys
      .map((key) => sftpSessionsByKey.get(key)?.connectionId)
      .filter((id): id is string => id !== undefined);

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

    terminalIds.forEach((id) => closeTerminal(id));
    let failureCount = 0;
    for (const connectionId of sftpIds) {
      try {
        await disconnectSftpSession(connectionId);
        closeSftp(connectionId);
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
  };

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
              <span
                aria-hidden="true"
                className={`${styles.toggleIcon} ${styles.toggleIconLeft}`}
              />
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
                  const selected = activeView === 'terminal' && session.id === activeTerminalId;
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
                      onClose={(keys) => void handleCloseTabs(keys)}
                    >
                      <div
                        className={`${styles.tab} ${selected ? styles.active : ''}`}
                        data-no-window-drag="true"
                      >
                        <button
                          aria-label={`切换到 ${accessibleName}`}
                          aria-selected={selected}
                          className={styles.tabSelect}
                          onClick={() => {
                            activateTerminal(session.id);
                            navigate('/connections');
                          }}
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
                  const selected = activeView === 'sftp' && session.connectionId === activeSftpId;
                  const key = sftpWorkspaceTabKey(session.connectionId);
                  const title = `${session.connectionName} · 文件 · ${session.username}@${session.host}:${session.port}`;
                  return (
                    <TabContextMenu
                      ids={tabKeys}
                      key={key}
                      targetId={key}
                      onClose={(keys) => void handleCloseTabs(keys)}
                    >
                      <div
                        className={`${styles.tab} ${styles.fileTab} ${selected ? styles.active : ''}`}
                        data-no-window-drag="true"
                      >
                        <button
                          aria-label={`切换到 ${session.connectionName} 的文件管理`}
                          aria-selected={selected}
                          className={styles.tabSelect}
                          onClick={() => {
                            activateSftp(session.connectionId);
                            navigate('/sftp');
                          }}
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
