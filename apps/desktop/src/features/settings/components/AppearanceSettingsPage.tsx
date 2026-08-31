import { useState, type CSSProperties } from 'react';

import type { AppTheme } from '../types/settings-types';
import { useSettings } from '../model/use-settings';
import {
  TERMINAL_FONT_SIZE_MAX,
  TERMINAL_FONT_SIZE_MIN,
  resolveTerminalTheme,
  terminalColorSchemes,
} from '../model/terminal-appearance';
import styles from './AppearanceSettingsPage.module.css';

const themeOptions: { value: AppTheme; label: string; description: string }[] = [
  { value: 'system', label: '跟随系统', description: '自动匹配系统外观' },
  { value: 'light', label: '浅色', description: '明亮、清晰的工作界面' },
  { value: 'dark', label: '深色', description: '适合暗光环境' },
];

/** 外观页使用 ANSI 输出示例呈现色板，选择结果会同步到已打开的本地与 SSH 会话。 */
export function AppearanceSettingsPage() {
  const {
    appTheme,
    terminalAppearance,
    loading,
    saving,
    persistenceAvailable,
    error,
    updateAppTheme,
    updateTerminalAppearance,
  } = useSettings();
  const [fontSizeDraft, setFontSizeDraft] = useState<number | null>(null);
  const displayedFontSize = fontSizeDraft ?? terminalAppearance.fontSize;
  const resolvedAppTheme =
    appTheme === 'system'
      ? window.matchMedia('(prefers-color-scheme: dark)').matches
        ? 'dark'
        : 'light'
      : appTheme;
  const followingApp = terminalAppearance.colorScheme === 'follow_app';
  const resolvedTerminalTheme = resolveTerminalTheme(
    terminalAppearance.colorScheme,
    resolvedAppTheme
  );

  const previewFontSize = (fontSize: number) => {
    setFontSizeDraft(fontSize);
    // 拖动期间只热更新运行界面，松手后再写 SQLite，避免连续 IPC 阻塞滑块。
    document.documentElement.dataset.terminalFontSize = String(fontSize);
  };

  const persistFontSize = () => {
    if (fontSizeDraft === null || fontSizeDraft === terminalAppearance.fontSize) return;
    const fontSize = fontSizeDraft;
    setFontSizeDraft(null);
    void updateTerminalAppearance({ ...terminalAppearance, fontSize });
  };

  const selectedScheme =
    terminalColorSchemes.find((scheme) => scheme.id === resolvedTerminalTheme) ??
    terminalColorSchemes[0];
  const rangeProgress =
    ((displayedFontSize - TERMINAL_FONT_SIZE_MIN) /
      (TERMINAL_FONT_SIZE_MAX - TERMINAL_FONT_SIZE_MIN)) *
    100;

  return (
    <div className={styles.page}>
      <section className={styles.section} aria-labelledby="application-theme-heading">
        <div className={styles.sectionHeading}>
          <h1 id="application-theme-heading">应用主题</h1>
          <span>界面</span>
        </div>
        <div
          className={styles.themeGrid}
          role="group"
          aria-busy={loading || saving}
          aria-label="应用主题"
        >
          {themeOptions.map((option) => {
            const selected = appTheme === option.value;
            return (
              <button
                aria-pressed={selected}
                className={`${styles.themeOption} ${selected ? styles.selected : ''}`}
                disabled={loading || saving}
                key={option.value}
                onClick={() => void updateAppTheme(option.value)}
                type="button"
              >
                <span
                  className={`${styles.themePreview} ${styles[option.value]}`}
                  aria-hidden="true"
                >
                  <span className={styles.previewRail} />
                  <span className={styles.previewBody}>
                    <span />
                    <span />
                  </span>
                </span>
                <span className={styles.optionCopy}>
                  <strong>{option.label}</strong>
                  <small>{option.description}</small>
                </span>
                <span className={styles.radio} aria-hidden="true" />
              </button>
            );
          })}
        </div>
      </section>

      <section className={styles.section} aria-labelledby="terminal-appearance-heading">
        <div className={styles.sectionHeading}>
          <h2 id="terminal-appearance-heading">终端外观</h2>
          <span>本地与 SSH</span>
        </div>

        <div className={styles.terminalStudio}>
          <div className={styles.terminalControlBar}>
            <div className={styles.fontControl}>
              <div className={styles.controlLabel}>
                <label htmlFor="terminal-font-size">字体大小</label>
                <output htmlFor="terminal-font-size">{displayedFontSize} px</output>
              </div>
              <input
                disabled={loading}
                id="terminal-font-size"
                max={TERMINAL_FONT_SIZE_MAX}
                min={TERMINAL_FONT_SIZE_MIN}
                onBlur={persistFontSize}
                onChange={(event) => previewFontSize(Number(event.target.value))}
                onKeyUp={persistFontSize}
                onPointerUp={persistFontSize}
                step="1"
                style={{ '--range-progress': `${rangeProgress}%` } as CSSProperties}
                type="range"
                value={displayedFontSize}
              />
              <div className={styles.rangeLabels} aria-hidden="true">
                <span>{TERMINAL_FONT_SIZE_MIN}</span>
                <span>{TERMINAL_FONT_SIZE_MAX}</span>
              </div>
            </div>

            <div className={styles.followControl}>
              <span className={styles.followCopy}>
                <strong>跟随应用</strong>
                <small>随应用主题自动切换明暗配色</small>
              </span>
              <button
                aria-checked={followingApp}
                aria-label="跟随应用主题"
                className={`${styles.switch} ${followingApp ? styles.switchOn : ''}`}
                disabled={loading || saving}
                onClick={() =>
                  void updateTerminalAppearance({
                    ...terminalAppearance,
                    colorScheme: followingApp
                      ? resolvedAppTheme === 'light'
                        ? 'nocterm_light'
                        : 'nocterm_dark'
                      : 'follow_app',
                  })
                }
                role="switch"
                type="button"
              />
            </div>
          </div>

          <div className={styles.terminalWorkbench}>
            <div
              className={`${styles.terminalPreview} ${styles[selectedScheme.previewClass]}`}
              style={{ fontSize: `${displayedFontSize}px` }}
              aria-label="ANSI 终端配色预览"
            >
              <div className={styles.previewToolbar} aria-hidden="true">
                <span className={styles.toolbarDot} />
                <span className={styles.toolbarDot} />
                <span className={styles.toolbarDot} />
              </div>
              <div className={styles.terminalScreen}>
                <p>root@server:/# ls -la</p>
                <p className={styles.muted}>total 24</p>
                <p>
                  drwxr-xr-x 4 root root 4096 <span className={styles.directory}>src</span>
                </p>
                <p>
                  lrwxrwxrwx 1 root root 7 <span className={styles.symlink}>lib</span> →{' '}
                  <span className={styles.directory}>usr/lib</span>
                </p>
                <p>
                  -rwxr-xr-x 1 root root 842 <span className={styles.executable}>deploy.sh</span>
                </p>
                <p>-rw-r--r-- 1 root root 321 README.md</p>
                <p>
                  <span className={styles.warning}>WARN</span> configuration changed
                </p>
                <p>
                  <span className={styles.terminalError}>ERROR</span> connection refused{' '}
                  <span className={styles.cursor} />
                </p>
              </div>
            </div>

            <div className={styles.paletteControl}>
              <div className={styles.paletteHeading}>
                <span className={styles.paletteTitle}>配色方案</span>
                <small>
                  {followingApp
                    ? `ANSI 配色 · 当前为 ${selectedScheme.label}`
                    : 'ANSI 配色 · 应用到本地与 SSH'}
                </small>
              </div>
              <div className={styles.paletteViewport} aria-label="终端配色方案列表" role="region">
                <div className={styles.paletteGrid} role="group" aria-label="终端配色">
                  {terminalColorSchemes.map((scheme) => {
                    const selected = !followingApp && terminalAppearance.colorScheme === scheme.id;
                    return (
                      <button
                        aria-pressed={selected}
                        className={`${styles.paletteOption} ${styles[scheme.previewClass]} ${selected ? styles.selectedPalette : ''}`}
                        disabled={loading || saving}
                        key={scheme.id}
                        onClick={() =>
                          void updateTerminalAppearance({
                            ...terminalAppearance,
                            colorScheme: scheme.id,
                          })
                        }
                        title={scheme.description}
                        type="button"
                      >
                        <span className={styles.paletteSwatch} aria-hidden="true">
                          <i />
                          <i />
                          <i />
                          <i />
                          <i />
                        </span>
                        <span>{scheme.label}</span>
                      </button>
                    );
                  })}
                </div>
              </div>
            </div>
          </div>
        </div>
      </section>

      <div className={styles.feedback}>
        {!persistenceAvailable ? (
          <p className={styles.notice}>浏览器仅预览效果，桌面客户端会自动保存设置。</p>
        ) : null}
        {error ? <p className={styles.error}>{error}</p> : null}
      </div>
    </div>
  );
}
