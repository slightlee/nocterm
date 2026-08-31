import { describe, expect, it } from 'vitest';

import { resolveTerminalTheme, terminalColorSchemes } from './terminal-appearance';

describe('resolveTerminalTheme', () => {
  it('offers the complete curated terminal palette set', () => {
    expect(terminalColorSchemes.map((scheme) => scheme.id)).toEqual([
      'nocterm_light',
      'nocterm_dark',
      'midnight',
      'graphite',
      'forest',
      'amber',
      'solarized_dark',
      'dracula',
      'monokai',
      'nord',
      'gruvbox_dark',
      'tokyo_night',
      'one_dark',
      'catppuccin_mocha',
      'material_ocean',
    ]);
    expect(terminalColorSchemes.map((scheme) => scheme.label)).toEqual([
      '明亮',
      '暗夜',
      '午夜蓝',
      '石墨灰',
      '森林绿',
      '琥珀',
      '日光暗色',
      '德古拉',
      '莫诺凯',
      '北境',
      '复古暗色',
      '霓虹黑',
      '原子暗色',
      '摩卡',
      '材质海洋',
    ]);
  });

  it('resolves the follow-app scheme at runtime', () => {
    expect(resolveTerminalTheme('follow_app', 'light')).toBe('nocterm_light');
    expect(resolveTerminalTheme('follow_app', 'dark')).toBe('nocterm_dark');
  });

  it('keeps explicit terminal schemes independent from the app', () => {
    expect(resolveTerminalTheme('nocterm_light', 'dark')).toBe('nocterm_light');
    expect(resolveTerminalTheme('nocterm_dark', 'light')).toBe('nocterm_dark');
    expect(resolveTerminalTheme('midnight', 'light')).toBe('midnight');
    expect(resolveTerminalTheme('graphite', 'light')).toBe('graphite');
    expect(resolveTerminalTheme('forest', 'dark')).toBe('forest');
    expect(resolveTerminalTheme('amber', 'dark')).toBe('amber');
    expect(resolveTerminalTheme('solarized_dark', 'light')).toBe('solarized_dark');
    expect(resolveTerminalTheme('dracula', 'light')).toBe('dracula');
    expect(resolveTerminalTheme('monokai', 'dark')).toBe('monokai');
    expect(resolveTerminalTheme('nord', 'dark')).toBe('nord');
    expect(resolveTerminalTheme('gruvbox_dark', 'light')).toBe('gruvbox_dark');
    expect(resolveTerminalTheme('tokyo_night', 'light')).toBe('tokyo_night');
    expect(resolveTerminalTheme('one_dark', 'light')).toBe('one_dark');
    expect(resolveTerminalTheme('catppuccin_mocha', 'dark')).toBe('catppuccin_mocha');
    expect(resolveTerminalTheme('material_ocean', 'light')).toBe('material_ocean');
  });
});
