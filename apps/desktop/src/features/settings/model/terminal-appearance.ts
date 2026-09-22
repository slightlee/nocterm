import type { TerminalColorScheme } from '../types/settings-types';

export const TERMINAL_FONT_SIZE_MIN = 10;
export const TERMINAL_FONT_SIZE_MAX = 24;
export const DEFAULT_TERMINAL_FONT_SIZE = 13;

export type ResolvedTerminalTheme = Exclude<TerminalColorScheme, 'follow_app'>;

export type TerminalSchemeGroup = 'featured' | 'dark' | 'light';

export const terminalSchemeGroups: { id: TerminalSchemeGroup; label: string }[] = [
  { id: 'featured', label: '高饱和与特色' },
  { id: 'dark', label: '暗色' },
  { id: 'light', label: '亮色' },
];

type TerminalColorSchemeOption = {
  id: ResolvedTerminalTheme;
  label: string;
  description: string;
  group: TerminalSchemeGroup;
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
    | 'materialOcean'
    | 'mobaxtermVivid'
    | 'catppuccinLatte'
    | 'catppuccinFrappe'
    | 'catppuccinMacchiato'
    | 'rosePine'
    | 'rosePineDawn'
    | 'rosePineMoon'
    | 'everforestDark'
    | 'kanagawa'
    | 'ayuDark'
    | 'ayuLight'
    | 'oxocarbonDark'
    | 'oneHalfLight'
    | 'githubLight'
    | 'synthwave84';
};

/** 元数据与渲染解耦；增加内置配色时只需追加配置与对应设计 Token。 */
export const terminalColorSchemes: TerminalColorSchemeOption[] = [
  {
    id: 'nocterm_light',
    group: 'light',
    label: '明亮',
    description: '明亮背景与高对比文字',
    previewClass: 'terminalLight',
  },
  {
    id: 'nocterm_dark',
    group: 'dark',
    label: '暗夜',
    description: '深色背景与柔和前景',
    previewClass: 'terminalDark',
  },
  {
    id: 'midnight',
    group: 'dark',
    label: '午夜蓝',
    description: '冷静深蓝与清晰高亮',
    previewClass: 'midnight',
  },
  {
    id: 'graphite',
    group: 'dark',
    label: '石墨灰',
    description: '低饱和石墨灰',
    previewClass: 'graphite',
  },
  {
    id: 'forest',
    group: 'dark',
    label: '森林绿',
    description: '沉静墨绿与自然色阶',
    previewClass: 'forest',
  },
  {
    id: 'amber',
    group: 'dark',
    label: '琥珀',
    description: '温暖琥珀复古风格',
    previewClass: 'amber',
  },
  {
    id: 'solarized_dark',
    group: 'dark',
    label: '日光暗色',
    description: 'Solarized 的精密低对比色阶',
    previewClass: 'solarizedDark',
  },
  {
    id: 'dracula',
    group: 'dark',
    label: '德古拉',
    description: '高辨识度紫色与鲜明强调色',
    previewClass: 'dracula',
  },
  {
    id: 'monokai',
    group: 'dark',
    label: '莫诺凯',
    description: '经典编辑器高饱和配色',
    previewClass: 'monokai',
  },
  {
    id: 'nord',
    group: 'dark',
    label: '北境',
    description: '柔和克制的北欧冷色调',
    previewClass: 'nord',
  },
  {
    id: 'gruvbox_dark',
    group: 'dark',
    label: '复古暗色',
    description: 'Gruvbox 的暖色复古对比',
    previewClass: 'gruvboxDark',
  },
  {
    id: 'tokyo_night',
    group: 'dark',
    label: '霓虹黑',
    description: '现代深蓝与霓虹高亮',
    previewClass: 'tokyoNight',
  },
  {
    id: 'one_dark',
    group: 'dark',
    label: '原子暗色',
    description: 'Atom One Dark 的平衡冷色调',
    previewClass: 'oneDark',
  },
  {
    id: 'catppuccin_mocha',
    group: 'dark',
    label: '摩卡',
    description: 'Catppuccin 的柔和粉彩配色',
    previewClass: 'catppuccinMocha',
  },
  {
    id: 'material_ocean',
    group: 'dark',
    label: '材质海洋',
    description: 'Material Ocean 的深海蓝绿配色',
    previewClass: 'materialOcean',
  },
  {
    id: 'mobaxterm_vivid',
    label: '鲜亮',
    description: 'MobaXterm 风格高饱和 16 色，粗体自动渲染为亮色变体',
    group: 'featured',
    previewClass: 'mobaxtermVivid',
  },
  {
    id: 'catppuccin_latte',
    label: '拿铁',
    description: 'Catppuccin Latte 的清爽亮色粉彩',
    group: 'light',
    previewClass: 'catppuccinLatte',
  },
  {
    id: 'catppuccin_frappe',
    label: '法布奇诺',
    description: 'Catppuccin Frappé 的柔和蓝灰粉彩',
    group: 'dark',
    previewClass: 'catppuccinFrappe',
  },
  {
    id: 'catppuccin_macchiato',
    label: '玛奇朵',
    description: 'Catppuccin Macchiato 的深蓝粉彩',
    group: 'dark',
    previewClass: 'catppuccinMacchiato',
  },
  {
    id: 'rose_pine',
    label: '玫瑰松',
    description: 'Rosé Pine 的柔和低对比紫调',
    group: 'dark',
    previewClass: 'rosePine',
  },
  {
    id: 'rose_pine_dawn',
    label: '玫瑰松·晨',
    description: 'Rosé Pine Dawn 的暖纸色亮色主题',
    group: 'light',
    previewClass: 'rosePineDawn',
  },
  {
    id: 'rose_pine_moon',
    label: '玫瑰松·月',
    description: 'Rosé Pine Moon 的更高对比月夜紫调',
    group: 'dark',
    previewClass: 'rosePineMoon',
  },
  {
    id: 'everforest_dark',
    label: '暮林',
    description: 'Everforest 的柔和森林绿与暖沙前景',
    group: 'dark',
    previewClass: 'everforestDark',
  },
  {
    id: 'kanagawa',
    label: '神奈川',
    description: 'Kanagawa 的葛饰北斋浮世绘配色',
    group: 'dark',
    previewClass: 'kanagawa',
  },
  {
    id: 'ayu_dark',
    label: 'Ayu 暗',
    description: 'Ayu Dark 的深邃底色与明亮强调',
    group: 'dark',
    previewClass: 'ayuDark',
  },
  {
    id: 'ayu_light',
    label: 'Ayu 亮',
    description: 'Ayu Light 的干净亮色与柔和强调',
    group: 'light',
    previewClass: 'ayuLight',
  },
  {
    id: 'oxocarbon_dark',
    label: '氧碳黑',
    description: 'Oxocarbon 的 IBM 碳黑与霓虹强调',
    group: 'dark',
    previewClass: 'oxocarbonDark',
  },
  {
    id: 'one_half_light',
    label: '一半亮',
    description: 'One Half Light 的平衡亮色编辑器配色',
    group: 'light',
    previewClass: 'oneHalfLight',
  },
  {
    id: 'github_light',
    label: 'GitHub 亮',
    description: 'GitHub Light 的官方亮色语法配色',
    group: 'light',
    previewClass: 'githubLight',
  },
  {
    id: 'synthwave_84',
    label: '合成波',
    description: "Synthwave '84 的复古霓虹夜色",
    group: 'featured',
    previewClass: 'synthwave84',
  },
];

export function resolveTerminalTheme(
  colorScheme: TerminalColorScheme,
  resolvedAppTheme: 'light' | 'dark'
): ResolvedTerminalTheme {
  if (colorScheme !== 'follow_app') return colorScheme;
  return resolvedAppTheme === 'light' ? 'nocterm_light' : 'nocterm_dark';
}
