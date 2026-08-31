import { describe, expect, it } from 'vitest';

import { formatArchitectureLabel, formatPlatformLabel } from './about-info';

describe('about info', () => {
  it('formats familiar platform labels without hiding unknown values', () => {
    expect(formatPlatformLabel('macos')).toBe('macOS');
    expect(formatPlatformLabel('windows')).toBe('Windows');
    expect(formatPlatformLabel('linux')).toBe('linux');
    expect(formatArchitectureLabel('aarch64')).toBe('ARM64');
    expect(formatArchitectureLabel('x86_64')).toBe('x86_64');
  });
});
