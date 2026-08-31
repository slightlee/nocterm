import type { Terminal } from '@xterm/xterm';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { applyTerminalAppearance, readTerminalTheme } from './terminal-appearance';

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
});
