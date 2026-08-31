import { useState, type MouseEvent } from 'react';

import appIcon from '../../../../src-tauri/icons/icon.png';
import { isDesktopRuntime } from '../../../shared/lib/tauri-runtime';
import { useRuntimeHealth } from '../../runtime';
import { openExternalUrl } from '../api/settings-client';
import { formatArchitectureLabel, formatPlatformLabel } from '../model/about-info';
import styles from './AboutSettingsPage.module.css';

const PROJECT_URL = 'https://github.com/slightlee/nocterm';
const LICENSE_URL = `${PROJECT_URL}/blob/main/LICENSE`;

/** 关于页只呈现用户需要确认的版本、平台和项目归属，不展示内部实现细节。 */
export function AboutSettingsPage() {
  const runtime = useRuntimeHealth();
  const health = runtime.health;
  const [externalOpenError, setExternalOpenError] = useState(false);

  const openExternalLink = (url: string, event: MouseEvent<HTMLAnchorElement>) => {
    if (!isDesktopRuntime()) return;
    event.preventDefault();
    setExternalOpenError(false);
    void openExternalUrl(url).catch(() => setExternalOpenError(true));
  };

  return (
    <div className={styles.page}>
      <header className={styles.hero}>
        <img className={styles.appIcon} src={appIcon} alt="Nocterm 应用图标" />
        <div className={styles.heroCopy}>
          <h1>Nocterm</h1>
          <p>本地终端、SSH 连接与远程文件管理</p>
          <div className={styles.heroMeta}>
            <span>{health ? `v${health.version}` : '桌面客户端'}</span>
          </div>
        </div>
      </header>

      <div className={styles.sectionGrid}>
        <section className={styles.card} aria-labelledby="product-info-heading">
          <div className={styles.cardHeading}>
            <span className={styles.sectionIndex}>01</span>
            <h2 id="product-info-heading">产品信息</h2>
          </div>
          <dl className={styles.detailList}>
            <div>
              <dt>开源许可</dt>
              <dd>
                <a
                  className={styles.sourceLink}
                  href={LICENSE_URL}
                  onClick={(event) => openExternalLink(LICENSE_URL, event)}
                  rel="noreferrer"
                  target="_blank"
                >
                  MIT
                </a>
              </dd>
            </div>
            <div>
              <dt>项目源码</dt>
              <dd>
                <a
                  className={styles.sourceLink}
                  href={PROJECT_URL}
                  onClick={(event) => openExternalLink(PROJECT_URL, event)}
                  rel="noreferrer"
                  target="_blank"
                >
                  查看 GitHub 源码
                </a>
              </dd>
            </div>
          </dl>
          {externalOpenError ? (
            <p className={styles.sourceError}>无法打开浏览器，请稍后重试。</p>
          ) : null}
        </section>

        <section className={styles.card} aria-labelledby="runtime-info-heading">
          <div className={styles.cardHeading}>
            <span className={styles.sectionIndex}>02</span>
            <h2 id="runtime-info-heading">运行环境</h2>
          </div>
          <dl className={styles.detailList}>
            <div>
              <dt>操作系统</dt>
              <dd>{health ? formatPlatformLabel(health.platform) : '桌面运行时中显示'}</dd>
            </div>
            <div>
              <dt>处理器架构</dt>
              <dd>{health ? formatArchitectureLabel(health.architecture) : '桌面运行时中显示'}</dd>
            </div>
          </dl>
        </section>
      </div>
    </div>
  );
}
