import { describe, expect, it } from 'vitest';

import { resolveAppTheme, toNativeAppTheme } from './app-theme';

describe('resolveAppTheme', () => {
  it('tracks the operating system only for the system preference', () => {
    expect(resolveAppTheme('system', false)).toBe('light');
    expect(resolveAppTheme('system', true)).toBe('dark');
  });

  it('keeps an explicit preference independent of the operating system', () => {
    expect(resolveAppTheme('light', true)).toBe('light');
    expect(resolveAppTheme('dark', false)).toBe('dark');
  });

  it('keeps native window chrome aligned with the preference', () => {
    expect(toNativeAppTheme('system')).toBeNull();
    expect(toNativeAppTheme('light')).toBe('light');
    expect(toNativeAppTheme('dark')).toBe('dark');
  });
});
