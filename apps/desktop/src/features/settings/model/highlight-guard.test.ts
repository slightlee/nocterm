import { describe, expect, it } from 'vitest';

import {
  contrastRatio,
  evaluateSlotContrast,
  findSlotConflicts,
  normalizeHex,
  relativeLuminance,
} from './highlight-guard';

describe('highlight guard', () => {
  it('computes WCAG contrast ratios for known pairs', () => {
    expect(contrastRatio('#000000', '#ffffff')).toBeCloseTo(21, 1);
    expect(contrastRatio('#ffffff', '#000000')).toBeCloseTo(21, 1);
    // 相同颜色对比度为 1。
    expect(contrastRatio('#1e1e2e', '#1e1e2e')).toBeCloseTo(1, 5);
  });

  it('expands 3-digit hex shorthand used by theme palettes', () => {
    // 主题里存在 #fff 这类短写，必须归一化而不是判非法。
    expect(normalizeHex('#fff')).toBe('#ffffff');
    expect(normalizeHex('#FFF')).toBe('#ffffff');
    expect(normalizeHex('#abc')).toBe('#aabbcc');
    expect(normalizeHex('not-a-color')).toBe('');
    // 简写与展开形式对比度一致。
    expect(contrastRatio('#000', '#fff')).toBeCloseTo(21, 1);
  });

  it('keeps slots selectable on a shorthand-hex background', () => {
    // 回归：浅色主题背景是 #fff，旧版按 6 位校验会把全部槽位误判禁选。
    const palette = {
      background: '#fff',
      slots: Array<string>(16).fill('#24292f'),
    };
    expect(evaluateSlotContrast(palette).every((v) => v)).toBe(true);
  });

  it('treats invalid hex as zero luminance', () => {
    expect(relativeLuminance('not-a-color')).toBe(0);
    // 亮度 0（黑）对白底的对比度是 21。
    expect(contrastRatio('nope', '#ffffff')).toBeCloseTo(21, 5);
  });

  it('rejects slots below the contrast threshold', () => {
    const palette = {
      background: '#ffffff',
      // 白底上白色槽位不可读，黑色槽位可读。
      slots: Array<string>(16).fill('#ffffff'),
    };
    const verdicts = evaluateSlotContrast(palette);
    expect(verdicts.every((v) => !v)).toBe(true);
    const dark = evaluateSlotContrast({ ...palette, slots: Array<string>(16).fill('#000000') });
    expect(dark.every((v) => v)).toBe(true);
  });

  it('flags roles that share the same slot', () => {
    const conflicts = findSlotConflicts({
      prompt_user_host: 10,
      prompt_path: 12,
      permissions: 6,
      date: 11,
      directory: 4,
      executable: 2,
      symlink: 5,
      device: 3,
      archive: 1,
      log: 3,
      config: 6,
      script: 2,
      media: 5,
      kw_error: 9,
      kw_warn: 11,
      kw_success: 10,
      kw_info: 12,
      ip: 14,
    });
    // 默认槽位表里 3/5/6/2/10/11/12 均有两个角色共用。
    expect(conflicts[3]).toEqual(['device', 'log']);
    expect(conflicts[10]).toEqual(['prompt_user_host', 'kw_success']);
    expect(conflicts[14]).toBeUndefined();
  });
});
