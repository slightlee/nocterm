import { useEffect, useState, type MouseEvent } from 'react';
import { Outlet, useLocation } from 'react-router-dom';

import { ConnectionList } from '../../features/connections';
import { AiPanel, useAiStore } from '../../features/ai';
import { useTopWindowDoubleClick, useTopWindowDrag } from '../../shared/hooks/useWindowDrag';
import { isWindowsDesktopRuntime } from '../../shared/lib/desktop-platform';
import { ActivityBar } from './ActivityBar';
import { Sidebar } from './Sidebar';
import { StatusBar } from './StatusBar';
import { TerminalWorkspace } from './TerminalWorkspace';
import { WindowsWindowControls } from './WindowsWindowControls';
import styles from './AppShell.module.css';

/** 旧客户端主布局：一级导航、连接侧栏、终端工作区和状态栏。 */
export function AppShell() {
  const location = useLocation();
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const connectionRoute = location.pathname.startsWith('/connections');
  const sidebarRoute = connectionRoute || location.pathname.startsWith('/sftp');
  const settingsRoute = location.pathname.startsWith('/settings');
  const handleTopWindowDrag = useTopWindowDrag();
  const handleTopWindowDoubleClick = useTopWindowDoubleClick();
  const windowsDesktop = isWindowsDesktopRuntime();
  const aiOpen = useAiStore((state) => state.open);
  const toggleAi = useAiStore((state) => state.toggle);

  /** 桌面客户端统一禁用 WebView 原生菜单；业务右键菜单仍由后续冒泡事件打开。 */
  const suppressBrowserContextMenu = (event: MouseEvent<HTMLDivElement>) => {
    event.preventDefault();
  };

  useEffect(() => {
    // 旧版只在空间不足时折叠，窗口恢复后尊重用户当前选择，不自动展开。
    const collapseWhenNeeded = () => {
      if (window.innerWidth < 548) setSidebarCollapsed(true);
    };
    collapseWhenNeeded();
    window.addEventListener('resize', collapseWhenNeeded);
    return () => window.removeEventListener('resize', collapseWhenNeeded);
  }, []);

  return (
    <div
      className={styles.root}
      onDoubleClickCapture={handleTopWindowDoubleClick}
      onMouseDownCapture={handleTopWindowDrag}
    >
      {/* Windows 无边框窗口：独立标题栏承载拖拽与窗口按钮，不遮挡任何内容区；
          macOS 保持 Overlay + 系统红绿灯，不渲染该栏。拖拽/双击处理挂在 root
          捕获层，两种窗口形态共享同一套顶部交互。 */}
      {windowsDesktop ? (
        <header className={styles.titleBar}>
          <div className={styles.titleBarBrand}>
            <svg aria-hidden="true" className={styles.titleBarLogo} viewBox="0 0 24 24">
              <rect
                fill="none"
                height="18"
                rx="4"
                stroke="currentColor"
                strokeWidth="1.8"
                width="18"
                x="3"
                y="3"
              />
              <polyline
                fill="none"
                points="7.5 9 11 12 7.5 15"
                stroke="currentColor"
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth="1.8"
              />
              <line
                stroke="currentColor"
                strokeLinecap="round"
                strokeWidth="1.8"
                x1="12.5"
                x2="16.5"
                y1="15.5"
                y2="15.5"
              />
            </svg>
            <span className={styles.titleBarName}>Nocterm</span>
          </div>
          <WindowsWindowControls />
        </header>
      ) : null}
      <div className={styles.app} onContextMenuCapture={suppressBrowserContextMenu}>
        <ActivityBar compactTop={windowsDesktop} />
        {sidebarRoute && !sidebarCollapsed ? (
          <Sidebar>
            <ConnectionList mode={location.pathname.startsWith('/sftp') ? 'sftp' : 'terminal'} />
          </Sidebar>
        ) : null}
        <section className={styles.workspace}>
          <div
            className={`${styles.terminalWorkspace} ${connectionRoute ? '' : styles.hiddenWorkspace}`}
          >
            <TerminalWorkspace
              sidebarCollapsed={sidebarCollapsed}
              onToggleSidebar={() => setSidebarCollapsed((collapsed) => !collapsed)}
            />
          </div>
          {!connectionRoute ? (
            <Outlet
              context={{
                sidebarCollapsed,
                toggleSidebar: () => setSidebarCollapsed((collapsed) => !collapsed),
              }}
            />
          ) : null}
          {!settingsRoute ? <StatusBar /> : null}
          {connectionRoute && !aiOpen ? (
            <button
              aria-label="显示 AI 助手"
              className={styles.aiReveal}
              onClick={toggleAi}
              title="显示 AI 助手"
              type="button"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path d="m14 7-5 5 5 5" />
              </svg>
            </button>
          ) : null}
        </section>
        {/* Runtime 常驻以承接后台输出和审批；visible 只控制工作台是否渲染。 */}
        <AiPanel visible={connectionRoute && aiOpen} />
      </div>
    </div>
  );
}
