import { describe, expect, it } from 'vitest';

import { runtimeStatusLabel } from './runtime-status';

describe('runtimeStatusLabel', () => {
  it('uses an explicit label for every runtime state', () => {
    expect(runtimeStatusLabel('checking')).toBe('checking runtime');
    expect(runtimeStatusLabel('preview')).toBe('browser preview');
    expect(runtimeStatusLabel('ready')).toBe('desktop core ready');
    expect(runtimeStatusLabel('error')).toBe('runtime unavailable');
  });
});
