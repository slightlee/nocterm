import { describe, expect, it } from 'vitest';

import { isNoDragTarget } from './useWindowDrag';

describe('isNoDragTarget', () => {
  it('blocks an SVG descendant inside an interactive control', () => {
    let receivedSelector = '';
    const svgTarget = {
      closest(selector: string) {
        receivedSelector = selector;
        return {};
      },
    } as unknown as EventTarget;

    expect(isNoDragTarget(svgTarget)).toBe(true);
    expect(receivedSelector).toContain('button');
  });

  it('allows a non-interactive blank area to start window dragging', () => {
    const blankTarget = {
      closest() {
        return null;
      },
    } as unknown as EventTarget;

    expect(isNoDragTarget(blankTarget)).toBe(false);
  });

  it('ignores targets that are not DOM elements', () => {
    expect(isNoDragTarget(null)).toBe(false);
    expect(isNoDragTarget({} as EventTarget)).toBe(false);
  });
});
