import type {
  HighlightPreset,
  HighlightRoleId,
  HighlightSlotMap,
} from '../../settings/types/settings-types';

/**
 * 终端字体高亮的角色注册表：角色 → ANSI 色槽的单一来源。
 *
 * 设计要点：
 * - 用户配置存的是**槽位号**（0-15）而不是 RGB——槽位经当前主题调色板翻译，
 *   切换主题时所有角色自动重新着色，浅色/深色适配由主题层免费承担；
 * - **加粗属性随角色固定**（如目录 = 加粗蓝）：加粗会触发 xterm.js 的
 *   drawBoldTextInBrightColors（基色渲染为亮色变体），是文件类型层次的来源，
 *   第一版不开放，避免破坏 dircolors 语义；
 * - 角色标识与 Rust 侧 `HIGHLIGHT_ROLE_IDS` 契约绑定（见 terminal-appearance.test.ts）。
 */
export interface HighlightRoleMeta {
  id: HighlightRoleId;
  label: string;
  group: 'prompt' | 'files' | 'keywords';
  bold: boolean;
  /** 经典预设的槽位（即配置化之前的出厂常量）。 */
  defaultSlot: number;
}

export const HIGHLIGHT_ROLES: HighlightRoleMeta[] = [
  {
    id: 'prompt_user_host',
    label: '机器名 user@host',
    group: 'prompt',
    bold: true,
    defaultSlot: 10,
  },
  { id: 'prompt_path', label: '提示符路径', group: 'prompt', bold: true, defaultSlot: 12 },
  { id: 'permissions', label: '权限位', group: 'prompt', bold: false, defaultSlot: 6 },
  { id: 'date', label: '修改日期', group: 'prompt', bold: false, defaultSlot: 11 },
  { id: 'directory', label: '目录', group: 'files', bold: true, defaultSlot: 4 },
  { id: 'executable', label: '可执行文件', group: 'files', bold: true, defaultSlot: 2 },
  { id: 'symlink', label: '软链接', group: 'files', bold: true, defaultSlot: 5 },
  { id: 'device', label: '设备文件', group: 'files', bold: true, defaultSlot: 3 },
  { id: 'archive', label: '压缩包', group: 'files', bold: true, defaultSlot: 1 },
  { id: 'log', label: '日志文件', group: 'files', bold: false, defaultSlot: 3 },
  { id: 'config', label: '配置文件', group: 'files', bold: false, defaultSlot: 6 },
  { id: 'script', label: '脚本文件', group: 'files', bold: true, defaultSlot: 2 },
  { id: 'media', label: '媒体文件', group: 'files', bold: false, defaultSlot: 5 },
  { id: 'kw_error', label: '错误词', group: 'keywords', bold: true, defaultSlot: 9 },
  { id: 'kw_warn', label: '警告词', group: 'keywords', bold: true, defaultSlot: 11 },
  { id: 'kw_success', label: '成功词', group: 'keywords', bold: true, defaultSlot: 10 },
  { id: 'kw_info', label: '信息词', group: 'keywords', bold: true, defaultSlot: 12 },
  { id: 'ip', label: 'IP 地址', group: 'keywords', bold: true, defaultSlot: 14 },
];

const ROLE_META: Record<HighlightRoleId, HighlightRoleMeta> = Object.fromEntries(
  HIGHLIGHT_ROLES.map((role) => [role.id, role])
) as Record<HighlightRoleId, HighlightRoleMeta>;

export function getHighlightRole(id: HighlightRoleId): HighlightRoleMeta {
  return ROLE_META[id];
}

/** 三套内置预设的角色 → 槽位映射。`theme` 与配置化前的行为逐字节一致。 */
/**
 * 「缤纷」预设：多彩鲜明，与「经典」在 10 个角色上不同（含加粗基色→
 * 亮色的渲染映射后逐槽核对）：路径洋红、权限位灰、日期青、目录亮青、
 * 软链接亮蓝、设备亮洋红、配置深蓝、媒体亮黄、信息词亮青、IP 亮蓝。
 * 经典里撞色的组合拆开；0/7/15（黑/白）在浅色主题下不可见，一律避开。
 */
const VIVID_PRESET: Record<HighlightRoleId, number> = {
  prompt_user_host: 10,
  prompt_path: 5,
  permissions: 8,
  date: 6,
  directory: 14,
  executable: 2,
  symlink: 12,
  device: 13,
  archive: 9,
  log: 3,
  config: 4,
  script: 2,
  media: 11,
  kw_error: 9,
  kw_warn: 11,
  kw_success: 10,
  kw_info: 14,
  ip: 12,
};

/**
 * 「醒目」预设：高对比场景。提示符路径/信息词用槽 8（亮灰）——
 * 该槽在浅色调色板为深灰（白底可读）、深色调色板为浅灰（深底可读），
 * 是少数在明暗主题下都稳定的槽位，无需引入任何特殊槽概念。
 */
export const HIGHLIGHT_PRESETS: Record<HighlightPreset, Record<HighlightRoleId, number>> = {
  theme: Object.fromEntries(HIGHLIGHT_ROLES.map((role) => [role.id, role.defaultSlot])),
  mobaxterm: VIVID_PRESET,
  high_contrast: {
    prompt_user_host: 14,
    prompt_path: 8,
    permissions: 14,
    date: 11,
    directory: 12,
    executable: 10,
    symlink: 13,
    device: 11,
    archive: 9,
    log: 11,
    config: 14,
    script: 10,
    media: 13,
    kw_error: 9,
    kw_warn: 11,
    kw_success: 10,
    kw_info: 8,
    ip: 14,
  },
} as Record<HighlightPreset, Record<HighlightRoleId, number>>;

export const HIGHLIGHT_PRESET_LABELS: Record<HighlightPreset, string> = {
  theme: '经典',
  mobaxterm: '缤纷',
  high_contrast: '醒目',
};

export const HIGHLIGHT_SLOT_MIN = 0;
export const HIGHLIGHT_SLOT_MAX = 15;

export function isValidHighlightSlot(slot: number): boolean {
  return Number.isInteger(slot) && slot >= HIGHLIGHT_SLOT_MIN && slot <= HIGHLIGHT_SLOT_MAX;
}

/** 槽位 → ANSI 前景色参数：0-7 走基色（30-37），8-15 走亮色（90-97）。 */
export function slotToAnsi(slot: number): string {
  return String(slot <= 7 ? 30 + slot : 82 + slot);
}

/** 生成注入用的 SGR 序列；加粗角色依赖 drawBoldTextInBrightColors 映射亮色变体。 */
export function buildHighlightCode(slot: number, bold: boolean): string {
  return bold ? `\x1b[1;${slotToAnsi(slot)}m` : `\x1b[${slotToAnsi(slot)}m`;
}

/** 解析 Rust 侧同格式的紧凑覆盖串（`role=slot` 以 `,` 相连）；非法项整体忽略。 */
export function parseHighlightOverrides(value: string): HighlightSlotMap {
  if (!value) return {};
  const map: HighlightSlotMap = {};
  for (const part of value.split(',')) {
    const [role, slot] = part.split('=');
    const slotNumber = Number(slot);
    if (
      !role ||
      !(role in ROLE_META) ||
      !Number.isInteger(slotNumber) ||
      !isValidHighlightSlot(slotNumber)
    ) {
      continue;
    }
    map[role as HighlightRoleId] = slotNumber;
  }
  return map;
}

/** 编码为与 Rust `canonicalize_highlight_overrides` 一致的紧凑串（按角色排序）。 */
export function encodeHighlightOverrides(overrides: HighlightSlotMap): string {
  return Object.entries(overrides)
    .filter(([role, slot]) => role in ROLE_META && isValidHighlightSlot(slot))
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([role, slot]) => `${role}=${slot}`)
    .join(',');
}

export interface ResolvedHighlightRole {
  id: HighlightRoleId;
  slot: number;
  bold: boolean;
  code: string;
  /** 是否被用户覆盖（UI 用于展示微调状态与重置入口）。 */
  overridden: boolean;
}

/** 解析出全部 18 个角色的最终槽位与 SGR 序列：预设为基础，覆盖项叠加。 */
export function resolveHighlightRoles(
  preset: HighlightPreset,
  overrides: HighlightSlotMap
): Record<HighlightRoleId, ResolvedHighlightRole> {
  const base = HIGHLIGHT_PRESETS[preset] ?? HIGHLIGHT_PRESETS.theme;
  return Object.fromEntries(
    HIGHLIGHT_ROLES.map((role) => {
      const overridden = overrides[role.id];
      const slot =
        overridden !== undefined && isValidHighlightSlot(overridden) ? overridden : base[role.id];
      return [
        role.id,
        {
          id: role.id,
          slot,
          bold: role.bold,
          code: buildHighlightCode(slot, role.bold),
          overridden: overridden !== undefined,
        },
      ];
    })
  ) as Record<HighlightRoleId, ResolvedHighlightRole>;
}
