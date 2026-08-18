import type { ConnectionIcon } from '../types/connection-types';

export const DEFAULT_CONNECTION_ICON: ConnectionIcon = 'server';

export const CONNECTION_ICON_VALUES = [
  'server',
  'cloud',
  'database',
  'terminal',
  'web',
  'build',
  'cache',
  'storage',
] as const satisfies readonly ConnectionIcon[];

export function isConnectionIcon(value: unknown): value is ConnectionIcon {
  return typeof value === 'string' && (CONNECTION_ICON_VALUES as readonly string[]).includes(value);
}

export function normalizeConnectionIcon(value: unknown): ConnectionIcon {
  return isConnectionIcon(value) ? value : DEFAULT_CONNECTION_ICON;
}
