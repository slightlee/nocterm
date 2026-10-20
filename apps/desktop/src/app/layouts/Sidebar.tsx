import type { ReactNode } from 'react';

import styles from './Sidebar.module.css';

/** 二级侧栏只负责布局，连接与 SFTP 内容由各功能模块提供。 */
export function Sidebar({
  children,
  navigationInset = false,
  compactTop = false,
}: {
  children: ReactNode;
  navigationInset?: boolean;
  /** Windows 标题栏独立存在且无顶部按钮，内嵌导航不再需要预留首行。 */
  compactTop?: boolean;
}) {
  return (
    <aside
      className={`${styles.sidebar} ${navigationInset ? styles.navigationInset : ''} ${
        navigationInset && compactTop ? styles.navigationInsetCompact : ''
      }`}
    >
      <div className={styles.content}>{children}</div>
    </aside>
  );
}
