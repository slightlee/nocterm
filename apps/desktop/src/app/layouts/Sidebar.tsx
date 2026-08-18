import type { ReactNode } from 'react';

import styles from './Sidebar.module.css';

/** 二级侧栏只负责布局，连接与 SFTP 内容由各功能模块提供。 */
export function Sidebar({ children }: { children: ReactNode }) {
  return (
    <aside className={styles.sidebar}>
      <div className={styles.content}>{children}</div>
    </aside>
  );
}
