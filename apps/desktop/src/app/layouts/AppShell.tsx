import { useEffect, useState, type MouseEvent } from 'react';
import { Outlet, useLocation } from 'react-router-dom';

import { ConnectionList } from '../../features/connections';
import { useTopWindowDrag } from '../../shared/hooks/useWindowDrag';
import { isWindowsDesktopRuntime } from '../../shared/lib/desktop-platform';
import { ActivityBar } from './ActivityBar';
import { Sidebar } from './Sidebar';
import { StatusBar } from './StatusBar';
import { TerminalWorkspace } from './TerminalWorkspace';
import styles from './AppShell.module.css';

/** 旧客户端主布局：一级导航、连接侧栏、终端工作区和状态栏。 */
export function AppShell() {
  const location = useLocation();
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const connectionRoute = location.pathname.startsWith('/connections');
  const sidebarRoute = connectionRoute || location.pathname.startsWith('/sftp');
  const settingsRoute = location.pathname.startsWith('/settings');
  const handleTopWindowDrag = useTopWindowDrag();
  const windowsDesktop = isWindowsDesktopRuntime();

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

  useEffect(() => {
    // 设置页尚未迁移时仍跟随系统主题，保持旧版默认的 system 语义。
    const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)');
    const syncTheme = () => {
      document.documentElement.dataset.theme = mediaQuery.matches ? 'dark' : 'light';
    };
    syncTheme();
    mediaQuery.addEventListener('change', syncTheme);
    return () => mediaQuery.removeEventListener('change', syncTheme);
  }, []);

  return (
    <div
      className={styles.app}
      onContextMenuCapture={suppressBrowserContextMenu}
      onMouseDownCapture={handleTopWindowDrag}
    >
      <ActivityBar compactTop={windowsDesktop} />
      {sidebarRoute && !sidebarCollapsed ? (
        <Sidebar>
          <ConnectionList />
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
        {!connectionRoute ? <Outlet /> : null}
        {!settingsRoute ? <StatusBar /> : null}
      </section>
    </div>
  );
}
