import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';

import { isDesktopRuntime } from '../../../shared/lib/tauri-runtime';
import {
  getAppTheme,
  getTerminalAppearance,
  normalizeSettingsError,
  setAppTheme,
  setNativeAppTheme,
  setTerminalAppearance,
} from '../api/settings-client';
import type { AppTheme, TerminalAppearance } from '../types/settings-types';
import { resolveAppTheme, toNativeAppTheme } from './app-theme';
import { SettingsContext } from './settings-context';
import { DEFAULT_TERMINAL_FONT_SIZE, resolveTerminalTheme } from './terminal-appearance';

/**
 * 设置 Provider 是应用主题的唯一写入者：AppShell 和页面不再各自监听系统主题，
 * 从而避免异步加载偏好时相互覆盖 `data-theme`。
 */
export function SettingsProvider({ children }: { children: ReactNode }) {
  const persistenceAvailable = isDesktopRuntime();
  const [appTheme, setAppThemeState] = useState<AppTheme>('system');
  const [terminalAppearance, setTerminalAppearanceState] = useState<TerminalAppearance>({
    fontSize: DEFAULT_TERMINAL_FONT_SIZE,
    colorScheme: 'follow_app',
  });
  const [loading, setLoading] = useState(persistenceAvailable);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!persistenceAvailable) return;
    let disposed = false;
    void Promise.all([getAppTheme(), getTerminalAppearance()])
      .then(([themeResponse, terminalResponse]) => {
        if (disposed) return;
        setAppThemeState(themeResponse.value);
        setTerminalAppearanceState(terminalResponse);
      })
      .catch((reason: unknown) => {
        if (!disposed) setError(normalizeSettingsError(reason).message);
      })
      .finally(() => {
        if (!disposed) setLoading(false);
      });
    return () => {
      disposed = true;
    };
  }, [persistenceAvailable]);

  useEffect(() => {
    const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)');
    const applyTheme = () => {
      const resolvedAppTheme = resolveAppTheme(appTheme, mediaQuery.matches);
      document.documentElement.dataset.theme = resolvedAppTheme;
      document.documentElement.dataset.terminalTheme = resolveTerminalTheme(
        terminalAppearance.colorScheme,
        resolvedAppTheme
      );
      document.documentElement.dataset.terminalFontSize = String(terminalAppearance.fontSize);
    };
    applyTheme();
    if (appTheme !== 'system') return;
    mediaQuery.addEventListener('change', applyTheme);
    return () => mediaQuery.removeEventListener('change', applyTheme);
  }, [appTheme, terminalAppearance]);

  useEffect(() => {
    if (!persistenceAvailable) return;
    // Overlay 标题栏属于原生窗口；只改 data-theme 会在 macOS 留下浅色窗口框线。
    void setNativeAppTheme(toNativeAppTheme(appTheme)).catch((reason: unknown) => {
      setError(normalizeSettingsError(reason).message);
    });
  }, [appTheme, persistenceAvailable]);

  const updateAppTheme = useCallback(
    async (theme: AppTheme) => {
      setError(null);
      if (!persistenceAvailable) {
        // 浏览器只提供明确的界面预览，不宣称已把偏好写入桌面数据库。
        setAppThemeState(theme);
        return;
      }
      setSaving(true);
      try {
        const response = await setAppTheme(theme);
        setAppThemeState(response.value);
      } catch (reason) {
        setError(normalizeSettingsError(reason).message);
      } finally {
        setSaving(false);
      }
    },
    [persistenceAvailable]
  );

  const updateTerminalAppearance = useCallback(
    async (appearance: TerminalAppearance) => {
      setError(null);
      // 先更新内存让预览和已打开终端立即响应；失败时恢复服务端确认过的旧值。
      const previous = terminalAppearance;
      setTerminalAppearanceState(appearance);
      if (!persistenceAvailable) return;
      setSaving(true);
      try {
        const response = await setTerminalAppearance(appearance);
        setTerminalAppearanceState(response);
      } catch (reason) {
        setTerminalAppearanceState(previous);
        setError(normalizeSettingsError(reason).message);
      } finally {
        setSaving(false);
      }
    },
    [persistenceAvailable, terminalAppearance]
  );

  const value = useMemo(
    () => ({
      appTheme,
      terminalAppearance,
      loading,
      saving,
      persistenceAvailable,
      error,
      updateAppTheme,
      updateTerminalAppearance,
    }),
    [
      appTheme,
      error,
      loading,
      persistenceAvailable,
      saving,
      terminalAppearance,
      updateAppTheme,
      updateTerminalAppearance,
    ]
  );

  return <SettingsContext.Provider value={value}>{children}</SettingsContext.Provider>;
}
