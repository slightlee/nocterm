import { requestConnectionDialog, useConnectionPresenceStore } from '../../features/connections';
import type { WheelEvent } from 'react';
import {
  LocalTerminal,
  SshTerminal,
  useTerminalStore,
  type TerminalSessionId,
} from '../../features/terminal';
import { TabContextMenu } from '../../shared/components/TabContextMenu';
import styles from './TerminalWorkspace.module.css';

const PRIMARY_LOCAL_TERMINAL_ID = 'local:default';
const EXTRA_LOCAL_TERMINAL_PREFIX = 'local:extra:';

interface TerminalWorkspaceProps {
  sidebarCollapsed: boolean;
  onToggleSidebar: () => void;
}

function sessionStatus(
  sessionId: TerminalSessionId,
  activeId: TerminalSessionId | null,
  statuses: ReturnType<typeof useTerminalStore.getState>['statuses'],
  activeStatus: ReturnType<typeof useTerminalStore.getState>['status']
) {
  return statuses[sessionId] ?? (sessionId === activeId ? activeStatus : 'connected');
}

/** 终端容器沿用旧客户端结构，负责多会话切换、状态反馈和重连入口。 */
export function TerminalWorkspace({ sidebarCollapsed, onToggleSidebar }: TerminalWorkspaceProps) {
  const sessions = useTerminalStore((state) => state.sessions);
  const activeId = useTerminalStore((state) => state.activeId);
  const status = useTerminalStore((state) => state.status);
  const statuses = useTerminalStore((state) => state.statuses);
  const reconnectNonces = useTerminalStore((state) => state.reconnectNonces);
  const closeConnection = useTerminalStore((state) => state.closeConnection);
  const activateConnection = useTerminalStore((state) => state.activateConnection);
  const reconnectConnection = useTerminalStore((state) => state.reconnectConnection);
  const openLocalSession = useTerminalStore((state) => state.openLocalSession);
  const hasConnections = useConnectionPresenceStore((state) => state.connectionCount > 0);
  const emptyTitle = hasConnections ? '还没有打开的终端' : '开始第一个会话';
  const emptyDescription = hasConnections
    ? '双击左侧连接打开远程终端，或先打开本地 Shell。'
    : '打开本地终端即可使用，也可以添加远程服务器。';
  const sessionIds = sessions.map((session) => session.id);

  const handleTabWheel = (event: WheelEvent<HTMLDivElement>) => {
    if (Math.abs(event.deltaY) <= Math.abs(event.deltaX)) return;
    event.currentTarget.scrollLeft += event.deltaY;
    event.preventDefault();
  };

  const openPrimaryLocalTerminal = () => {
    const primary = sessions.find((session) => session.id === PRIMARY_LOCAL_TERMINAL_ID);
    if (primary) {
      activateConnection(primary.id);
      return;
    }
    openLocalSession(PRIMARY_LOCAL_TERMINAL_ID, '本地终端');
  };

  const createLocalTerminal = () => {
    const localCount = sessions.filter((session) => session.kind === 'local').length;
    const name = localCount === 0 ? '本地终端' : `本地终端 ${localCount + 1}`;
    openLocalSession(`${EXTRA_LOCAL_TERMINAL_PREFIX}${crypto.randomUUID()}`, name);
  };

  return (
    <div className={styles.container}>
      <header className={styles.tabBar}>
        <button
          aria-label={sidebarCollapsed ? '显示连接面板' : '隐藏连接面板'}
          aria-pressed={!sidebarCollapsed}
          className={styles.panelToggle}
          onClick={onToggleSidebar}
          title={sidebarCollapsed ? '显示连接面板' : '隐藏连接面板'}
          type="button"
        >
          <span
            aria-hidden="true"
            className={`${styles.toggleIcon} ${
              sidebarCollapsed ? styles.toggleIconLeft : styles.toggleIconRight
            }`}
          />
        </button>

        <div
          className={styles.remoteTabViewport}
          role="tablist"
          aria-label="SSH 终端会话"
          onWheel={handleTabWheel}
        >
          <div className={styles.remoteTabTrack}>
            {sessions.map((session) => {
              const selected = session.id === activeId;
              const currentStatus = sessionStatus(session.id, activeId, statuses, status);
              const production = session.kind === 'remote' && /prod|生产/i.test(session.name);
              const primaryLocal = session.id === PRIMARY_LOCAL_TERMINAL_ID;
              return (
                <TabContextMenu
                  ids={sessionIds}
                  key={session.id}
                  targetId={session.id}
                  onClose={(ids) => ids.forEach((id) => closeConnection(id))}
                >
                  <div
                    className={`${styles.tab} ${primaryLocal ? styles.localTerminalTab : ''} ${selected ? styles.active : ''}`}
                    data-no-window-drag="true"
                  >
                    <button
                      aria-selected={selected}
                      className={styles.tabSelect}
                      onClick={() => activateConnection(session.id)}
                      role="tab"
                      type="button"
                    >
                      <span className={styles.tabDot} data-status={currentStatus} />
                      {primaryLocal ? (
                        <svg
                          className={styles.localTerminalIcon}
                          viewBox="0 0 24 24"
                          aria-hidden="true"
                        >
                          <polyline points="4 17 10 11 4 5" />
                          <line x1="12" y1="19" x2="20" y2="19" />
                        </svg>
                      ) : null}
                      <span className={styles.tabName}>{session.name}</span>
                      {production ? <span className={styles.tabBadge}>PROD</span> : null}
                    </button>
                    <button
                      aria-label={`关闭 ${session.name}`}
                      className={styles.tabClose}
                      onClick={() => closeConnection(session.id)}
                      title="关闭终端"
                      type="button"
                    >
                      ×
                    </button>
                  </div>
                </TabContextMenu>
              );
            })}
            {sessions.length > 0 ? (
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
      </header>

      <div className={styles.terminalArea}>
        {sessions.length > 0 ? (
          <>
            {sessions.map((session) => (
              <div
                className={session.id === activeId ? styles.terminalWrapper : styles.hidden}
                key={`${session.id}-${reconnectNonces[String(session.id)] ?? 0}`}
              >
                {session.kind === 'local' ? (
                  <LocalTerminal active={session.id === activeId} sessionId={session.id} />
                ) : (
                  <SshTerminal active={session.id === activeId} connection={session} />
                )}
              </div>
            ))}
            {activeId !== null && ['closed', 'error'].includes(statuses[activeId] ?? status) ? (
              <button
                className={styles.reconnectBtn}
                onClick={() => reconnectConnection(activeId)}
                type="button"
              >
                重新连接
              </button>
            ) : null}
          </>
        ) : (
          <section className={styles.emptyState}>
            <svg
              className={styles.emptyIcon}
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.5"
              aria-hidden="true"
            >
              <rect x="2" y="3" width="20" height="18" rx="2" />
              <polyline points="6 9 10 13 6 17" />
              <line x1="14" y1="17" x2="18" y2="17" />
            </svg>
            <div className={styles.emptyTitle}>{emptyTitle}</div>
            <div className={styles.emptyDesc}>{emptyDescription}</div>
            <div className={styles.emptyActions}>
              <button
                className={styles.emptyPrimaryBtn}
                onClick={openPrimaryLocalTerminal}
                type="button"
              >
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  <polyline points="4 17 10 11 4 5" />
                  <line x1="12" y1="19" x2="20" y2="19" />
                </svg>
                打开本地终端
              </button>
              <button
                className={styles.emptySecondaryBtn}
                onClick={requestConnectionDialog}
                type="button"
              >
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  <path d="M12 5v14M5 12h14" />
                </svg>
                新建远程连接
              </button>
            </div>
          </section>
        )}
      </div>
    </div>
  );
}
