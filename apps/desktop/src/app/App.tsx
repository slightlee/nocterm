import { useRuntimeHealth } from '../features/runtime';
import styles from './App.module.css';

const modules = [
  { name: 'Connections', code: '01', state: 'planned' },
  { name: 'Terminal', code: '02', state: 'planned' },
  { name: 'SFTP', code: '03', state: 'planned' },
  { name: 'Monitor', code: '04', state: 'planned' },
];

export function App() {
  const runtime = useRuntimeHealth();

  return (
    <main className={styles.shell}>
      <header className={styles.topbar}>
        <div className={styles.brand}>
          <span className={styles.brandMark} aria-hidden="true">
            N/
          </span>
          <span>Nocterm</span>
        </div>
        <div className={styles.runtimePill} data-state={runtime.status}>
          <span className={styles.statusDot} aria-hidden="true" />
          {runtime.label}
        </div>
      </header>

      <section className={styles.workspace}>
        <aside className={styles.rail} aria-label="基础模块">
          <span className={styles.railLabel}>SYSTEM MAP</span>
          <nav>
            {modules.map((module) => (
              <button className={styles.moduleButton} key={module.code} type="button" disabled>
                <span>{module.code}</span>
                {module.name}
              </button>
            ))}
          </nav>
          <span className={styles.buildTag}>FOUNDATION / 0.1</span>
        </aside>

        <article className={styles.stage}>
          <div className={styles.gridGlow} aria-hidden="true" />
          <div className={styles.eyebrow}>LOCAL-FIRST DESKTOP TERMINAL</div>
          <h1>
            架构基线
            <span>已经接通</span>
          </h1>
          <p className={styles.lede}>
            React 界面、Typed IPC、Application 用例与 Rust 平台探针已经形成第一条完整链路。
            核心业务将按连接、终端、SSH 与 SFTP 顺序纵向迁移。
          </p>

          <div className={styles.telemetry}>
            <section>
              <span className={styles.metricLabel}>TARGET</span>
              <strong>{runtime.health?.platform ?? 'browser-preview'}</strong>
              <small>{runtime.health?.architecture ?? 'web renderer'}</small>
            </section>
            <section>
              <span className={styles.metricLabel}>TERMINAL</span>
              <strong>{runtime.health?.terminalBackend ?? 'adapter pending'}</strong>
              <small>platform boundary</small>
            </section>
            <section>
              <span className={styles.metricLabel}>CREDENTIAL</span>
              <strong>{runtime.health?.credentialStore ?? 'adapter pending'}</strong>
              <small>secret isolation</small>
            </section>
          </div>

          {runtime.error ? <p className={styles.runtimeError}>{runtime.error}</p> : null}
        </article>
      </section>

      <footer className={styles.statusbar}>
        <span>NO BACKEND REQUIRED</span>
        <span>MACOS + WINDOWS CONTRACT</span>
        <span>{runtime.eventCount} RUNTIME EVENT</span>
      </footer>
    </main>
  );
}
