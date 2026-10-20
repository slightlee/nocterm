import styles from './ConnectionBackupSettingsPage.module.css';

interface ConnectionBackupMenuProps {
  busy: boolean;
  onImportConnections: () => void;
  onExportConnections: () => void;
  onImportSshConfig: () => void;
}

/** 设置页只编排操作入口；文件选择、解析和数据库写入仍由页面与后端负责。 */
export function ConnectionBackupMenu({
  busy,
  onImportConnections,
  onExportConnections,
  onImportSshConfig,
}: ConnectionBackupMenuProps) {
  return (
    <div aria-busy={busy} className={styles.actionList}>
      <div className={styles.actionRow}>
        <span aria-hidden="true" className={styles.actionIcon}>
          <svg viewBox="0 0 24 24">
            <path d="M20 15v4H4v-4" />
            <path d="m8 8 4-4 4 4" />
            <path d="M12 4v10" />
          </svg>
        </span>
        <div className={styles.actionCopy}>
          <h2>导出连接</h2>
          <p>将当前连接与分组保存为本地备份</p>
        </div>
        <button
          className={styles.actionButton}
          disabled={busy}
          onClick={onExportConnections}
          type="button"
        >
          导出…
        </button>
      </div>

      <div className={styles.actionRow}>
        <span aria-hidden="true" className={styles.actionIcon}>
          <svg viewBox="0 0 24 24">
            <path d="M20 15v4H4v-4" />
            <path d="m8 10 4 4 4-4" />
            <path d="M12 14V4" />
          </svg>
        </span>
        <div className={styles.actionCopy}>
          <h2>导入连接</h2>
          <p>从 Nocterm JSON 备份恢复连接与分组</p>
        </div>
        <button
          className={styles.actionButton}
          disabled={busy}
          onClick={onImportConnections}
          type="button"
        >
          导入…
        </button>
      </div>

      <div className={styles.actionRow}>
        <span aria-hidden="true" className={styles.actionIcon}>
          <svg viewBox="0 0 24 24">
            <path d="m5 7 4 4-4 4" />
            <path d="M11 15h8" />
          </svg>
        </span>
        <div className={styles.actionCopy}>
          <h2>从 SSH Config 导入</h2>
          <p>读取本机 OpenSSH 主机配置并创建连接</p>
        </div>
        <button
          className={styles.actionButton}
          disabled={busy}
          onClick={onImportSshConfig}
          type="button"
        >
          选择文件…
        </button>
      </div>

      <div className={styles.securityNote}>
        <svg aria-hidden="true" viewBox="0 0 20 20">
          <path d="M10 2.8 16 5v4.6c0 3.7-2.4 6.2-6 7.6-3.6-1.4-6-3.9-6-7.6V5z" />
          <path d="m7.4 10 1.7 1.7 3.7-4" />
        </svg>
        <span>只备份连接与分组，不包含密码和私钥。</span>
      </div>
    </div>
  );
}
