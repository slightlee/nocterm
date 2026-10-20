import { Link, NavLink, Outlet } from 'react-router-dom';

import { isWindowsDesktopRuntime } from '../../../shared/lib/desktop-platform';

import styles from './SettingsPage.module.css';

/** 设置框架只呈现已经交付的子菜单，禁止用空入口暗示尚未实现的功能。 */
export function SettingsPage() {
  const compactTop = isWindowsDesktopRuntime();

  return (
    <main className={styles.page}>
      <aside
        className={`${styles.navigation} ${compactTop ? styles.compactTop : ''}`}
        aria-label="设置导航"
      >
        {/* macOS 的系统红绿灯覆盖在 WebView 顶部；Windows 已有独立标题栏。 */}
        <div aria-hidden="true" className={styles.windowInset} />
        <header className={styles.navigationHeader}>
          <h1>设置</h1>
        </header>
        <nav className={styles.navigationList} aria-label="设置分类">
          <NavLink
            className={({ isActive }) =>
              `${styles.navigationItem} ${isActive ? styles.active : ''}`
            }
            to="/settings/appearance"
          >
            <svg aria-hidden="true" viewBox="0 0 20 20">
              <circle cx="10" cy="10" r="3.2" />
              <path d="M10 2.8v1.4M10 15.8v1.4M2.8 10h1.4M15.8 10h1.4" />
              <path d="m4.9 4.9 1 1M14.1 14.1l1 1M15.1 4.9l-1 1M5.9 14.1l-1 1" />
            </svg>
            <span>外观</span>
          </NavLink>
          <NavLink
            className={({ isActive }) =>
              `${styles.navigationItem} ${isActive ? styles.active : ''}`
            }
            to="/settings/backup"
          >
            <svg aria-hidden="true" viewBox="0 0 20 20">
              <path d="M4 6.5h12v9H4z" />
              <path d="M6 3.5h8l1.5 3h-11z" />
              <path d="M7.5 10h5" />
            </svg>
            <span>备份与恢复</span>
          </NavLink>
          <NavLink
            className={({ isActive }) =>
              `${styles.navigationItem} ${isActive ? styles.active : ''}`
            }
            to="/settings/about"
          >
            <svg aria-hidden="true" viewBox="0 0 20 20">
              <circle cx="10" cy="10" r="6.5" />
              <path d="M10 8.5v4.3" />
              <path d="M10 6.2h.01" />
            </svg>
            <span>关于</span>
          </NavLink>
        </nav>
        <div className={styles.navigationFooter}>
          <Link className={styles.workspaceLink} to="/connections">
            <svg aria-hidden="true" viewBox="0 0 20 20">
              <path d="m9 5-5 5 5 5" />
              <path d="M4.5 10H16" />
            </svg>
            <span>返回工作区</span>
          </Link>
        </div>
      </aside>
      <div className={styles.content}>
        <Outlet />
      </div>
    </main>
  );
}
