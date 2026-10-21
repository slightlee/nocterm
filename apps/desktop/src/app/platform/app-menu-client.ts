import { listen, type UnlistenFn } from '@tauri-apps/api/event';

import { isDesktopRuntime } from '../../shared/lib/tauri-runtime';

const APP_MENU_EVENT = 'nocterm://app-menu-action';
const noopUnlisten: UnlistenFn = () => undefined;

export const APP_MENU_ACTIONS = [
  'open-about',
  'open-settings',
  'new-local-terminal',
  'close-current-tab',
  'toggle-sidebar',
  'toggle-ai',
] as const;

export type AppMenuAction = (typeof APP_MENU_ACTIONS)[number];

export function isAppMenuAction(value: unknown): value is AppMenuAction {
  return typeof value === 'string' && APP_MENU_ACTIONS.some((action) => action === value);
}

/** 浏览器预览没有原生菜单；桌面事件也只接受白名单动作，避免未知载荷进入应用壳。 */
export function onAppMenuAction(handler: (action: AppMenuAction) => void): Promise<UnlistenFn> {
  if (!isDesktopRuntime()) return Promise.resolve(noopUnlisten);
  return listen<unknown>(APP_MENU_EVENT, (event) => {
    if (isAppMenuAction(event.payload)) handler(event.payload);
  });
}
