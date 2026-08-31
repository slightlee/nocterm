import type { AppTheme } from '../types/settings-types';

export type ResolvedAppTheme = Exclude<AppTheme, 'system'>;

/** 用户偏好与系统状态分离，便于 system 模式持续响应操作系统变化。 */
export function resolveAppTheme(preference: AppTheme, systemDark: boolean): ResolvedAppTheme {
  if (preference === 'system') return systemDark ? 'dark' : 'light';
  return preference;
}

/** Tauri 使用 null 表示继续跟随系统；显式偏好则同步到原生标题栏与窗口边框。 */
export function toNativeAppTheme(preference: AppTheme): ResolvedAppTheme | null {
  return preference === 'system' ? null : preference;
}
