import { useMemo, useState } from 'react';

import type { TerminalAppearance, HighlightPreset, HighlightRoleId } from '../types/settings-types';
import {
  HIGHLIGHT_PRESETS,
  HIGHLIGHT_PRESET_LABELS,
  HIGHLIGHT_ROLES,
  encodeHighlightOverrides,
  getHighlightRole,
  parseHighlightOverrides,
  resolveHighlightRoles,
} from '../../terminal/model/highlight-roles';
import {
  contrastRatio,
  evaluateSlotContrast,
  findSlotConflicts,
  MIN_HIGHLIGHT_CONTRAST,
  type HighlightPaletteSnapshot,
} from '../model/highlight-guard';
import styles from './HighlightColorSettings.module.css';

const PRESET_IDS: HighlightPreset[] = ['theme', 'mobaxterm', 'high_contrast'];

/**
 * 色板显示顺序，与存储槽号解耦（存储的仍是 ANSI 槽位编号，重排不影响已保存配置）：
 * 中性色（黑、灰）开场 → 彩色按红黄绿青蓝紫过渡 → 白收尾，
 * 每对基色→亮色由深到浅，避免 ANSI 原序「左黑右黑、中间夹白」的凌乱观感。
 * 「跟随主题文字色」不进常规色板——浅色主题下它与黑槽同值，常驻只会造成困惑；
 * 仅当某角色当前正用它（如醒目预设的路径、信息词）时，才在该行行内单独显示。
 */
const SLOT_DISPLAY_ORDER = [0, 8, 1, 9, 3, 11, 2, 10, 6, 14, 4, 12, 5, 13, 7, 15];

const GROUP_LABELS: Record<'prompt' | 'files' | 'keywords', string> = {
  prompt: '提示符与元数据',
  files: '文件类型',
  keywords: '关键词',
};

interface HighlightColorSettingsProps {
  appearance: TerminalAppearance;
  palette: HighlightPaletteSnapshot;
  disabled: boolean;
  onChange: (next: TerminalAppearance) => void;
}

/**
 * 终端字体颜色自定义：预设方案一键切换 + 逐角色微调。
 * 角色存 ANSI 色槽，色块颜色直接取当前主题调色板翻译结果——
 * 所见即终端所得；不达对比度门槛的槽位禁选（守卫，见 highlight-guard.ts）。
 * 调色板由外层（AppearanceSettingsPage 经 useTerminalPalette）传入，全页共用一份。
 */
export function HighlightColorSettings({
  appearance,
  palette,
  disabled,
  onChange,
}: HighlightColorSettingsProps) {
  const [customOpen, setCustomOpen] = useState(() => appearance.highlightOverrides !== '');

  const slotEnabled = useMemo(() => evaluateSlotContrast(palette), [palette]);
  const overrides = useMemo(
    () => parseHighlightOverrides(appearance.highlightOverrides),
    [appearance.highlightOverrides]
  );
  const resolved = useMemo(
    () => resolveHighlightRoles(appearance.highlightPreset, overrides),
    [appearance.highlightPreset, overrides]
  );
  const conflicts = useMemo(
    () =>
      findSlotConflicts(
        Object.fromEntries(
          HIGHLIGHT_ROLES.map((role) => [role.id, resolved[role.id].slot])
        ) as Record<HighlightRoleId, number>
      ),
    [resolved]
  );

  const applyPreset = (preset: HighlightPreset) => {
    // 预设与逐角色微调互斥：套用预设即退出微调模式，选中态只落在一边。
    setCustomOpen(false);
    onChange({ ...appearance, highlightPreset: preset, highlightOverrides: '' });
  };

  const setRoleSlot = (role: HighlightRoleId, slot: number) => {
    const base = HIGHLIGHT_PRESETS[appearance.highlightPreset][role];
    const next = { ...overrides };
    // 选回预设槽位即视为取消微调，保持存储最小。
    if (slot === base) delete next[role];
    else next[role] = slot;
    onChange({ ...appearance, highlightOverrides: encodeHighlightOverrides(next) });
  };

  const resetRole = (role: HighlightRoleId) => {
    const next = { ...overrides };
    delete next[role];
    onChange({ ...appearance, highlightOverrides: encodeHighlightOverrides(next) });
  };

  return (
    <div className={styles.highlightPanel}>
      <div className={styles.panelHeading}>
        <span className={styles.panelTitle}>字体颜色</span>
        <small>按内容角色配色，随主题调色板自动适配明暗</small>
      </div>

      <div className={styles.presetRow} role="group" aria-label="字体颜色预设">
        {PRESET_IDS.map((preset) => {
          // 微调模式开启期间预设卡不显示选中态，避免"两边同时选中"。
          const selected = !customOpen && appearance.highlightPreset === preset;
          const sample = HIGHLIGHT_PRESETS[preset];
          return (
            <button
              aria-pressed={selected}
              className={`${styles.presetCard} ${selected ? styles.selectedPreset : ''}`}
              disabled={disabled}
              key={preset}
              onClick={() => applyPreset(preset)}
              type="button"
            >
              <strong>{HIGHLIGHT_PRESET_LABELS[preset]}</strong>
              <span className={styles.presetSwatch} aria-hidden="true">
                {(
                  [
                    'directory',
                    'executable',
                    'symlink',
                    'kw_error',
                    'prompt_user_host',
                  ] as HighlightRoleId[]
                ).map((role) => (
                  <i
                    key={role}
                    style={{ background: palette.slots[sample[role]] ?? 'transparent' }}
                  />
                ))}
              </span>
            </button>
          );
        })}
        <button
          aria-expanded={customOpen}
          className={`${styles.customToggle} ${customOpen ? styles.customOpen : ''}`}
          onClick={() => setCustomOpen((open) => !open)}
          type="button"
        >
          自定义颜色
        </button>
      </div>

      {customOpen ? (
        <div className={styles.roleList} aria-label="自定义颜色列表">
          {(['prompt', 'files', 'keywords'] as const).map((group) => (
            <div key={group} className={styles.roleGroup}>
              <div className={styles.groupTitle}>{GROUP_LABELS[group]}</div>
              {HIGHLIGHT_ROLES.filter((role) => role.group === group).map((role) => {
                const current = resolved[role.id];
                const sharers = conflicts[current.slot]?.filter((id) => id !== role.id) ?? [];
                const sharerLabels = sharers.map((id) => getHighlightRole(id).label).join('、');
                // 同色角色多时只列前 3 个，其余收进悬停提示，避免整行被长列表撑乱。
                const hintText =
                  sharers.length > 3
                    ? `与${sharers
                        .slice(0, 3)
                        .map((id) => getHighlightRole(id).label)
                        .join('、')}等 ${sharers.length} 项同色`
                    : `与${sharerLabels}同色`;
                return (
                  <div className={styles.roleRow} key={role.id}>
                    <span className={styles.roleLabel} title={role.label}>
                      {role.label}
                      {current.overridden ? <i className={styles.overrideDot} /> : null}
                    </span>
                    <div className={styles.roleMain}>
                      <span
                        className={styles.slotRow}
                        role="radiogroup"
                        aria-label={`${role.label}颜色`}
                      >
                        {SLOT_DISPLAY_ORDER.map((slot) => {
                          const hex = palette.slots[slot] ?? 'transparent';
                          const enabled = slotEnabled[slot];
                          const selected = current.slot === slot;
                          const ratio = contrastRatio(hex, palette.background);
                          return (
                            <button
                              aria-checked={selected}
                              aria-label={`色槽 ${slot}${enabled ? '' : '（对比度不足）'}`}
                              className={`${styles.slotChip} ${selected ? styles.selectedChip : ''} ${enabled ? '' : styles.disabledChip}`}
                              disabled={disabled || !enabled}
                              key={slot}
                              onClick={() => setRoleSlot(role.id, slot)}
                              style={{ background: hex }}
                              title={
                                enabled
                                  ? `对比度 ${ratio.toFixed(1)}:1`
                                  : `对比度 ${ratio.toFixed(1)}:1，低于 ${MIN_HIGHLIGHT_CONTRAST}:1 已禁选`
                              }
                              type="button"
                            />
                          );
                        })}
                      </span>
                      {current.overridden ? (
                        <button
                          className={styles.resetButton}
                          disabled={disabled}
                          onClick={() => resetRole(role.id)}
                          type="button"
                        >
                          重置
                        </button>
                      ) : null}
                      {sharers.length > 0 ? (
                        <span
                          className={styles.conflictHint}
                          title={`这些内容当前显示同一颜色：${sharerLabels}。颜色够用时无需处理；想让它们区分开，给对应行换一个颜色槽位即可。`}
                        >
                          {hintText}
                        </span>
                      ) : null}
                    </div>
                  </div>
                );
              })}
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}
