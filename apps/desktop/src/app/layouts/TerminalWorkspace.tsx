import { useEffect } from 'react';

import { requestConnectionDialog, useConnectionPresenceStore } from '../../features/connections';
import { useSettings } from '../../features/settings';
import {
  LocalTerminal,
  SshTerminal,
  notifyTerminalAppearanceChanged,
  useTerminalStore,
} from '../../features/terminal';
import styles from './TerminalWorkspace.module.css';

const PRIMARY_LOCAL_TERMINAL_ID = 'local:default';

/** 终端容器沿用旧客户端结构，负责多会话切换、状态反馈和重连入口。 */
export function TerminalWorkspace() {
  const { terminalThemeId } = useSettings();

  // 主题作用域属性挂在本容器上；React 提交完成后再广播，终端 observer
  // 读到的 CSS 变量才保证是新主题的（见 terminal-appearance.ts 事件说明）。
  useEffect(() => {
    notifyTerminalAppearanceChanged();
  }, [terminalThemeId]);

  const sessions = useTerminalStore((state) => state.sessions);
  const activeId = useTerminalStore((state) => state.activeId);
  const status = useTerminalStore((state) => state.status);
  const statuses = useTerminalStore((state) => state.statuses);
  const reconnectNonces = useTerminalStore((state) => state.reconnectNonces);
  const activateConnection = useTerminalStore((state) => state.activateConnection);
  const reconnectConnection = useTerminalStore((state) => state.reconnectConnection);
  const openLocalSession = useTerminalStore((state) => state.openLocalSession);
  const hasConnections = useConnectionPresenceStore((state) => state.connectionCount > 0);
  const emptyTitle = '暂无打开的会话';
  const emptyDescription = hasConnections
    ? '双击左侧服务器打开终端，或打开本地终端。'
    : '打开本地终端，或先添加一台远程服务器。';
  const openPrimaryLocalTerminal = () => {
    const primary = sessions.find((session) => session.id === PRIMARY_LOCAL_TERMINAL_ID);
    if (primary) {
      activateConnection(primary.id);
      return;
    }
    openLocalSession(PRIMARY_LOCAL_TERMINAL_ID, '本地终端');
  };

  return (
    <div className={styles.container} data-terminal-theme={terminalThemeId}>
      {/* 主题作用域：属性在外层、surface 类在内层——主题规则的
          「[data-terminal-theme] .nocterm-terminal-surface」是后代选择器，
          属性与类同元素时自身不命中，变量会回落 :root 默认（切主题无效果）。 */}
      <div className={`${styles.themeScope} nocterm-terminal-surface`}>
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
                {!hasConnections ? (
                  <button
                    className={styles.emptySecondaryBtn}
                    onClick={requestConnectionDialog}
                    type="button"
                  >
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <path d="M12 5v14M5 12h14" />
                    </svg>
                    添加服务器
                  </button>
                ) : null}
              </div>
            </section>
          )}
        </div>
      </div>
    </div>
  );
}
