import type { Terminal } from '@xterm/xterm';
import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  applyTerminalAppearance,
  observeTerminalAppearance,
  readTerminalTheme,
} from './terminal-appearance';

const palette = {
  '--term': '#101820',
  '--term-text': '#f0f4f8',
  '--term-blue': '#6699ff',
  '--term-selection': '#334455',
  '--term-black': '#101820',
  '--term-red': '#ff6677',
  '--term-green': '#66dd99',
  '--term-yellow': '#ffcc66',
  '--term-magenta': '#cc88ff',
  '--term-cyan': '#66ddee',
  '--term-white': '#e8edf2',
  '--term-bright-black': '#778899',
  '--term-bright-red': '#ff99a5',
  '--term-bright-green': '#99eebb',
  '--term-bright-yellow': '#ffe099',
  '--term-bright-blue': '#99bbff',
  '--term-bright-magenta': '#ddb5ff',
  '--term-bright-cyan': '#99edf4',
  '--term-bright-white': '#ffffff',
} as const;

afterEach(() => vi.unstubAllGlobals());

function stubAppearance(fontSize = '16') {
  vi.stubGlobal('getComputedStyle', () => ({
    getPropertyValue: (name: keyof typeof palette) => palette[name] ?? '',
  }));
  vi.stubGlobal('document', {
    documentElement: { dataset: { terminalFontSize: fontSize } },
  });
}

describe('terminal appearance runtime mapping', () => {
  it('maps the complete ANSI palette into the real xterm theme', () => {
    stubAppearance();

    expect(readTerminalTheme({} as HTMLElement)).toMatchObject({
      background: palette['--term'],
      foreground: palette['--term-text'],
      red: palette['--term-red'],
      green: palette['--term-green'],
      yellow: palette['--term-yellow'],
      blue: palette['--term-blue'],
      magenta: palette['--term-magenta'],
      cyan: palette['--term-cyan'],
      brightBlue: palette['--term-bright-blue'],
      brightCyan: palette['--term-bright-cyan'],
    });
  });

  it('applies theme and font size to an existing xterm instance', () => {
    stubAppearance('18');
    const terminal = { options: {} } as Terminal;

    applyTerminalAppearance(terminal, {} as HTMLElement);

    expect(terminal.options.fontSize).toBe(18);
    expect(terminal.options.theme).toMatchObject({
      background: palette['--term'],
      red: palette['--term-red'],
      brightWhite: palette['--term-bright-white'],
    });
  });

  it('re-applies appearance when the workspace broadcasts the theme commit event', () => {
    stubAppearance('20');
    // 主题作用域属性提交后由工作区广播事件；此处验证 observer 挂了事件监听
    // 且收到事件后重读外观（根属性通道已不再承载主题）。
    const listenerRef: { fn?: () => void } = {};
    vi.stubGlobal('window', {
      addEventListener: (_type: string, listener: () => void) => {
        listenerRef.fn = listener;
      },
      removeEventListener: () => {},
    });
    vi.stubGlobal(
      'MutationObserver',
      class {
        observe() {}

        disconnect() {}
      }
    );
    const terminal = { options: {} } as Terminal;
    const stop = observeTerminalAppearance(terminal, {} as HTMLElement, () => {});

    listenerRef.fn?.();
    stop();

    expect(terminal.options.fontSize).toBe(20);
    expect(terminal.options.theme).toMatchObject({ background: palette['--term'] });
  });

  it('pushes updated highlight config when the root attribute changes', () => {
    stubAppearance('16');
    // 字体颜色走根属性通道（SettingsProvider 写 data-terminal-highlight）：
    // 守住 attributeFilter 包含该属性 + 变化后把新配置推给终端回调。
    const observerCallbacks: MutationCallback[] = [];
    const observedOptions: MutationObserverInit[] = [];
    vi.stubGlobal(
      'MutationObserver',
      class {
        constructor(callback: MutationCallback) {
          observerCallbacks.push(callback);
        }

        observe(_target: Node, options: MutationObserverInit) {
          observedOptions.push(options);
        }

        disconnect() {}
      }
    );
    vi.stubGlobal('window', {
      addEventListener: () => {},
      removeEventListener: () => {},
    });
    const dataset = document.documentElement.dataset as Record<string, string>;
    const onHighlight = vi.fn();
    const stop = observeTerminalAppearance(
      { options: {} } as Terminal,
      {} as HTMLElement,
      () => {},
      onHighlight
    );

    expect(observedOptions[0]?.attributeFilter).toContain('data-terminal-highlight');
    dataset.terminalHighlight = JSON.stringify({ p: 'theme', o: 'directory=12,kw_info=14' });
    observerCallbacks[0]?.([], undefined as never);

    expect(onHighlight).toHaveBeenCalledWith({
      preset: 'theme',
      overrides: { directory: 12, kw_info: 14 },
    });
    stop();
  });
});
