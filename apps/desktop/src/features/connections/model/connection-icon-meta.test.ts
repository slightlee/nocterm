import { describe, expect, it } from 'vitest';

import { DEFAULT_CONNECTION_ICON, normalizeConnectionIcon } from './connection-icon-meta';

describe('connection icon metadata', () => {
  it('falls back safely for legacy or unknown values', () => {
    expect(normalizeConnectionIcon('database')).toBe('database');
    expect(normalizeConnectionIcon('legacy-icon')).toBe(DEFAULT_CONNECTION_ICON);
    expect(normalizeConnectionIcon(null)).toBe(DEFAULT_CONNECTION_ICON);
  });
});
