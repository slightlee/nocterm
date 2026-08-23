import { describe, expect, it, vi } from 'vitest';

import {
  type TerminalCopySource,
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
