import { describe, expect, it, vi } from 'vitest';

import {
  attachTerminalSelectionAction,
  getTerminalSelectionAnchor,
  positionTerminalSelectionAction,
} from './terminal-selection';

/** 用最小事件目标验证选区动作生命周期，无需启动真实 xterm 或 WebView。 */
function selectionHarness(selection = 'selected output') {
  const hostListeners = new Map<string, (event: MouseEvent) => void>();
  const documentListeners = new Map<string, (event: MouseEvent) => void>();
  let active = true;
  let contentChangeListener: (() => void) | null = null;
  let selectionChangeListener: (() => void) | null = null;
  let viewportChangeListener: (() => void) | null = null;
  const ownerDocument = {
    addEventListener: vi.fn((type: string, listener: (event: MouseEvent) => void) => {
      documentListeners.set(type, listener);
    }),
    removeEventListener: vi.fn(),
  } as unknown as Document;
  const host = {
    ownerDocument,
    addEventListener: vi.fn((type: string, listener: (event: MouseEvent) => void) => {
      hostListeners.set(type, listener);
    }),
    removeEventListener: vi.fn(),
    getBoundingClientRect: () => ({ left: 100, top: 50, width: 640, height: 360 }),
  } as unknown as HTMLElement;
  const onChange = vi.fn();
  const detach = attachTerminalSelectionAction(
    host,
    {
      active: () => active,
      selection: () => selection,
      selectionAnchor: () => ({ compact: false, right: 320, y: 80, rowHeight: 18 }),
      onContentChange: (listener) => {
        contentChangeListener = listener;
        return { dispose: vi.fn() };
      },
      onSelectionChange: (listener) => {
        selectionChangeListener = listener;
        return { dispose: vi.fn() };
      },
      onViewportChange: (listener) => {
        viewportChangeListener = listener;
        return { dispose: vi.fn() };
      },
    },
    onChange
  );

  return {
    detach,
    host,
    ownerDocument,
    onChange,
    setActive: (next: boolean) => {
      active = next;
    },
    triggerHost: (type: string, event = {}) =>
      hostListeners.get(type)?.({ button: 0, clientX: 220, clientY: 130, ...event } as MouseEvent),
    triggerDocument: (type: string, event = {}) =>
      documentListeners.get(type)?.({
        button: 0,
        clientX: 220,
        clientY: 130,
        ...event,
      } as MouseEvent),
    triggerSelectionChange: () => selectionChangeListener?.(),
    triggerContentChange: () => contentChangeListener?.(),
    triggerViewportChange: () => viewportChangeListener?.(),
  };
}

describe('terminal selection action placement', () => {
  it('anchors inside the right edge of the first selected row', () => {
    expect(
      positionTerminalSelectionAction(
        { compact: false, right: 320, y: 80, rowHeight: 18 },
        { width: 640, height: 360 }
      )
    ).toEqual({ compact: false, left: 228, top: 80 });
  });

  it('moves inside the first selected row when the right edge has no room', () => {
    expect(
      positionTerminalSelectionAction(
        { compact: false, right: 620, y: 80, rowHeight: 18 },
        { width: 640, height: 360 }
      )
    ).toEqual({ compact: false, left: 528, top: 80 });
  });

  it('keeps the action inside a compact terminal viewport', () => {
    expect(
      positionTerminalSelectionAction(
        { compact: true, right: 118, y: 2, rowHeight: 18 },
        { width: 120, height: 60 }
      )
    ).toEqual({
      compact: true,
      left: 86,
      top: 6,
    });
  });

  it('uses the selected text end for a single row in either drag direction', () => {
    const host = {
      getBoundingClientRect: () => ({ left: 100, top: 50 }),
    } as unknown as HTMLElement;
    const screen = {
      getBoundingClientRect: () => ({ left: 110, top: 60, width: 800, height: 360 }),
    } as unknown as HTMLElement;
    const terminalElement = {
      querySelector: () => screen,
    } as unknown as HTMLElement;

    expect(
      getTerminalSelectionAnchor(
        host,
        terminalElement,
        { start: { x: 25, y: 4 }, end: { x: 10, y: 4 } },
        0,
        80,
        18
      )
    ).toEqual({ compact: true, right: 260, y: 90, rowHeight: 20 });
  });

  it('uses the screen edge for the first row of a multi-line selection', () => {
    const host = {
      getBoundingClientRect: () => ({ left: 100, top: 50 }),
    } as unknown as HTMLElement;
    const screen = {
      getBoundingClientRect: () => ({ left: 110, top: 60, width: 800, height: 360 }),
    } as unknown as HTMLElement;
    const terminalElement = {
      querySelector: () => screen,
    } as unknown as HTMLElement;

    expect(
      getTerminalSelectionAnchor(
        host,
        terminalElement,
        { start: { x: 20, y: 7 }, end: { x: 33, y: 5 } },
        0,
        80,
        18
      )
    ).toEqual({ compact: false, right: 810, y: 110, rowHeight: 20 });
  });
});

describe('terminal selection action lifecycle', () => {
  it('shows the final selection when xterm publishes before document mouseup', () => {
    vi.useFakeTimers();
    try {
      const harness = selectionHarness();

      harness.triggerHost('mousedown');
      harness.triggerDocument('mousemove', { clientX: 220, clientY: 130 });
      harness.triggerSelectionChange();
      harness.triggerDocument('mouseup');
      vi.runAllTimers();

      expect(harness.onChange).toHaveBeenLastCalledWith({
        compact: false,
        left: 228,
        top: 80,
        selection: 'selected output',
      });
    } finally {
      vi.useRealTimers();
    }
  });

  it('shows the final selection when xterm publishes after document mouseup', () => {
    const harness = selectionHarness();

    harness.triggerHost('mousedown');
    harness.triggerDocument('mouseup');
    harness.triggerSelectionChange();

    expect(harness.onChange).toHaveBeenLastCalledWith({
      compact: false,
      left: 228,
      top: 80,
      selection: 'selected output',
    });
  });

  it('waits for mouseup across repeated selection changes during a long drag', () => {
    vi.useFakeTimers();
    try {
      const harness = selectionHarness();

      harness.triggerHost('mousedown');
      harness.triggerSelectionChange();
      harness.triggerSelectionChange();
      harness.triggerDocument('mouseup');
      vi.runAllTimers();

      expect(harness.onChange).toHaveBeenLastCalledWith({
        compact: false,
        left: 228,
        top: 80,
        selection: 'selected output',
      });
    } finally {
      vi.useRealTimers();
    }
  });

  it('does not publish after content changes or the terminal becomes inactive', () => {
    vi.useFakeTimers();
    try {
      const contentChanged = selectionHarness();
      contentChanged.triggerHost('mousedown');
      contentChanged.triggerDocument('mouseup');
      contentChanged.triggerContentChange();
      vi.runAllTimers();
      expect(contentChanged.onChange).toHaveBeenLastCalledWith(null);

      const inactive = selectionHarness();
      inactive.triggerHost('mousedown');
      inactive.triggerDocument('mouseup');
      inactive.setActive(false);
      vi.runAllTimers();
      expect(inactive.onChange).toHaveBeenLastCalledWith(null);
    } finally {
      vi.useRealTimers();
    }
  });

  it('clears a completed gesture when xterm reports no selection', () => {
    vi.useFakeTimers();
    try {
      const harness = selectionHarness('');

      harness.triggerHost('mousedown');
      harness.triggerDocument('mouseup');
      vi.runAllTimers();

      expect(harness.onChange).toHaveBeenLastCalledWith(null);
    } finally {
      vi.useRealTimers();
    }
  });

  it('ignores mouseup without a primary-button selection started in this terminal', async () => {
    const harness = selectionHarness();

    harness.triggerDocument('mouseup');
    harness.triggerHost('mousedown', { button: 2 });
    harness.triggerDocument('mouseup', { button: 2 });
    await Promise.resolve();

    expect(harness.onChange).not.toHaveBeenCalled();
  });

  it('hides stale actions on a new gesture, selection change, viewport change, wheel, and detach', () => {
    const harness = selectionHarness();

    harness.triggerSelectionChange();
    harness.triggerHost('mousedown');
    harness.triggerViewportChange();
    harness.triggerHost('wheel');
    harness.detach();

    expect(harness.onChange).toHaveBeenCalledTimes(5);
    expect(harness.onChange).toHaveBeenNthCalledWith(1, null);
    expect(harness.onChange).toHaveBeenNthCalledWith(2, null);
    expect(harness.onChange).toHaveBeenNthCalledWith(3, null);
    expect(harness.onChange).toHaveBeenNthCalledWith(4, null);
    expect(harness.onChange).toHaveBeenNthCalledWith(5, null);
    expect(harness.host.removeEventListener).toHaveBeenCalledTimes(2);
    expect(harness.ownerDocument.removeEventListener).toHaveBeenCalledTimes(1);
  });

  it('does not publish a queued selection after detach', async () => {
    const harness = selectionHarness();

    harness.triggerHost('mousedown');
    harness.triggerDocument('mouseup');
    harness.detach();
    harness.triggerSelectionChange();
    await Promise.resolve();

    expect(harness.onChange).toHaveBeenCalledTimes(2);
    expect(harness.onChange).toHaveBeenNthCalledWith(1, null);
    expect(harness.onChange).toHaveBeenNthCalledWith(2, null);
  });
});
