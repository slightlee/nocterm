import { isDesktopRuntime } from './tauri-runtime';

export function isWindowsUserAgent(userAgent: string): boolean {
  return /Windows/i.test(userAgent);
}

/** 浏览器预览保持普通页面布局，仅 Windows WebView 启用自绘窗口控件。 */
export function isWindowsDesktopRuntime(): boolean {
  return isDesktopRuntime() && isWindowsUserAgent(window.navigator.userAgent);
}
