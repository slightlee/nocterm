import type { TerminalColorScheme } from '../types/settings-types';

export const TERMINAL_FONT_SIZE_MIN = 10;
export const TERMINAL_FONT_SIZE_MAX = 24;
export const DEFAULT_TERMINAL_FONT_SIZE = 13;

export type ResolvedTerminalTheme = Exclude<TerminalColorScheme, 'follow_app'>;

type TerminalColorSchemeOption = {
  id: ResolvedTerminalTheme;
  label: string;
  description: string;
  previewClass:
    | 'terminalLight'
    | 'terminalDark'
    | 'midnight'
    | 'graphite'
    | 'forest'
    | 'amber'
    | 'solarizedDark'
    | 'dracula'
    | 'monokai'
    | 'nord'
    | 'gruvboxDark'
    | 'tokyoNight'
    | 'oneDark'
    | 'catppuccinMocha'
    | 'materialOcean';
};

/** 元数据与渲染解耦；增加内置配色时只需追加配置与对应设计 Token。 */
export const terminalColorSchemes: TerminalColorSchemeOption[] = [
  {
    id: 'nocterm_light',
    label: '明亮',
    description: '明亮背景与高对比文字',
    previewClass: 'terminalLight',
  },
  {
    id: 'nocterm_dark',
    label: '暗夜',
    description: '深色背景与柔和前景',
    previewClass: 'terminalDark',
  },
  {
    id: 'midnight',
    label: '午夜蓝',
    description: '冷静深蓝与清晰高亮',
    previewClass: 'midnight',
  },
  {
    id: 'graphite',
    label: '石墨灰',
    description: '低饱和石墨灰',
    previewClass: 'graphite',
  },
  {
    id: 'forest',
    label: '森林绿',
    description: '沉静墨绿与自然色阶',
    previewClass: 'forest',
  },
  {
    id: 'amber',
    label: '琥珀',
    description: '温暖琥珀复古风格',
    previewClass: 'amber',
  },
  {
    id: 'solarized_dark',
    label: '日光暗色',
    description: 'Solarized 的精密低对比色阶',
    previewClass: 'solarizedDark',
  },
  {
    id: 'dracula',
    label: '德古拉',
    description: '高辨识度紫色与鲜明强调色',
    previewClass: 'dracula',
  },
  {
    id: 'monokai',
    label: '莫诺凯',
    description: '经典编辑器高饱和配色',
    previewClass: 'monokai',
  },
  {
    id: 'nord',
    label: '北境',
    description: '柔和克制的北欧冷色调',
    previewClass: 'nord',
  },
  {
    id: 'gruvbox_dark',
    label: '复古暗色',
    description: 'Gruvbox 的暖色复古对比',
    previewClass: 'gruvboxDark',
  },
  {
    id: 'tokyo_night',
    label: '霓虹黑',
    description: '现代深蓝与霓虹高亮',
    previewClass: 'tokyoNight',
  },
  {
    id: 'one_dark',
    label: '原子暗色',
    description: 'Atom One Dark 的平衡冷色调',
    previewClass: 'oneDark',
  },
  {
    id: 'catppuccin_mocha',
    label: '摩卡',
    description: 'Catppuccin 的柔和粉彩配色',
    previewClass: 'catppuccinMocha',
  },
  {
    id: 'material_ocean',
    label: '材质海洋',
    description: 'Material Ocean 的深海蓝绿配色',
    previewClass: 'materialOcean',
  },
];

export function resolveTerminalTheme(
  colorScheme: TerminalColorScheme,
  resolvedAppTheme: 'light' | 'dark'
): ResolvedTerminalTheme {
  if (colorScheme !== 'follow_app') return colorScheme;
  return resolvedAppTheme === 'light' ? 'nocterm_light' : 'nocterm_dark';
}
