export type AppTheme = 'system' | 'light' | 'dark';

export interface AppThemeResponse {
  value: AppTheme;
}

export type TerminalColorScheme =
  | 'follow_app'
  | 'nocterm_light'
  | 'nocterm_dark'
  | 'midnight'
  | 'graphite'
  | 'forest'
  | 'amber'
  | 'solarized_dark'
  | 'dracula'
  | 'monokai'
  | 'nord'
  | 'gruvbox_dark'
  | 'tokyo_night'
  | 'one_dark'
  | 'catppuccin_mocha'
  | 'material_ocean'
  | 'mobaxterm_vivid'
  | 'catppuccin_latte'
  | 'catppuccin_frappe'
  | 'catppuccin_macchiato'
  | 'rose_pine'
  | 'rose_pine_dawn'
  | 'rose_pine_moon'
  | 'everforest_dark'
  | 'kanagawa'
  | 'ayu_dark'
  | 'ayu_light'
  | 'oxocarbon_dark'
  | 'one_half_light'
  | 'github_light'
  | 'synthwave_84';

export interface TerminalAppearance {
  fontSize: number;
  colorScheme: TerminalColorScheme;
}

export interface SettingsError {
  code: string;
  message: string;
  retryable: boolean;
}
