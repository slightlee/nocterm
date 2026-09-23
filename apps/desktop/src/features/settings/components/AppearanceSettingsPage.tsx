import { Fragment, useEffect, useMemo, useRef, useState, type CSSProperties } from 'react';

import type { AppTheme, HighlightRoleId } from '../types/settings-types';
import { useSettings } from '../model/use-settings';
import { HighlightColorSettings } from './HighlightColorSettings';
import { useTerminalPalette } from '../model/use-terminal-palette';
import {
  HIGHLIGHT_PRESET_LABELS,
  encodeHighlightOverrides,
  parseHighlightOverrides,
  resolveHighlightRoles,
} from '../../terminal/model/highlight-roles';
import {
  TERMINAL_FONT_SIZE_MAX,
  TERMINAL_FONT_SIZE_MIN,
  resolveTerminalTheme,
  terminalColorSchemes,
  terminalSchemeGroups,
} from '../model/terminal-appearance';
import { evaluateSlotContrast } from '../model/highlight-guard';
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

  // 字体颜色预览：角色 → 当前主题调色板翻译出的 RGB，与终端渲染同源。
  // 加粗角色落在基色槽（0-7）时，真实终端经 drawBoldTextInBrightColors
  // 以亮色变体呈现，预览按同样规则映射，保证所见即所得。
  const palette = useTerminalPalette();
  const resolvedRoles = useMemo(
    () =>
      resolveHighlightRoles(
        terminalAppearance.highlightPreset,
        parseHighlightOverrides(terminalAppearance.highlightOverrides)
      ),
    [terminalAppearance.highlightPreset, terminalAppearance.highlightOverrides]
  );
  const roleColor = (id: HighlightRoleId) => {
    const role = resolvedRoles[id];
    const slot = role.bold && role.slot <= 7 ? role.slot + 8 : role.slot;
    return palette.slots[slot];
  };
  const roleStyle = (id: HighlightRoleId): CSSProperties => ({
    color: roleColor(id),
    fontWeight: resolvedRoles[id].bold ? 600 : undefined,
  });

  // 换主题自动兜底：对比度守卫只拦截"当下新选"的颜色，管不了历史选择——
  // 在浅色主题下选的黑，切到深色主题后依然保存着、但已看不清。主题切换时
  // 检查自定义覆盖项，把在新主题下不达对比度的角色恢复为预设默认
  // （预设按各主题调色板渲染，天然可读），并轻提示一次，用户无需手动处理。
  const slotEnabled = useMemo(() => evaluateSlotContrast(palette), [palette]);
  const [adjustHint, setAdjustHint] = useState('');
  const prevBackgroundRef = useRef(palette.background);
  useEffect(() => {
    if (prevBackgroundRef.current === palette.background) return;
    prevBackgroundRef.current = palette.background;
    const overrides = parseHighlightOverrides(terminalAppearance.highlightOverrides);
    const unreadable = Object.entries(overrides).filter(([, slot]) => !slotEnabled[slot]);
    if (unreadable.length === 0) return;
    const next = { ...overrides };
    for (const [role] of unreadable) delete next[role as HighlightRoleId];
    void updateTerminalAppearance({
      ...terminalAppearance,
      highlightOverrides: encodeHighlightOverrides(next),
    });
    const showTimer = window.setTimeout(
      () => setAdjustHint(`已自动恢复 ${unreadable.length} 处在新主题下看不清的字体颜色`),
      0
    );
    const hideTimer = window.setTimeout(() => setAdjustHint(''), 8000);
    return () => {
      window.clearTimeout(showTimer);
      window.clearTimeout(hideTimer);
    };
  }, [palette.background, slotEnabled, terminalAppearance, updateTerminalAppearance]);

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
                <span className={styles.schemeBadge}>
                  {terminalAppearance.highlightOverrides
                    ? `自定义 · 基于${HIGHLIGHT_PRESET_LABELS[terminalAppearance.highlightPreset]}`
                    : HIGHLIGHT_PRESET_LABELS[terminalAppearance.highlightPreset]}
                </span>
              </div>
              {/* 样例覆盖全部 18 个角色：预设之间的差异（如设备/压缩包/脚本/
                  信息词/IP）只有在样例中露出，切换预设时预览才有可见变化。 */}
              <div className={styles.terminalScreen}>
                <p>
                  <span style={roleStyle('prompt_user_host')}>root@server</span>
                  <span style={roleStyle('prompt_path')}>:/var/www</span># ls -la /srv/app
                </p>
                <p className={styles.muted}>total 4.2M</p>
                <p>
                  <span style={roleStyle('permissions')}>drwxr-xr-x</span> 5 root root 4096{' '}
                  <span style={roleStyle('date')}>Jun 12 09:30</span>{' '}
                  <span style={roleStyle('directory')}>apps</span>
                </p>
                <p>
                  <span style={roleStyle('permissions')}>lrwxrwxrwx</span> 1 root root 7{' '}
                  <span style={roleStyle('date')}>Jun 12 09:30</span>{' '}
                  <span style={roleStyle('symlink')}>latest</span> → v2.1
                </p>
                <p>
                  <span style={roleStyle('permissions')}>-rwxr-xr-x</span> 1 root root 842{' '}
                  <span style={roleStyle('date')}>Jun 12 09:30</span>{' '}
                  <span style={roleStyle('executable')}>deploy.sh</span>
                </p>
                <p>
                  <span style={roleStyle('permissions')}>-rw-r--r--</span> 1 root root 331{' '}
                  <span style={roleStyle('date')}>Jun 12 09:30</span>{' '}
                  <span style={roleStyle('script')}>setup.sh</span>
                </p>
                <p>
                  <span style={roleStyle('permissions')}>-rw-r--r--</span> 1 root root 12M{' '}
                  <span style={roleStyle('date')}>Jun 12 09:30</span>{' '}
                  <span style={roleStyle('archive')}>backup.tar.gz</span>
                </p>
                <p>
                  <span style={roleStyle('permissions')}>-rw-r--r--</span> 1 root root 86K{' '}
                  <span style={roleStyle('date')}>Jun 12 09:30</span>{' '}
                  <span style={roleStyle('log')}>error.log</span>
                </p>
                <p>
                  <span style={roleStyle('permissions')}>-rw-r--r--</span> 1 root root 321{' '}
                  <span style={roleStyle('date')}>Jun 12 09:30</span>{' '}
                  <span style={roleStyle('config')}>nginx.conf</span>
                </p>
                <p>
                  <span style={roleStyle('permissions')}>-rw-r--r--</span> 1 root root 4.5K{' '}
                  <span style={roleStyle('date')}>Jun 12 09:30</span>{' '}
                  <span style={roleStyle('media')}>banner.png</span>
                </p>
                <p>
                  <span style={roleStyle('permissions')}>crw-rw-rw-</span> 1 root tty 1, 3{' '}
                  <span style={roleStyle('date')}>Jun 12 09:30</span>{' '}
                  <span style={roleStyle('device')}>tty0</span>
                </p>
                <p>
                  <span style={roleStyle('prompt_user_host')}>root@server</span>
                  <span style={roleStyle('prompt_path')}>:/var/www</span># ssh admin@
                  <span style={roleStyle('ip')}>10.0.0.8</span>
                </p>
                <p>
                  <span style={roleStyle('kw_info')}>INFO</span> deploy finished in 3.2s
                </p>
                <p>
                  <span style={roleStyle('kw_warn')}>WARN</span> disk usage reached 87%
                </p>
                <p>
                  <span style={roleStyle('kw_error')}>ERROR</span> connection refused{' '}
                  <span className={styles.cursor} />
                </p>
                <p>
                  <span style={roleStyle('kw_success')}>OK</span> rollback completed
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
                  {terminalSchemeGroups.map((group) => (
                    <Fragment key={group.id}>
                      <div className={styles.paletteGroupTitle} role="presentation">
                        {group.label}
                      </div>
                      {terminalColorSchemes
                        .filter((scheme) => scheme.group === group.id)
                        .map((scheme) => {
                          const selected =
                            !followingApp && terminalAppearance.colorScheme === scheme.id;
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
                    </Fragment>
                  ))}
                </div>
              </div>
            </div>
          </div>
        </div>

        <HighlightColorSettings
          appearance={terminalAppearance}
          palette={palette}
          disabled={loading || saving}
          onChange={(next) => void updateTerminalAppearance(next)}
        />
      </section>

      <div className={styles.feedback}>
        {!persistenceAvailable ? (
          <p className={styles.notice}>浏览器仅预览效果，桌面客户端会自动保存设置。</p>
        ) : null}
        {adjustHint ? <p className={styles.notice}>{adjustHint}</p> : null}
        {error ? <p className={styles.error}>{error}</p> : null}
      </div>
    </div>
  );
}
