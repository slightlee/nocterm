import { describe, expect, it } from 'vitest';

import { resolveSessionPickerPosition } from './server-session-picker-layout';

describe('resolveSessionPickerPosition', () => {
  it('opens beside the server row when the right side has room', () => {
    expect(
      resolveSessionPickerPosition(
        { left: 240, right: 264, top: 210 },
        { width: 264, height: 400 },
        { width: 1280, height: 720 }
      )
    ).toEqual({ left: 272, top: 210, width: 264, maxHeight: 440 });
  });

  it('uses the left side when the window edge blocks the right side', () => {
    expect(
      resolveSessionPickerPosition(
        { left: 1000, right: 1024, top: 210 },
        { width: 264, height: 400 },
        { width: 1280, height: 720 }
      )
    ).toEqual({ left: 728, top: 210, width: 264, maxHeight: 440 });
  });

  it('shrinks to the viewport and keeps a long list inside a short window', () => {
    expect(
      resolveSessionPickerPosition(
        { left: 78, right: 102, top: 350 },
        { width: 264, height: 700 },
        { width: 240, height: 420 }
      )
    ).toEqual({ left: 12, top: 12, width: 216, maxHeight: 396 });
  });

  it('moves upward when the server row is too close to the bottom edge', () => {
    expect(
      resolveSessionPickerPosition(
        { left: 100, right: 124, top: 380 },
        { width: 264, height: 240 },
        { width: 800, height: 500 }
      )
    ).toEqual({ left: 132, top: 248, width: 264, maxHeight: 440 });
  });

  it('caps an unusually long list in a tall window', () => {
    expect(
      resolveSessionPickerPosition(
        { left: 240, right: 264, top: 210 },
        { width: 264, height: 900 },
        { width: 1280, height: 900 }
      )
    ).toEqual({ left: 272, top: 210, width: 264, maxHeight: 440 });
  });
});
