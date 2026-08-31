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
  | 'material_ocean';

export interface TerminalAppearance {
  fontSize: number;
  colorScheme: TerminalColorScheme;
}

export interface SettingsError {
  code: string;
  message: string;
  retryable: boolean;
}
