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

/** 字体高亮预设：theme = 当前主题的出厂角色配色。 */
export type HighlightPreset = 'theme' | 'mobaxterm' | 'high_contrast';

/**
 * 可自定义颜色的语义角色。与 Rust 侧 HIGHLIGHT_ROLE_IDS 契约绑定，
 * 新增角色必须同步 settings.rs 并扩展契约测试。
 */
export type HighlightRoleId =
  | 'prompt_user_host'
  | 'prompt_path'
  | 'permissions'
  | 'date'
  | 'directory'
  | 'executable'
  | 'symlink'
  | 'device'
  | 'archive'
  | 'log'
  | 'config'
  | 'script'
  | 'media'
  | 'kw_error'
  | 'kw_warn'
  | 'kw_success'
  | 'kw_info'
  | 'ip';

/** 角色 → ANSI 色槽（0-15）；最终 RGB 由当前主题调色板翻译。 */
export type HighlightSlotMap = Partial<Record<HighlightRoleId, number>>;

export interface TerminalAppearance {
  fontSize: number;
  colorScheme: TerminalColorScheme;
  highlightPreset: HighlightPreset;
  /** 紧凑序列化（`role=slot` 以 `,` 相连），与 Rust 侧格式一致；空串表示无覆盖。 */
  highlightOverrides: string;
}

export interface SettingsError {
  code: string;
  message: string;
  retryable: boolean;
}
