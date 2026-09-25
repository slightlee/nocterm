import type { HighlightRoleId } from '../types/settings-types';

/**
 * 对比度守卫：把"用户只能在安全色里选"落到实处。
 *
 * 角色颜色存的是 ANSI 色槽（0-15），最终 RGB 由当前主题调色板翻译；
 * 本模块在设置页实时读取该调色板，对每个槽位计算与终端背景的
 * WCAG 对比度——不达标的槽位在 UI 上禁选，从源头避免"选出来的颜色看不清"。
 * 计算是纯数学（微秒级、按需触发），30 套主题与未来新主题零维护成本。
 */

/** WCAG 2.x 相对亮度。 */
export function relativeLuminance(hex: string): number {
  const normalized = normalizeHex(hex);
  if (!normalized) return 0;
  const channels = [0, 2, 4].map((offset) => {
    const raw = Number.parseInt(normalized.slice(offset + 1, offset + 3), 16) / 255;
    return raw <= 0.03928 ? raw / 12.92 : ((raw + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2];
}

/** WCAG 对比度（1 ~ 21）。 */
export function contrastRatio(foreground: string, background: string): number {
  const l1 = relativeLuminance(foreground);
  const l2 = relativeLuminance(background);
  const [lighter, darker] = l1 >= l2 ? [l1, l2] : [l2, l1];
  return (lighter + 0.05) / (darker + 0.05);
}

/**
 * 设置页与真实终端共用的最低对比度门槛。
 * 取 WCAG 1.4.11 非文本/UI 组件的 3:1 而非正文文本的 4.5:1——
 * 角色颜色用于关键词、路径等短词与辅助信息，4.5 档会误伤低饱和
 * 主题（如玫瑰松·晨、拿铁）的大量原生槽位，导致色板大面积禁选。
 */
export const MIN_HIGHLIGHT_CONTRAST = 3;

/**
 * 归一化 hex 颜色为 `#rrggbb`。主题调色板存在三位简写（如 `#fff`），
 * 任何按 6 位校验的逻辑都必须先过这里，否则整块调色板会被误判非法。
 */
export function normalizeHex(hex: string): string {
  const raw = hex.replace('#', '').trim();
  if (/^[0-9a-fA-F]{6}$/.test(raw)) return `#${raw.toLowerCase()}`;
  if (/^[0-9a-fA-F]{3}$/.test(raw)) {
    return `#${raw
      .split('')
      .map((c) => c + c)
      .join('')
      .toLowerCase()}`;
  }
  return '';
}

/**
 * 槽位 → 主题 CSS 变量后缀（tokens.css 单源命名），0-15 对应真实 ANSI 色槽。
 */
const SLOT_VARIABLES = [
  'black',
  'red',
  'green',
  'yellow',
  'blue',
  'magenta',
  'cyan',
  'white',
  'bright-black',
  'bright-red',
  'bright-green',
  'bright-yellow',
  'bright-blue',
  'bright-magenta',
  'bright-cyan',
  'bright-white',
] as const;

export interface HighlightPaletteSnapshot {
  /** 终端背景（`--term`），对比度的基准面。 */
  background: string;
  /** 16 个 ANSI 色槽的当前主题 RGB（索引即槽位）。 */
  slots: string[];
}

const VARIABLE_NAMES = ['--term', ...SLOT_VARIABLES.map((suffix) => `--term-${suffix}`)] as const;

/**
 * 读取当前生效的调色板快照。scope 必须是能命中主题选择器的元素——
 * 16 色变量只挂在 `[data-terminal-theme='…'] .nocterm-terminal-surface` 作用域，
 * 终端工作区容器与设置页的哨兵元素（见 HighlightColorSettings）均可；
 * 从文档根读取只能拿到应用级兜底色，不是真实终端调色板。
 */
export function readHighlightPalette(scope: Element): HighlightPaletteSnapshot {
  const styles = getComputedStyle(scope);
  const values = VARIABLE_NAMES.map((name) => styles.getPropertyValue(name).trim());
  const [background, ...slots] = values;
  return { background, slots };
}

/** 每个槽位是否达到对比度门槛（索引即槽位）。 */
export function evaluateSlotContrast(palette: HighlightPaletteSnapshot): boolean[] {
  const background = normalizeHex(palette.background);
  if (!background) return palette.slots.map(() => false);
  return palette.slots.map((hex) => {
    const normalized = normalizeHex(hex);
    return normalized !== '' && contrastRatio(normalized, background) >= MIN_HIGHLIGHT_CONTRAST;
  });
}

/** 同槽位冲突检测：多个角色指向同一槽位会让层次混淆，UI 只警示不静默改写。 */
export function findSlotConflicts(
  slots: Record<HighlightRoleId, number>
): Record<number, HighlightRoleId[]> {
  const bySlot = new Map<number, HighlightRoleId[]>();
  for (const [role, slot] of Object.entries(slots)) {
    const list = bySlot.get(slot) ?? [];
    list.push(role as HighlightRoleId);
    bySlot.set(slot, list);
  }
  return Object.fromEntries([...bySlot.entries()].filter(([, roles]) => roles.length > 1));
}
