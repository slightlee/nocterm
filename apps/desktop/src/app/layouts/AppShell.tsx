import { useEffect, useState, type MouseEvent } from 'react';
import { Outlet, useLocation, useNavigate } from 'react-router-dom';

import { ConnectionList } from '../../features/connections';
import { AiPanel, useAiStore } from '../../features/ai';
import { useTerminalStore } from '../../features/terminal';
import { useTopWindowDoubleClick, useTopWindowDrag } from '../../shared/hooks/useWindowDrag';
import { isWindowsDesktopRuntime } from '../../shared/lib/desktop-platform';
import { ActivityBar } from './ActivityBar';
import { Sidebar } from './Sidebar';
import { StatusBar } from './StatusBar';
import { TerminalWorkspace } from './TerminalWorkspace';
import { WorkspaceSessionTabs } from './WorkspaceSessionTabs';
import { WindowsWindowControls } from './WindowsWindowControls';
import styles from './AppShell.module.css';

/** 旧客户端主布局：一级导航、连接侧栏、终端工作区和状态栏。 */
export function AppShell() {
  const location = useLocation();
  const navigate = useNavigate();
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const connectionRoute = location.pathname.startsWith('/connections');
  const sidebarRoute = connectionRoute || location.pathname.startsWith('/sftp');
  const settingsRoute = location.pathname.startsWith('/settings');
  const handleTopWindowDrag = useTopWindowDrag();
  const handleTopWindowDoubleClick = useTopWindowDoubleClick();
  const windowsDesktop = isWindowsDesktopRuntime();
  const aiOpen = useAiStore((state) => state.open);
  const toggleAi = useAiStore((state) => state.toggle);
  const activateTerminal = useTerminalStore((state) => state.activateConnection);

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
          捕获层，两种窗口形态共享同一套顶部交互。侧栏折叠按钮固定在品牌名
          右侧：标题栏不随侧栏折叠消失，展开/折叠两态按钮位置恒定。 */}
      {windowsDesktop ? (
        <header className={styles.titleBar}>
          <div className={styles.titleBarBrand}>
            <img alt="" className={styles.titleBarLogo} src="/favicon.png" />
            <span className={styles.titleBarName}>Nocterm</span>
            {sidebarRoute ? (
              <button
                aria-expanded={!sidebarCollapsed}
                aria-label={sidebarCollapsed ? '显示连接面板' : '隐藏连接面板'}
                className={`${styles.titleBarToggle} ${
                  sidebarCollapsed ? styles.titleBarToggleCollapsed : styles.titleBarToggleExpanded
                }`}
                onClick={() => setSidebarCollapsed((collapsed) => !collapsed)}
                title={sidebarCollapsed ? '显示连接面板' : '隐藏连接面板'}
                type="button"
              >
                <span aria-hidden="true" className={styles.titleBarToggleIcon} />
              </button>
            ) : null}
          </div>
          <WindowsWindowControls />
        </header>
      ) : null}
      <div className={styles.app} onContextMenuCapture={suppressBrowserContextMenu}>
        {sidebarRoute ? (
          <div
            aria-hidden={sidebarCollapsed}
            className={`${styles.navigationPane} ${
              sidebarCollapsed ? styles.navigationPaneCollapsed : ''
            }`}
            inert={sidebarCollapsed}
          >
            <ActivityBar
              compactTop={windowsDesktop}
              embedded
              /* Windows 的折叠入口已上移到标题栏；macOS 保留侧栏顶部按钮。 */
              onToggleSidebar={windowsDesktop ? undefined : () => setSidebarCollapsed(true)}
            />
            <Sidebar compactTop={windowsDesktop} navigationInset>
              <ConnectionList mode={location.pathname.startsWith('/sftp') ? 'sftp' : 'terminal'} />
            </Sidebar>
          </div>
        ) : null}
        <section className={styles.workspace}>
          {sidebarRoute ? (
            <WorkspaceSessionTabs
              activeView={connectionRoute ? 'terminal' : 'sftp'}
              aiOpen={aiOpen}
              compactTop={windowsDesktop}
              onToggleAi={toggleAi}
              sidebarCollapsed={sidebarCollapsed}
              onToggleSidebar={() => setSidebarCollapsed((collapsed) => !collapsed)}
            />
          ) : null}
          <div
            className={`${styles.terminalWorkspace} ${connectionRoute ? '' : styles.hiddenWorkspace}`}
          >
            <TerminalWorkspace />
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
        </section>
        {/* Runtime 常驻以承接后台输出和审批；visible 只控制工作台是否渲染。 */}
        <AiPanel
          activeView={connectionRoute ? 'terminal' : 'sftp'}
          onActivateTerminal={(sessionId) => {
            activateTerminal(sessionId);
            navigate('/connections');
          }}
          visible={sidebarRoute && aiOpen}
        />
      </div>
    </div>
  );
}
