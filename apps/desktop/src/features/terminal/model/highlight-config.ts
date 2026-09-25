import { encodeHighlightOverrides, parseHighlightOverrides } from './highlight-roles';
import type { HighlightPreset, HighlightSlotMap } from '../../settings/types/settings-types';

/**
 * 终端高亮配置在 DOM 上的传递通道：SettingsProvider 把配置写到
 * `documentElement.dataset.terminalHighlight`，各终端实例在创建时读取一次，
 * 并由 observeTerminalAppearance 在该属性变化时推送更新——与主题/字号
 * 共用同一条"根设置属性 → MutationObserver"链路，不引入 React 重渲染。
 */
export interface TerminalHighlightConfig {
  preset: HighlightPreset;
  overrides: HighlightSlotMap;
}

const PRESETS: HighlightPreset[] = ['theme', 'mobaxterm', 'high_contrast'];

export function readTerminalHighlightConfig(): TerminalHighlightConfig {
  const raw = document.documentElement.dataset.terminalHighlight;
  if (!raw) return { preset: 'theme', overrides: {} };
  try {
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== 'object' || parsed === null) return { preset: 'theme', overrides: {} };
    const { p, o } = parsed as { p?: unknown; o?: unknown };
    const preset = PRESETS.includes(p as HighlightPreset) ? (p as HighlightPreset) : 'theme';
    return { preset, overrides: parseHighlightOverrides(typeof o === 'string' ? o : '') };
  } catch {
    return { preset: 'theme', overrides: {} };
  }
}

export function applyTerminalHighlightConfig(config: TerminalHighlightConfig): void {
  document.documentElement.dataset.terminalHighlight = JSON.stringify({
    p: config.preset,
    o: encodeHighlightOverrides(config.overrides),
  });
}
