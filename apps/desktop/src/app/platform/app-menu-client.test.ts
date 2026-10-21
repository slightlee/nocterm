import { describe, expect, it } from 'vitest';

import { isAppMenuAction } from './app-menu-client';

describe('app menu client', () => {
  it('accepts only the actions owned by the frontend shell', () => {
    expect(isAppMenuAction('new-local-terminal')).toBe(true);
    expect(isAppMenuAction('toggle-ai')).toBe(true);
    expect(isAppMenuAction('open-project')).toBe(false);
    expect(isAppMenuAction({ action: 'open-settings' })).toBe(false);
  });
});
