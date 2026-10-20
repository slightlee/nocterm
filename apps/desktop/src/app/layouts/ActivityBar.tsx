import { useLocation, useNavigate } from 'react-router-dom';

import { useWindowDrag } from '../../shared/hooks/useWindowDrag';
import styles from './ActivityBar.module.css';

const navigation = [
  {
    label: '连接',
    path: '/connections',
    icon: (
      <svg
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.9"
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        <path d="M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.7 1.7" />
        <path d="M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.7-1.7" />
      </svg>
    ),
  },
  {
    label: '文件',
    path: '/sftp',
    icon: (
      <svg
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.9"
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        <path d="M3 6a2 2 0 0 1 2-2h5l2 2h7a2 2 0 0 1 2 2v9a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3z" />
      </svg>
    ),
  },
];

interface ActivityBarProps {
  compactTop?: boolean;
  embedded?: boolean;
  onToggleSidebar?: () => void;
}

/** 设置页保留一级导航；工作区的终端与文件入口由各服务器行直接提供。 */
export function ActivityBar({
  compactTop = false,
  embedded = false,
  onToggleSidebar,
}: ActivityBarProps) {
  const location = useLocation();
  const navigate = useNavigate();
  const handleWindowDrag = useWindowDrag();

  return (
    <nav
      className={`${styles.activityBar} ${compactTop ? styles.compactTop : ''} ${embedded ? styles.embedded : ''}`}
      aria-label="主导航"
      onMouseDown={handleWindowDrag}
    >
      {/* 内嵌形态下 navItems 只承载侧栏折叠按钮；未提供回调（Windows 已把
              入口上移标题栏）时整行不渲染，避免透明层拦截下方搜索框点击。 */}
      {!embedded || onToggleSidebar ? (
        <div className={styles.navItems}>
          {embedded ? (
            <button
              aria-label="隐藏连接面板"
              className={`${styles.navItem} ${styles.sidebarToggle}`}
              onClick={onToggleSidebar}
              title="隐藏连接面板"
              type="button"
            >
              <span
                aria-hidden="true"
                className={`${styles.toggleIcon} ${styles.toggleIconRight}`}
              />
            </button>
          ) : (
            navigation.map((item) => (
              <button
                className={`${styles.navItem} ${location.pathname.startsWith(item.path) ? styles.active : ''}`}
                key={item.path}
                onClick={() => navigate(item.path)}
                title={item.label === '文件' ? 'SFTP 文件管理' : item.label}
                type="button"
              >
                {item.icon}
                <span className={styles.navLabel}>{item.label}</span>
              </button>
            ))
          )}
        </div>
      ) : null}
      <div className={styles.spacer} />
      <div className={styles.bottomSection}>
        <button
          className={`${styles.navItem} ${location.pathname.startsWith('/settings') ? styles.active : ''}`}
          onClick={() => navigate('/settings')}
          title="设置"
          type="button"
        >
          <svg
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.9"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <circle cx="12" cy="12" r="3" />
            <path d="M19.4 15a1.7 1.7 0 0 0 .34 2l.06.06a2.1 2.1 0 1 1-3 3l-.06-.06a1.7 1.7 0 0 0-2-.36 1.7 1.7 0 0 0-1 1.64V21a2.1 2.1 0 1 1-4.2 0v-.1a1.7 1.7 0 0 0-1-1.64 1.7 1.7 0 0 0-2 .36l-.06.06a2.1 2.1 0 1 1-3-3l.06-.06a1.7 1.7 0 0 0 .36-2 1.7 1.7 0 0 0-1.64-1H3a2.1 2.1 0 1 1 0-4.2h.1a1.7 1.7 0 0 0 1.64-1 1.7 1.7 0 0 0-.36-2l-.06-.06a2.1 2.1 0 1 1 3-3l.06.06a1.7 1.7 0 0 0 2 .36 1.7 1.7 0 0 0 1-1.64V3a2.1 2.1 0 1 1 4.2 0v.1a1.7 1.7 0 0 0 1 1.64 1.7 1.7 0 0 0 2-.36l.06-.06a2.1 2.1 0 1 1 3 3l-.06.06a1.7 1.7 0 0 0-.36 2 1.7 1.7 0 0 0 1.64 1h.1a2.1 2.1 0 1 1 0 4.2h-.1a1.7 1.7 0 0 0-1.6.3Z" />
          </svg>
          <span className={styles.navLabel}>设置</span>
        </button>
      </div>
    </nav>
  );
}
