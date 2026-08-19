import { useEffect, useState } from 'react';

import {
  closeDesktopWindow,
  minimizeDesktopWindow,
  toggleDesktopWindowMaximize,
  watchDesktopWindowMaximized,
} from '../../shared/lib/window-client';
import styles from './WindowsWindowControls.module.css';

function runWindowAction(action: () => Promise<void>) {
  void action().catch(() => {
    console.error('Window action failed');
  });
}

/** Windows 无边框窗口使用原生位置与命中尺寸，视觉保持与应用顶部工具栏一致。 */
export function WindowsWindowControls() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    void watchDesktopWindowMaximized((value) => {
      if (!disposed) setMaximized(value);
    })
      .then((dispose) => {
        if (disposed) {
          dispose();
          return;
        }
        unlisten = dispose;
      })
      .catch(() => {
        console.error('Window state subscription failed');
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  return (
    <div className={styles.controls} data-no-window-drag="true">
      <button
        aria-label="最小化窗口"
        className={styles.controlButton}
        onClick={() => runWindowAction(minimizeDesktopWindow)}
        title="最小化"
        type="button"
      >
        <span aria-hidden="true" className={styles.minimizeIcon} />
      </button>
      <button
        aria-label={maximized ? '还原窗口' : '最大化窗口'}
        className={styles.controlButton}
        onClick={() => runWindowAction(toggleDesktopWindowMaximize)}
        title={maximized ? '还原' : '最大化'}
        type="button"
      >
        <span aria-hidden="true" className={maximized ? styles.restoreIcon : styles.maximizeIcon} />
      </button>
      <button
        aria-label="关闭窗口"
        className={`${styles.controlButton} ${styles.closeButton}`}
        onClick={() => runWindowAction(closeDesktopWindow)}
        title="关闭"
        type="button"
      >
        <span aria-hidden="true" className={styles.closeIcon} />
      </button>
    </div>
  );
}
