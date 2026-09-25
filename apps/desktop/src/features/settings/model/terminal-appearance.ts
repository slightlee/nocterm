import type { TerminalColorScheme } from '../types/settings-types';

export const TERMINAL_FONT_SIZE_MIN = 10;
export const TERMINAL_FONT_SIZE_MAX = 24;
export const DEFAULT_TERMINAL_FONT_SIZE = 13;

export type ResolvedTerminalTheme = Exclude<TerminalColorScheme, 'follow_app'>;

export type TerminalSchemeGroup = 'dark' | 'light';

export const terminalSchemeGroups: { id: TerminalSchemeGroup; label: string }[] = [
  { id: 'dark', label: '暗色' },
  { id: 'light', label: '亮色' },
];

type TerminalColorSchemeOption = {
  id: ResolvedTerminalTheme;
  label: string;
  description: string;
  group: TerminalSchemeGroup;
};

/** 元数据与渲染解耦；增加内置配色时只需追加配置与对应设计 Token。 */
export const terminalColorSchemes: TerminalColorSchemeOption[] = [
  {
    id: 'nocterm_light',
    group: 'light',
    label: '明亮',
    description: '明亮背景与高对比文字',
  },
  {
    id: 'nocterm_dark',
    group: 'dark',
    label: '暗夜',
    description: '深色背景与柔和前景',
  },
  {
    id: 'midnight',
    group: 'dark',
    label: '午夜蓝',
    description: '冷静深蓝与清晰高亮',
  },
  {
    id: 'graphite',
    group: 'dark',
    label: '石墨灰',
    description: '低饱和石墨灰',
  },
  {
    id: 'forest',
    group: 'dark',
    label: '森林绿',
    description: '沉静墨绿与自然色阶',
  },
  {
    id: 'amber',
    group: 'dark',
    label: '琥珀',
    description: '温暖琥珀复古风格',
  },
  {
    id: 'mobaxterm_vivid',
    group: 'dark',
    label: '鲜亮',
    description: 'MobaXterm 风格高饱和 16 色，粗体自动渲染为亮色变体',
  },
  {
    id: 'solarized_dark',
    group: 'dark',
    label: 'Solarized Dark',
    description: 'Solarized 的精密低对比色阶',
  },
  {
    id: 'dracula',
    group: 'dark',
    label: 'Dracula',
    description: '高辨识度紫色与鲜明强调色',
  },
  {
    id: 'monokai',
    group: 'dark',
    label: 'Monokai',
    description: '经典编辑器高饱和配色',
  },
  {
    id: 'nord',
    group: 'dark',
    label: 'Nord',
    description: '柔和克制的北欧冷色调',
  },
  {
    id: 'gruvbox_dark',
    group: 'dark',
    label: 'Gruvbox Dark',
    description: '暖色复古对比',
  },
  {
    id: 'tokyo_night',
    group: 'dark',
    label: 'Tokyo Night',
    description: '现代深蓝与霓虹高亮',
  },
  {
    id: 'one_dark',
    group: 'dark',
    label: 'One Dark',
    description: 'Atom One Dark 的平衡冷色调',
  },
  {
    id: 'catppuccin_mocha',
    group: 'dark',
    label: 'Catppuccin Mocha',
    description: 'Catppuccin 的柔和粉彩配色',
  },
  {
    id: 'material_ocean',
    group: 'dark',
    label: 'Material Ocean',
    description: '深海蓝绿配色',
  },
  {
    id: 'catppuccin_latte',
    group: 'light',
    label: 'Catppuccin Latte',
    description: '清爽亮色粉彩',
  },
  {
    id: 'catppuccin_frappe',
    group: 'dark',
    label: 'Catppuccin Frappé',
    description: '柔和蓝灰粉彩',
  },
  {
    id: 'catppuccin_macchiato',
    group: 'dark',
    label: 'Catppuccin Macchiato',
    description: '深蓝粉彩',
  },
  {
    id: 'rose_pine',
    group: 'dark',
    label: 'Rosé Pine',
    description: '柔和低对比紫调',
  },
  {
    id: 'rose_pine_dawn',
    group: 'light',
    label: 'Rosé Pine Dawn',
    description: '暖纸色亮色主题',
  },
  {
    id: 'rose_pine_moon',
    group: 'dark',
    label: 'Rosé Pine Moon',
    description: '更高对比月夜紫调',
  },
  {
    id: 'everforest_dark',
    group: 'dark',
    label: 'Everforest Dark',
    description: '柔和森林绿与暖沙前景',
  },
  {
    id: 'kanagawa',
    group: 'dark',
    label: 'Kanagawa',
    description: '葛饰北斋浮世绘配色',
  },
  {
    id: 'ayu_dark',
    group: 'dark',
    label: 'Ayu Dark',
    description: '深邃底色与明亮强调',
  },
  {
    id: 'ayu_light',
    group: 'light',
    label: 'Ayu Light',
    description: '干净亮色与柔和强调',
  },
  {
    id: 'oxocarbon_dark',
    group: 'dark',
    label: 'Oxocarbon Dark',
    description: 'IBM 碳黑与霓虹强调',
  },
  {
    id: 'one_half_light',
    group: 'light',
    label: 'One Half Light',
    description: '平衡亮色编辑器配色',
  },
  {
    id: 'github_light',
    group: 'light',
    label: 'GitHub Light',
    description: '官方亮色语法配色',
  },
  {
    id: 'synthwave_84',
    group: 'dark',
    label: "SynthWave '84",
    description: '复古霓虹夜色',
  },
];

export function resolveTerminalTheme(
  colorScheme: TerminalColorScheme,
  resolvedAppTheme: 'light' | 'dark'
): ResolvedTerminalTheme {
  if (colorScheme !== 'follow_app') return colorScheme;
  return resolvedAppTheme === 'light' ? 'nocterm_light' : 'nocterm_dark';
}
