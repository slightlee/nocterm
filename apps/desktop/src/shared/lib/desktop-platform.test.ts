import { describe, expect, it } from 'vitest';

import { isWindowsUserAgent } from './desktop-platform';

describe('isWindowsUserAgent', () => {
  it('recognizes the Windows WebView user agent', () => {
    expect(
      isWindowsUserAgent(
        'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/140 Safari/537.36 Edg/140'
      )
    ).toBe(true);
  });

  it('does not treat macOS as Windows', () => {
    expect(
      isWindowsUserAgent('Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15')
    ).toBe(false);
  });
});
