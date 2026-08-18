import type { ReactNode } from 'react';

import { DEFAULT_CONNECTION_ICON, normalizeConnectionIcon } from './connection-icon-meta';
import type { ConnectionIcon } from '../types/connection-types';

export type ConnectionIconColor = 'red' | 'orange' | 'blue' | 'green' | 'purple';

export interface ConnectionIconOption {
  value: ConnectionIcon;
  label: string;
  color: ConnectionIconColor;
  icon: ReactNode;
}

export const CONNECTION_ICON_OPTIONS: ConnectionIconOption[] = [
  {
    value: 'server',
    label: '服务器',
    color: 'blue',
    icon: (
      <svg viewBox="0 0 24 24">
        <rect x="5" y="4" width="14" height="16" rx="2" />
        <path d="M8 8h4M8 13h2" />
      </svg>
    ),
  },
  {
    value: 'cloud',
    label: '云服务器',
    color: 'purple',
    icon: (
      <svg viewBox="0 0 24 24">
        <path d="M18 10h-1.26A8 8 0 1 0 9 20h9a5 5 0 0 0 0-10z" />
      </svg>
    ),
  },
  {
    value: 'database',
    label: '数据库',
    color: 'orange',
    icon: (
      <svg viewBox="0 0 24 24">
        <ellipse cx="12" cy="6" rx="7" ry="3" />
        <path d="M5 6v10c0 1.7 3.1 3 7 3s7-1.3 7-3V6M5 11c0 1.7 3.1 3 7 3s7-1.3 7-3" />
      </svg>
    ),
  },
  {
    value: 'terminal',
    label: '终端',
    color: 'green',
    icon: (
      <svg viewBox="0 0 24 24">
        <polyline points="4 17 10 11 4 5" />
        <line x1="12" y1="19" x2="20" y2="19" />
      </svg>
    ),
  },
  {
    value: 'web',
    label: '网站',
    color: 'blue',
    icon: (
      <svg viewBox="0 0 24 24">
        <circle cx="12" cy="12" r="9" />
        <path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18" />
      </svg>
    ),
  },
  {
    value: 'build',
    label: '构建',
    color: 'red',
    icon: (
      <svg viewBox="0 0 24 24">
        <rect x="4" y="5" width="16" height="11" rx="2" />
        <path d="M9 20h6M12 16v4" />
      </svg>
    ),
  },
  {
    value: 'cache',
    label: '缓存',
    color: 'orange',
    icon: (
      <svg viewBox="0 0 24 24">
        <path d="M6 7h12M6 12h12M6 17h12M8 4h8a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z" />
      </svg>
    ),
  },
  {
    value: 'storage',
    label: '存储',
    color: 'purple',
    icon: (
      <svg viewBox="0 0 24 24">
        <path d="M3 7.5A2.5 2.5 0 0 1 5.5 5H10l2 2.5h6.5A2.5 2.5 0 0 1 21 10v7a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 17z" />
      </svg>
    ),
  },
];

export function getConnectionIconOption(value?: ConnectionIcon | null): ConnectionIconOption {
  const normalized = normalizeConnectionIcon(value ?? DEFAULT_CONNECTION_ICON);
  return (
    CONNECTION_ICON_OPTIONS.find((option) => option.value === normalized) ??
    CONNECTION_ICON_OPTIONS[0]
  );
}
