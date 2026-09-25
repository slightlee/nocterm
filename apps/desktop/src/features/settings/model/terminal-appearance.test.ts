import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { describe, expect, it } from 'vitest';

import type { HighlightPreset } from '../types/settings-types';
import {
  resolveTerminalTheme,
  terminalColorSchemes,
  terminalSchemeGroups,
} from './terminal-appearance';
import { HIGHLIGHT_ROLES } from '../../terminal/model/highlight-roles';

describe('resolveTerminalTheme', () => {
  it('offers the complete curated terminal palette set', () => {
    expect(terminalColorSchemes.map((scheme) => scheme.id)).toEqual([
      'nocterm_light',
      'nocterm_dark',
      'midnight',
      'graphite',
      'forest',
      'amber',
      'mobaxterm_vivid',
      'solarized_dark',
      'dracula',
      'monokai',
      'nord',
      'gruvbox_dark',
      'tokyo_night',
      'one_dark',
      'catppuccin_mocha',
      'material_ocean',
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
      '鲜亮',
      'Solarized Dark',
      'Dracula',
      'Monokai',
      'Nord',
      'Gruvbox Dark',
      'Tokyo Night',
      'One Dark',
      'Catppuccin Mocha',
      'Material Ocean',
      'Catppuccin Latte',
      'Catppuccin Frappé',
      'Catppuccin Macchiato',
      'Rosé Pine',
      'Rosé Pine Dawn',
      'Rosé Pine Moon',
      'Everforest Dark',
      'Kanagawa',
      'Ayu Dark',
      'Ayu Light',
      'Oxocarbon Dark',
      'One Half Light',
      'GitHub Light',
      "SynthWave '84",
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

/**
 * 跨边界契约：终端配色由前端注册表与 Rust 设置校验（TerminalColorScheme::parse）共同
 * 约束，任何一侧单方面扩充都会在保存设置时被另一侧拒绝。本测试锁定两侧清单一致，
 * 新增配色时必须同时更新 crates/nocterm-domain/src/settings.rs。
 */
describe('terminal scheme registry (cross-boundary contract)', () => {
  it('registers every scheme id in the Rust settings validator', () => {
    const rustSource = readFileSync(
      fileURLToPath(
        new URL('../../../../../../crates/nocterm-domain/src/settings.rs', import.meta.url)
      ),
      'utf8'
    );
    const missing = terminalColorSchemes
      .map((scheme) => scheme.id)
      .filter((id) => !rustSource.includes(`"${id}" => Ok(Self::`));
    expect(missing).toEqual([]);
  });
});

/**
 * 跨边界契约：字体颜色角色与预设同样受两侧约束——前端 highlight-roles.ts 的
 * 角色清单、预设清单必须与 Rust `HIGHLIGHT_ROLE_IDS` / `HighlightPreset::parse`
 * 完全一致，否则覆盖项会在保存或读取时被另一侧拒绝。
 */
describe('highlight role registry (cross-boundary contract)', () => {
  const rustSource = readFileSync(
    fileURLToPath(
      new URL('../../../../../../crates/nocterm-domain/src/settings.rs', import.meta.url)
    ),
    'utf8'
  );

  it('registers every highlight role id in the Rust validator', () => {
    const roleIds = HIGHLIGHT_ROLES.map((role) => role.id);
    for (const id of roleIds) {
      expect(rustSource.includes(`"${id}"`), `${id} in Rust HIGHLIGHT_ROLE_IDS`).toBe(true);
    }
    // Rust 侧声明的角色数量与前端一致，防止 Rust 单方面追加后前端漏同步。
    const rustRoleList = rustSource.match(/HIGHLIGHT_ROLE_IDS: \[&str; (\d+)\]/);
    expect(rustRoleList?.[1]).toBe(String(roleIds.length));
  });

  it('registers every highlight preset id in the Rust parser', () => {
    const presetIds: HighlightPreset[] = ['theme', 'mobaxterm', 'high_contrast'];
    const missing = presetIds.filter((id) => !rustSource.includes(`"${id}" => Ok(Self::`));
    expect(missing).toEqual([]);
  });
});
