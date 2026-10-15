import { getCurrentWindow } from '@tauri-apps/api/window';

export function closeDesktopWindow(): Promise<void> {
  return getCurrentWindow().close();
}

export function minimizeDesktopWindow(): Promise<void> {
  return getCurrentWindow().minimize();
}

export function toggleDesktopWindowMaximize(): Promise<void> {
  return getCurrentWindow().toggleMaximize();
}

/** 最大化图标随系统窗口状态更新，取消订阅由组件生命周期统一负责。 */
export async function watchDesktopWindowMaximized(
  handler: (maximized: boolean) => void
): Promise<() => void> {
  const currentWindow = getCurrentWindow();
  handler(await currentWindow.isMaximized());
  return currentWindow.onResized(async () => {
    handler(await currentWindow.isMaximized());
  });
}
