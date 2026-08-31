import { NavLink, Outlet } from 'react-router-dom';

import styles from './SettingsPage.module.css';

/** 设置框架只呈现已经交付的子菜单，禁止用空入口暗示尚未实现的功能。 */
export function SettingsPage() {
  return (
    <main className={styles.page}>
      <aside className={styles.navigation}>
        <header className={styles.navigationHeader}>
          <p>NOCTERM</p>
          <h1>设置</h1>
        </header>
        <nav aria-label="设置分类">
          <NavLink
            className={({ isActive }) =>
              `${styles.navigationItem} ${isActive ? styles.active : ''}`
            }
            to="/settings/appearance"
          >
            <span>外观</span>
          </NavLink>
        </nav>
      </aside>
      <div className={styles.content}>
        <Outlet />
      </div>
    </main>
  );
}
