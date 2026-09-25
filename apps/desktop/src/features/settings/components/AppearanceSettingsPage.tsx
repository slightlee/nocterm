import { Fragment, useEffect, useMemo, useRef, useState, type CSSProperties } from 'react';

import { isWindowsUserAgent } from '../../../shared/lib/desktop-platform';
import type { AppTheme, HighlightRoleId } from '../types/settings-types';
import { useSettings } from '../model/use-settings';
import { HighlightColorSettings } from './HighlightColorSettings';
import { useTerminalPalette } from '../model/use-terminal-palette';
import {
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

/**
 * 右栏双页签：配色方案与字体颜色一次只显示一个面板，右栏高度恒定、
 * 不存在展开/收起动作。字体大小与跟随应用是「调整怎么看预览」的参数，
 * 归位到预览上方的工具条而非右栏。
 */
type ControlTab = 'scheme' | 'color';

/** 点击预览文字时的定位请求：切到字体颜色页签并滚动到对应角色行。 */
interface RevealRequest {
  role: HighlightRoleId;
  n: number;
}

/** 外观页使用 ANSI 输出示例呈现色板，选择结果会同步到已打开的本地与 SSH 会话。 */
export function AppearanceSettingsPage() {
  const {
    appTheme,
    terminalAppearance,
    terminalThemeId,
    loading,
    saving,
    persistenceAvailable,
    error,
    updateAppTheme,
    updateTerminalAppearance,
  } = useSettings();
  const [fontSizeDraft, setFontSizeDraft] = useState<number | null>(null);
  const [activeTab, setActiveTab] = useState<ControlTab>('scheme');
  const [activeRole, setActiveRole] = useState<HighlightRoleId | null>(null);
  const [reveal, setReveal] = useState<RevealRequest | null>(null);
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
  const palette = useTerminalPalette(terminalThemeId);
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

  // 双向联动：悬停任一侧，另一侧同步高亮；点击预览文字滚动定位到角色行。
  const hoverProps = (id: HighlightRoleId) => ({
    onMouseEnter: () => setActiveRole(id),
    onMouseLeave: () => setActiveRole(null),
  });
  const revealRole = (id: HighlightRoleId) => {
    setActiveTab('color');
    setReveal((prev) => ({ role: id, n: (prev?.n ?? 0) + 1 }));
  };
  const roleSpan = (id: HighlightRoleId, text: string) => (
    <span
      className={`${styles.roleSpan} ${activeRole === id ? styles.roleSpanActive : ''}`}
      data-role={id}
      onClick={() => revealRole(id)}
      role="presentation"
      style={roleStyle(id)}
      {...hoverProps(id)}
    >
      {text}
    </span>
  );

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

        <div className={styles.studioLayout}>
          <div className={styles.previewPane}>
            <div className={styles.previewControls}>
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
                  <small>随应用主题切换配色</small>
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

            <div className={styles.previewScope} data-terminal-theme={selectedScheme.id}>
              <div
                className={`${styles.terminalPreview} nocterm-terminal-surface`}
                style={{ fontSize: `${displayedFontSize}px` }}
                aria-label="ANSI 终端配色预览"
              >
                <div className={styles.previewToolbar} aria-hidden="true">
                  {/* 窗口装饰按当前平台渲染，与产品在该平台的真实窗口 chrome 同构：
                      Windows 自绘控件在右上角，其余平台为左上红绿灯。 */}
                  {isWindowsUserAgent(window.navigator.userAgent) ? (
                    <span className={styles.windowControls}>
                      <svg stroke="currentColor" viewBox="0 0 10 10">
                        <path d="M1 5h8" />
                      </svg>
                      <svg fill="none" stroke="currentColor" viewBox="0 0 10 10">
                        <rect height="7" width="7" x="1.5" y="1.5" />
                      </svg>
                      <svg stroke="currentColor" viewBox="0 0 10 10">
                        <path d="M1.5 1.5l7 7M8.5 1.5l-7 7" />
                      </svg>
                    </span>
                  ) : (
                    <>
                      <span className={styles.toolbarDot} />
                      <span className={styles.toolbarDot} />
                      <span className={styles.toolbarDot} />
                    </>
                  )}
                </div>
                {/* 样例按「开发者的真实操作流」组织，覆盖全部 18 个角色：
                    预设差异（设备/压缩包/脚本等）只有露出，切换时预览才有变化；
                    带角色的文字同时是联动入口，悬停/点击与右栏角色行互链。 */}
                <div className={styles.terminalScreen}>
                  <p>
                    {roleSpan('prompt_user_host', 'deploy@prod-01')}
                    {roleSpan('prompt_path', ':/srv/apps')}$ ls -l /srv/apps
                  </p>
                  <p className={styles.muted}>total 92M</p>
                  <p>
                    {roleSpan('permissions', 'drwxr-xr-x')} 6 deploy ops 4.0K{' '}
                    {roleSpan('date', 'Sep 24 09:12')} {roleSpan('directory', 'api')}
                  </p>
                  <p>
                    {roleSpan('permissions', '-rwxr-xr-x')} 1 deploy ops 89M{' '}
                    {roleSpan('date', 'Sep 23 18:40')} {roleSpan('executable', 'migrate')}
                  </p>
                  <p>
                    {roleSpan('permissions', 'lrwxrwxrwx')} 1 deploy ops 29{' '}
                    {roleSpan('date', 'Sep 21 11:03')} {roleSpan('symlink', 'current')} →
                    releases/v2.4.1
                  </p>
                  <p>
                    {roleSpan('permissions', '-rw-r--r--')} 1 deploy ops 331{' '}
                    {roleSpan('date', 'Sep 20 08:00')} {roleSpan('config', 'nginx.conf')}
                  </p>
                  <p>
                    {roleSpan('permissions', '-rw-r--r--')} 1 deploy ops 86K{' '}
                    {roleSpan('date', 'Sep 20 08:00')} {roleSpan('log', 'error.log')}
                  </p>
                  <p>
                    {roleSpan('permissions', '-rw-r--r--')} 1 deploy ops 12M{' '}
                    {roleSpan('date', 'Sep 19 22:41')} {roleSpan('archive', 'backup-0919.tar.gz')}
                  </p>
                  <p>
                    {roleSpan('permissions', '-rw-r--r--')} 1 deploy ops 4.5K{' '}
                    {roleSpan('date', 'Sep 18 10:15')} {roleSpan('media', 'banner.png')}
                  </p>
                  <p>
                    {roleSpan('prompt_user_host', 'deploy@prod-01')}
                    {roleSpan('prompt_path', ':/srv/apps')}$ ls -l /dev | head -2
                  </p>
                  <p>
                    {roleSpan('permissions', 'brw-rw----')} 1 root disk 8, 0{' '}
                    {roleSpan('date', 'Sep 18 10:15')} {roleSpan('device', 'sda')}
                  </p>
                  <p>
                    {roleSpan('prompt_user_host', 'deploy@prod-01')}
                    {roleSpan('prompt_path', ':/srv/apps')}$ tail -n 3 /var/log/app/error.log
                  </p>
                  <p>
                    {roleSpan('date', '09:31:02')} {roleSpan('kw_error', 'ERROR')} [api] connection
                    refused → {roleSpan('ip', '10.0.0.5')}:5432
                  </p>
                  <p>
                    {roleSpan('date', '09:31:03')} {roleSpan('kw_warn', 'WARN')} [api] retry 2/5
                    backoff 400ms
                  </p>
                  <p>
                    {roleSpan('date', '09:31:05')} {roleSpan('kw_info', 'INFO')} [api] healthcheck
                    ok
                  </p>
                  <p>
                    {roleSpan('prompt_user_host', 'deploy@prod-01')}
                    {roleSpan('prompt_path', ':/srv/apps')}$ find /srv/apps -name '*.conf'
                  </p>
                  <p>
                    {roleSpan('directory', '/srv/apps/nginx/conf/')}
                    {roleSpan('config', 'nginx.conf')}
                  </p>
                  <p>
                    {roleSpan('prompt_user_host', 'deploy@prod-01')}
                    {roleSpan('prompt_path', ':/srv/apps')}$ grep -n ERROR /srv/apps/
                    {roleSpan('symlink', 'current')} | head -1
                  </p>
                  <p>
                    {roleSpan('directory', '/srv/apps/api/logs/')}
                    {roleSpan('log', 'error.log')}:3: {roleSpan('date', '09:31:02')}{' '}
                    {roleSpan('kw_error', 'ERROR')} connection refused
                  </p>
                  <p>
                    {roleSpan('prompt_user_host', 'deploy@prod-01')}
                    {roleSpan('prompt_path', ':/srv/apps')}$ /usr/local/bin/
                    {roleSpan('executable', 'migrate')} --version
                  </p>
                  <p>
                    {roleSpan('executable', 'migrate')} 2.4.1 (build{' '}
                    {roleSpan('date', '2026-09-18')})
                  </p>
                  <p>
                    {roleSpan('prompt_user_host', 'deploy@prod-01')}
                    {roleSpan('prompt_path', ':/srv/apps')}$ ./scripts/
                    {roleSpan('script', 'deploy.sh')} --env prod
                  </p>
                  <p>
                    {roleSpan('date', '09:32:10')} {roleSpan('kw_success', 'OK')} deploy finished in
                    3.2s
                  </p>
                  <p>
                    {roleSpan('prompt_user_host', 'deploy@prod-01')}
                    {roleSpan('prompt_path', ':/srv/apps')}$ <span className={styles.cursor} />
                  </p>
                </div>
              </div>
            </div>
          </div>

          <div className={styles.controlsPane}>
            <div className={styles.tabBar} role="group" aria-label="外观设置分区">
              <button
                aria-pressed={activeTab === 'scheme'}
                className={`${styles.tab} ${activeTab === 'scheme' ? styles.tabActive : ''}`}
                onClick={() => setActiveTab('scheme')}
                type="button"
              >
                配色方案
              </button>
              <button
                aria-pressed={activeTab === 'color'}
                className={`${styles.tab} ${activeTab === 'color' ? styles.tabActive : ''}`}
                onClick={() => setActiveTab('color')}
                type="button"
              >
                字体颜色
              </button>
            </div>

            {activeTab === 'scheme' ? (
              <div className={styles.panelBody} role="presentation">
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
                              className={`${styles.paletteOption} ${selected ? styles.selectedPalette : ''}`}
                              data-terminal-theme={scheme.id}
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
                              <span
                                className={`${styles.paletteSwatch} nocterm-terminal-surface`}
                                aria-hidden="true"
                              >
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
            ) : (
              <div className={styles.panelBody} role="presentation">
                <HighlightColorSettings
                  activeRole={activeRole}
                  appearance={terminalAppearance}
                  disabled={loading || saving}
                  onHoverRole={setActiveRole}
                  palette={palette}
                  reveal={reveal}
                  onChange={(next) => void updateTerminalAppearance(next)}
                />
              </div>
            )}
          </div>
        </div>
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
