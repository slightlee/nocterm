import { describe, expect, it, vi } from 'vitest';

import {
  type TerminalClipboardTarget,
  type TerminalCopySource,
  attachTerminalClipboard,
  copyTerminalSelection,
  createCopyKeyHandler,
  isCopyShortcut,
} from './terminal-clipboard';

/** 造一个只带修饰键信息的按键事件，避免为纯判定逻辑引入 DOM。 */
function key(
  value: string,
  modifiers: Partial<Record<'ctrlKey' | 'shiftKey' | 'altKey' | 'metaKey', boolean>> = {}
) {
  return {
    key: value,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
    metaKey: false,
    ...modifiers,
  };
}

function copySource(overrides: Partial<TerminalCopySource> = {}): TerminalCopySource {
  return {
    hasSelection: () => true,
    selection: () => 'selected',
    clearSelection: vi.fn(),
    onUnavailable: vi.fn(),
    ...overrides,
  };
}

function clipboardTarget(
  overrides: Partial<TerminalClipboardTarget> = {}
): TerminalClipboardTarget {
  return {
    ...copySource(),
    paste: vi.fn(),
    ...overrides,
  };
}

/** 用最小事件目标驱动原生 contextmenu 监听，无需在单元测试中启动 WebView。 */
function contextMenuHarness() {
  let listener: ((event: MouseEvent) => void) | null = null;
  const container = {
    addEventListener: vi.fn((_type: string, next: (event: MouseEvent) => void) => {
      listener = next;
    }),
    removeEventListener: vi.fn(),
  } as unknown as HTMLElement;
  const event = {
    preventDefault: vi.fn(),
    stopPropagation: vi.fn(),
  } as unknown as MouseEvent;

  return {
    container,
    event,
    trigger: () => {
      if (!listener) throw new Error('contextmenu listener was not attached');
      listener(event);
    },
  };
}

describe('isCopyShortcut', () => {
  it('treats Ctrl+C as copy only when something is selected', () => {
    // 没有选区时 Ctrl+C 必须原样当中断信号送给远端，否则 Ctrl+C 就再也停不掉命令。
    expect(isCopyShortcut(key('c', { ctrlKey: true }), true)).toBe(true);
    expect(isCopyShortcut(key('c', { ctrlKey: true }), false)).toBe(false);
  });

  it('copies unconditionally for Ctrl+Shift+C and Ctrl+Insert', () => {
    // 按下 Shift 时 key 是大写的 C，两种写法都要认。
    expect(isCopyShortcut(key('C', { ctrlKey: true, shiftKey: true }), false)).toBe(true);
    expect(isCopyShortcut(key('c', { ctrlKey: true, shiftKey: true }), false)).toBe(true);
    expect(isCopyShortcut(key('Insert', { ctrlKey: true }), false)).toBe(true);
  });

  it('leaves other combinations to the terminal and the system', () => {
    expect(isCopyShortcut(key('c'), true)).toBe(false);
    // Alt+字母是 Meta 前缀，Cmd+C 交给 macOS 的原生复制命令。
    expect(isCopyShortcut(key('c', { ctrlKey: true, altKey: true }), true)).toBe(false);
    expect(isCopyShortcut(key('c', { metaKey: true }), true)).toBe(false);
    // Shift+Insert 是粘贴，绝不能被复制吃掉。
    expect(isCopyShortcut(key('Insert', { ctrlKey: true, shiftKey: true }), true)).toBe(false);
    expect(isCopyShortcut(key('v', { ctrlKey: true }), true)).toBe(false);
  });
});

describe('copyTerminalSelection', () => {
  it('writes the selection and then clears it', async () => {
    // 复制后清空选区，紧接着的 Ctrl+C 才能恢复成中断信号。
    const write = vi.fn().mockResolvedValue(true);
    const source = copySource();
    await expect(copyTerminalSelection(source, write)).resolves.toBe(true);
    expect(write).toHaveBeenCalledWith('selected');
    expect(source.clearSelection).toHaveBeenCalledOnce();
    expect(source.onUnavailable).not.toHaveBeenCalled();
  });

  it('does nothing at all when the selection is empty', async () => {
    const write = vi.fn().mockResolvedValue(true);
    const source = copySource({ selection: () => '' });
    await expect(copyTerminalSelection(source, write)).resolves.toBe(false);
    expect(write).not.toHaveBeenCalled();
    // 空选区不是错误，不该弹提示。
    expect(source.onUnavailable).not.toHaveBeenCalled();
  });

  it('reports failure and keeps the selection when the clipboard refuses', async () => {
    const source = copySource();
    await expect(copyTerminalSelection(source, () => Promise.resolve(false))).resolves.toBe(false);
    expect(source.onUnavailable).toHaveBeenCalledOnce();
    expect(source.clearSelection).not.toHaveBeenCalled();
  });
});

describe('attachTerminalClipboard', () => {
  it('copies a selection without reading or pasting clipboard content', async () => {
    const harness = contextMenuHarness();
    const target = clipboardTarget();
    const read = vi.fn().mockResolvedValue('old clipboard');
    const write = vi.fn().mockResolvedValue(true);
    attachTerminalClipboard(harness.container, target, read, write);

    harness.trigger();

    await vi.waitFor(() => expect(write).toHaveBeenCalledWith('selected'));
    expect(read).not.toHaveBeenCalled();
    expect(target.paste).not.toHaveBeenCalled();
    expect(target.clearSelection).toHaveBeenCalledOnce();
    expect(harness.event.preventDefault).toHaveBeenCalledOnce();
    expect(harness.event.stopPropagation).toHaveBeenCalledOnce();
  });

  it('pastes clipboard content only when the terminal has no selection', async () => {
    const harness = contextMenuHarness();
    const target = clipboardTarget({ hasSelection: () => false, selection: () => '' });
    const read = vi.fn().mockResolvedValue('paste me');
    const write = vi.fn().mockResolvedValue(true);
    attachTerminalClipboard(harness.container, target, read, write);

    harness.trigger();

    await vi.waitFor(() => expect(target.paste).toHaveBeenCalledWith('paste me'));
    expect(write).not.toHaveBeenCalled();
    expect(target.clearSelection).not.toHaveBeenCalled();
  });

  it('does not paste or report an error for an empty clipboard', async () => {
    const harness = contextMenuHarness();
    const target = clipboardTarget({ hasSelection: () => false, selection: () => '' });
    const read = vi.fn().mockResolvedValue('');
    attachTerminalClipboard(harness.container, target, read);

    harness.trigger();
    await vi.waitFor(() => expect(read).toHaveBeenCalledOnce());

    expect(target.paste).not.toHaveBeenCalled();
    expect(target.onUnavailable).not.toHaveBeenCalled();
  });

  it('reports clipboard read failures without attempting a paste', async () => {
    const harness = contextMenuHarness();
    const target = clipboardTarget({ hasSelection: () => false, selection: () => '' });
    attachTerminalClipboard(harness.container, target, () => Promise.reject(new Error('denied')));

    harness.trigger();

    await vi.waitFor(() => expect(target.onUnavailable).toHaveBeenCalledOnce());
    expect(target.paste).not.toHaveBeenCalled();
  });

  it('removes the same contextmenu listener when detached', () => {
    const harness = contextMenuHarness();
    const detach = attachTerminalClipboard(harness.container, clipboardTarget());

    detach();

    const attached = vi.mocked(harness.container.addEventListener).mock.calls[0]?.[1];
    expect(harness.container.removeEventListener).toHaveBeenCalledWith('contextmenu', attached);
  });
});

describe('createCopyKeyHandler', () => {
  it('swallows the key it handled so xterm does not send an interrupt', () => {
    const write = vi.fn().mockResolvedValue(true);
    const handler = createCopyKeyHandler(copySource(), write);
    const event = { ...key('c', { ctrlKey: true }), type: 'keydown' } as KeyboardEvent;
    expect(handler(event)).toBe(false);
    expect(write).toHaveBeenCalledWith('selected');
  });

  it('passes keyup and unrelated keys through untouched', () => {
    const write = vi.fn().mockResolvedValue(true);
    const handler = createCopyKeyHandler(copySource(), write);
    // keyup 也会走一遍回调，若不过滤就会复制两次。
    expect(handler({ ...key('c', { ctrlKey: true }), type: 'keyup' } as KeyboardEvent)).toBe(true);
    expect(handler({ ...key('a', { ctrlKey: true }), type: 'keydown' } as KeyboardEvent)).toBe(
      true
    );
    expect(write).not.toHaveBeenCalled();
  });

  it('does not intercept Ctrl+C while the source reports no selection', () => {
    // SSH 的口令提示期间正是这么做的：Ctrl+C 必须落到 onData 去取消连接。
    const write = vi.fn().mockResolvedValue(true);
    const handler = createCopyKeyHandler(copySource({ hasSelection: () => false }), write);
    expect(handler({ ...key('c', { ctrlKey: true }), type: 'keydown' } as KeyboardEvent)).toBe(
      true
    );
    expect(write).not.toHaveBeenCalled();
  });
});
