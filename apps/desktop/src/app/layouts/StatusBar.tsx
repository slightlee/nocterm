import styles from './StatusBar.module.css';
import { useTerminalStore } from '../../features/terminal';

/** SSH 阶段状态栏先呈现会话状态，传输队列留给 SFTP 阶段接入。 */
export function StatusBar() {
  const { sessions, activeId, status } = useTerminalStore();
  const session = sessions.find((item) => item.id === activeId) ?? null;
  const statusColor =
    status === 'connected' ? styles.green : status === 'error' ? styles.red : styles.orange;
  const sessionPath = session?.kind === 'remote' ? session.remoteInitialPath || '~' : '~';

  return (
    <footer className={styles.statusbar}>
      {session ? (
        <div className={styles.statusLeft} title={`${session.name} ${sessionPath}`}>
          <span className={`${styles.statusDot} ${statusColor}`} />
          <span className={styles.sessionName}>{session.name}</span>
          <span className={styles.sessionPath}>{sessionPath}</span>
        </div>
      ) : null}
      <span className={styles.spacer} />
      <div className={styles.statusRight}>
        <span className={styles.statusItem}>UTF-8</span>
        <span className={styles.statusItem}>LF</span>
      </div>
    </footer>
  );
}
