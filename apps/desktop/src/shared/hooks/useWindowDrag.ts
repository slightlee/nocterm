import { getCurrentWindow } from '@tauri-apps/api/window';
import { useCallback, type MouseEvent } from 'react';

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

function isNoDragTarget(target: EventTarget | null) {
  return target instanceof HTMLElement && Boolean(target.closest(NO_DRAG_SELECTOR));
}

function startWindowDrag(event: MouseEvent<HTMLElement>) {
  event.preventDefault();
  if (!isDesktopRuntime()) return;
  void getCurrentWindow()
    .startDragging()
    .catch((error: unknown) => console.warn('启动窗口拖拽失败', error));
}

/** 保持旧版无侵入标题栏拖拽，同时跳过所有交互控件。 */
export function useTopWindowDrag() {
  return useCallback((event: MouseEvent<HTMLElement>) => {
    if (event.button !== 0) return;
    if (event.clientY > TOP_DRAG_HEIGHT) return;
    if (event.clientX < TRAFFIC_LIGHT_SAFE_WIDTH) return;
    if (isNoDragTarget(event.target)) return;
    startWindowDrag(event);
  }, []);
}

/** ActivityBar 没有标题栏控件，沿用旧版整栏拖拽行为。 */
export function useWindowDrag() {
  return useCallback((event: MouseEvent<HTMLElement>) => {
    if (event.button !== 0 || isNoDragTarget(event.target)) return;
    startWindowDrag(event);
  }, []);
}
