import { getCurrentWindow } from '@tauri-apps/api/window';
import { useCallback, type MouseEvent } from 'react';

import { isWindowsDesktopRuntime } from '../lib/desktop-platform';
import { isDesktopRuntime } from '../lib/tauri-runtime';

const TOP_DRAG_HEIGHT = 52;
const TRAFFIC_LIGHT_SAFE_WIDTH = 92;
const NO_DRAG_SELECTOR = [
  'a',
  'button',
  'input',
  'textarea',
  'select',
  '[contenteditable="true"]',
  '[data-no-window-drag="true"]',
  '[role="button"]',
].join(',');

export function isNoDragTarget(target: EventTarget | null) {
  // SVGElement 同样支持 closest；按 DOM 能力判断，避免点击图标路径时误启动窗口拖拽。
  if (!target || typeof (target as Element).closest !== 'function') return false;
  return Boolean((target as Element).closest(NO_DRAG_SELECTOR));
}

function startWindowDrag(event: MouseEvent<HTMLElement>) {
  event.preventDefault();
  if (!isDesktopRuntime()) return;
  void getCurrentWindow()
    .startDragging()
    .catch((error: unknown) => console.warn('启动窗口拖拽失败', error));
}

/** Windows 自绘标题栏没有系统红绿灯，顶部拖拽区无需左侧避让。 */
function topDragSafeLeft(): number {
  return isWindowsDesktopRuntime() ? 0 : TRAFFIC_LIGHT_SAFE_WIDTH;
}

/** 保持旧版无侵入标题栏拖拽，同时跳过所有交互控件。 */
export function useTopWindowDrag() {
  return useCallback((event: MouseEvent<HTMLElement>) => {
    if (event.button !== 0) return;
    if (event.clientY > TOP_DRAG_HEIGHT) return;
    if (event.clientX < topDragSafeLeft()) return;
    if (isNoDragTarget(event.target)) return;
    startWindowDrag(event);
  }, []);
}

/** 双击顶部空白切换最大化：仅 Windows 自绘标题栏启用；
    macOS 保持 Overlay 原有行为不变。 */
export function useTopWindowDoubleClick() {
  return useCallback((event: MouseEvent<HTMLElement>) => {
    if (!isWindowsDesktopRuntime()) return;
    if (event.button !== 0) return;
    if (event.clientY > TOP_DRAG_HEIGHT) return;
    if (event.clientX < topDragSafeLeft()) return;
    if (isNoDragTarget(event.target)) return;
    if (!isDesktopRuntime()) return;
    void getCurrentWindow()
      .toggleMaximize()
      .catch((error: unknown) => console.warn('切换窗口最大化失败', error));
  }, []);
}

/** ActivityBar 没有标题栏控件，沿用旧版整栏拖拽行为。 */
export function useWindowDrag() {
  return useCallback((event: MouseEvent<HTMLElement>) => {
    if (event.button !== 0 || isNoDragTarget(event.target)) return;
    startWindowDrag(event);
  }, []);
}
