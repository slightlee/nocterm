/**
 * 终端字体高亮契约：settings 负责配置、terminal 负责渲染，两侧共用同一领域，
 * 类型统一下沉到 shared，避免 Feature 之间互相引用内部文件形成循环。
 */

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
