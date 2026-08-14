import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

import type { RuntimeHealth } from '../model/runtime-types';

const HEALTH_EVENT = 'nocterm://health-checked';

export function isDesktopRuntime(): boolean {
  return '__TAURI_INTERNALS__' in window;
}

export function checkRuntimeHealth(): Promise<RuntimeHealth> {
  return invoke<RuntimeHealth>('health_check');
}

export function onRuntimeHealth(handler: (health: RuntimeHealth) => void): Promise<UnlistenFn> {
  return listen<RuntimeHealth>(HEALTH_EVENT, (event) => handler(event.payload));
}
