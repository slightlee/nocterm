import { useCallback, useEffect, useState, type MouseEvent } from 'react';
import { Outlet, useLocation, useNavigate } from 'react-router-dom';

import { ConnectionList } from '../../features/connections';
import { AiPanel, useAiStore, type AiTerminalSelectionRequest } from '../../features/ai';
import { useTerminalStore } from '../../features/terminal';
import {
  FileEditorCloseGuard,
  SftpView,
  useFileEditorStore,
  useSftpStore,
} from '../../features/sftp';
import { PanelSideIcon } from '../../shared/components/PanelSideIcon';
import { useTopWindowDoubleClick, useTopWindowDrag } from '../../shared/hooks/useWindowDrag';
import { isWindowsDesktopRuntime } from '../../shared/lib/desktop-platform';
import { onAppMenuAction } from '../platform/app-menu-client';
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
  const [terminalSelectionRequest, setTerminalSelectionRequest] =
    useState<AiTerminalSelectionRequest | null>(null);
  const connectionRoute = location.pathname.startsWith('/connections');
  const sftpRoute = location.pathname.startsWith('/sftp');
  const fileViewVisited = useSftpStore((state) => state.fileViewVisited);
  const hasFileEditors = useFileEditorStore((state) => state.editors.length > 0);
  // 只在真正进入过文件页后常驻；创建 SFTP 会话本身不会提前读本地主目录。
  const sftpMounted = sftpRoute || fileViewVisited || hasFileEditors;
  const sidebarRoute = connectionRoute || sftpRoute;
  const settingsRoute = location.pathname.startsWith('/settings');
  const handleTopWindowDrag = useTopWindowDrag();
  const handleTopWindowDoubleClick = useTopWindowDoubleClick();
  const windowsDesktop = isWindowsDesktopRuntime();
  const aiOpen = useAiStore((state) => state.open);
  const setAiOpen = useAiStore((state) => state.setOpen);
  const toggleAi = useAiStore((state) => state.toggle);
  const activateTerminal = useTerminalStore((state) => state.activateConnection);

  const addTerminalSelectionToAi = useCallback(
    (terminalSessionId: string, selection: string) => {
      setTerminalSelectionRequest({
        id: crypto.randomUUID(),
        terminalSessionId,
        selection,
      });
      setAiOpen(true);
    },
    [setAiOpen]
  );

  const handleTerminalSelectionHandled = useCallback((requestId: string) => {
    setTerminalSelectionRequest((current) => (current?.id === requestId ? null : current));
  }, []);

  const createLocalTerminal = useCallback(() => {
    const { sessions, openLocalSession } = useTerminalStore.getState();
    const localCount = sessions.filter((session) => session.kind === 'local').length;
    const name = localCount === 0 ? '本地终端' : `本地终端 ${localCount + 1}`;
    useFileEditorStore.getState().activateEditor(null);
    openLocalSession(`local:extra:${crypto.randomUUID()}`, name);
    navigate('/connections');
  }, [navigate]);

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
    // 任何入口进入终端页都要退出编辑展示：编辑标签失去激活态，
    // 与「新建本地终端 / AI 面板切终端」的显式清理保持同一不变式。
    if (connectionRoute) useFileEditorStore.getState().activateEditor(null);
  }, [connectionRoute]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    void onAppMenuAction((action) => {
      switch (action) {
        case 'open-about':
          navigate('/settings/about');
          break;
        case 'open-settings':
          navigate('/settings');
          break;
        case 'new-local-terminal':
          createLocalTerminal();
          break;
        case 'toggle-sidebar':
          if (!sidebarRoute) {
            setSidebarCollapsed(false);
            navigate('/connections');
          } else {
            setSidebarCollapsed((collapsed) => !collapsed);
          }
          break;
        case 'toggle-ai':
          if (!sidebarRoute) {
            if (!aiOpen) toggleAi();
            navigate('/connections');
          } else {
            toggleAi();
          }
          break;
        case 'close-current-tab':
          // 关闭操作由 WorkspaceSessionTabs 处理，以复用 SFTP 传输确认和资源清理。
          break;
      }
    })
      .then((dispose) => {
        if (disposed) dispose();
        else unlisten = dispose;
      })
      .catch(() => {
        console.error('App menu subscription failed');
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [aiOpen, createLocalTerminal, navigate, sidebarRoute, toggleAi]);

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
                className={styles.titleBarToggle}
                onClick={() => setSidebarCollapsed((collapsed) => !collapsed)}
                title={sidebarCollapsed ? '显示连接面板' : '隐藏连接面板'}
                type="button"
              >
                <PanelSideIcon className={styles.titleBarToggleIcon} side="left" />
              </button>
            ) : null}
          </div>
          <WindowsWindowControls />
        </header>
      ) : null}
      <div className={styles.app} onContextMenuCapture={suppressBrowserContextMenu}>
        <FileEditorCloseGuard />
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
              onCreateLocalTerminal={createLocalTerminal}
              sidebarCollapsed={sidebarCollapsed}
              onToggleSidebar={() => setSidebarCollapsed((collapsed) => !collapsed)}
            />
          ) : null}
          <div
            className={`${styles.terminalWorkspace} ${connectionRoute ? '' : styles.hiddenWorkspace}`}
          >
            <TerminalWorkspace onAddSelectionToAi={addTerminalSelectionToAi} />
          </div>
          {/* 首次进入文件页后常驻；切换路由只隐藏，不销毁文件浏览和 CodeMirror 状态。 */}
          {sftpMounted ? (
            <div
              className={`${styles.terminalWorkspace} ${sftpRoute ? '' : styles.hiddenWorkspace}`}
            >
              <SftpView visible={sftpRoute} />
            </div>
          ) : null}
          {!sidebarRoute ? (
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
            useFileEditorStore.getState().activateEditor(null);
            activateTerminal(sessionId);
            navigate('/connections');
          }}
          onTerminalSelectionHandled={handleTerminalSelectionHandled}
          terminalSelectionRequest={terminalSelectionRequest}
          visible={sidebarRoute && aiOpen}
        />
      </div>
    </div>
  );
}
