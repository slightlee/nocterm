import { setTheme as setTauriTheme } from '@tauri-apps/api/app';
import { invoke } from '@tauri-apps/api/core';
import { openUrl } from '@tauri-apps/plugin-opener';

import type {
  AppTheme,
  AppThemeResponse,
  SettingsError,
  TerminalAppearance,
} from '../types/settings-types';

function isSettingsError(value: unknown): value is SettingsError {
  if (typeof value !== 'object' || value === null) return false;
  const candidate = value as Record<string, unknown>;
  return (
    typeof candidate.code === 'string' &&
    typeof candidate.message === 'string' &&
    typeof candidate.retryable === 'boolean'
  );
}

/** IPC 拒绝值可能不是 Error，Feature 边界统一成设置页可消费的稳定结构。 */
export function normalizeSettingsError(error: unknown): SettingsError {
  if (isSettingsError(error)) return error;
  return {
    code: 'SETTINGS_UNKNOWN_ERROR',
    message: error instanceof Error ? error.message : '设置操作失败，请稍后重试',
    retryable: true,
  };
}

export function getAppTheme(): Promise<AppThemeResponse> {
  return invoke<AppThemeResponse>('settings_app_theme_get');
}

export function setAppTheme(value: AppTheme): Promise<AppThemeResponse> {
  return invoke<AppThemeResponse>('settings_app_theme_set', { value });
}

/** 原生窗口主题与 WebView 分开设置；该调用只允许由桌面运行时触发。 */
export function setNativeAppTheme(value: Exclude<AppTheme, 'system'> | null): Promise<void> {
  return setTauriTheme(value);
}

export function getTerminalAppearance(): Promise<TerminalAppearance> {
  return invoke<TerminalAppearance>('settings_terminal_appearance_get');
}

export function setTerminalAppearance(request: TerminalAppearance): Promise<TerminalAppearance> {
  return invoke<TerminalAppearance>('settings_terminal_appearance_set', { request });
}

/** 外部地址统一交给系统默认浏览器，避免在桌面 WebView 内导航离开应用。 */
export function openExternalUrl(url: string): Promise<void> {
  return openUrl(url);
}
