import { useEffect, useRef, useState } from 'react';

import styles from './ConnectionList.module.css';

interface ConnectionBackupMenuProps {
  busy: boolean;
  onImportConnections: () => void;
  onExportConnections: () => void;
  onImportSshConfig: () => void;
}

/** 备份菜单保持旧版入口和顺序，具体文件与数据操作由列表编排。 */
export function ConnectionBackupMenu({
  busy,
  onImportConnections,
  onExportConnections,
  onImportSshConfig,
}: ConnectionBackupMenuProps) {
  const [open, setOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const close = (event: MouseEvent) => {
      if (!menuRef.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', close);
    return () => document.removeEventListener('mousedown', close);
  }, [open]);

  const run = (action: () => void) => {
    setOpen(false);
    action();
  };

  return (
    <div className={styles.backupWrap} ref={menuRef}>
      <button
        aria-expanded={open}
        className={styles.backupBtn}
        disabled={busy}
        onClick={() => setOpen((current) => !current)}
        type="button"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
          <polyline points="7 10 12 15 17 10" />
          <line x1="12" y1="15" x2="12" y2="3" />
        </svg>
        {busy ? '处理中…' : '备份与恢复'}
      </button>
      {open ? (
        <div className={styles.backupMenu} role="menu">
          <div className={styles.backupMenuHeader}>备份与恢复</div>
          <button
            className={styles.backupMenuItem}
            onClick={() => run(onImportConnections)}
            role="menuitem"
            type="button"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
              <polyline points="7 10 12 15 17 10" />
              <line x1="12" y1="15" x2="12" y2="3" />
            </svg>
            导入连接…
          </button>
          <button
            className={styles.backupMenuItem}
            onClick={() => run(onExportConnections)}
            role="menuitem"
            type="button"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
              <polyline points="17 8 12 3 7 8" />
              <line x1="12" y1="3" x2="12" y2="15" />
            </svg>
            导出连接…
          </button>
          <div className={styles.menuSeparator} role="separator" />
          <button
            className={styles.backupMenuItem}
            onClick={() => run(onImportSshConfig)}
            role="menuitem"
            type="button"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <line x1="4" y1="6" x2="20" y2="6" />
              <line x1="4" y1="12" x2="20" y2="12" />
              <line x1="4" y1="18" x2="20" y2="18" />
            </svg>
            从本机 SSH 配置导入
          </button>
        </div>
      ) : null}
    </div>
  );
}
