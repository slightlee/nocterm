import { describe, expect, it } from 'vitest';

import {
  resolveTerminalTheme,
  terminalColorSchemes,
  terminalSchemeGroups,
} from './terminal-appearance';

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
      'mobaxterm_vivid',
      'catppuccin_latte',
      'catppuccin_frappe',
      'catppuccin_macchiato',
      'rose_pine',
      'rose_pine_dawn',
      'rose_pine_moon',
      'everforest_dark',
      'kanagawa',
      'ayu_dark',
      'ayu_light',
      'oxocarbon_dark',
      'one_half_light',
      'github_light',
      'synthwave_84',
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
      '鲜亮',
      '拿铁',
      '法布奇诺',
      '玛奇朵',
      '玫瑰松',
      '玫瑰松·晨',
      '玫瑰松·月',
      '暮林',
      '神奈川',
      'Ayu 暗',
      'Ayu 亮',
      '氧碳黑',
      '一半亮',
      'GitHub 亮',
      '合成波',
    ]);
  });

  it('groups every scheme into exactly one curated group', () => {
    const groupIds = new Set(terminalSchemeGroups.map((group) => group.id));
    for (const scheme of terminalColorSchemes) {
      expect(groupIds.has(scheme.group), `${scheme.id} group`).toBe(true);
    }
    // 每个分组都至少有一个方案，避免出现空分组标题。
    for (const group of terminalSchemeGroups) {
      expect(
        terminalColorSchemes.some((scheme) => scheme.group === group.id),
        `${group.id} non-empty`
      ).toBe(true);
    }
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
    expect(resolveTerminalTheme('mobaxterm_vivid', 'light')).toBe('mobaxterm_vivid');
    expect(resolveTerminalTheme('rose_pine_dawn', 'dark')).toBe('rose_pine_dawn');
    expect(resolveTerminalTheme('github_light', 'dark')).toBe('github_light');
  });
});
