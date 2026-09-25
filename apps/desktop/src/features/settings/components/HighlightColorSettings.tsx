import { useEffect, useMemo, useRef, useState } from 'react';

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
 */
const SLOT_DISPLAY_ORDER = [0, 8, 1, 9, 3, 11, 2, 10, 6, 14, 4, 12, 5, 13, 7, 15];

const GROUP_LABELS: Record<'prompt' | 'files' | 'keywords', string> = {
  prompt: '提示符与元数据',
  files: '文件类型',
  keywords: '关键词',
};

/**
 * 角色列表双列排布：文件类型（11 角色）独占左列，提示符 + 关键词（7 角色）
 * 共右列。分组不跨列，扫描方向仍是自上而下；行内只放「名称 + 当前色块」，
 * 列表高度恒定、无展开收起，改色操作集中在顶部共享色板。
 */
const ROLE_COLUMNS: { groups: ('prompt' | 'files' | 'keywords')[] }[] = [
  { groups: ['files'] },
  { groups: ['prompt', 'keywords'] },
];

interface HighlightColorSettingsProps {
  appearance: TerminalAppearance;
  palette: HighlightPaletteSnapshot;
  disabled: boolean;
  onChange: (next: TerminalAppearance) => void;
  /** 与预览联动：当前悬停的角色行（双向高亮）。 */
  activeRole?: HighlightRoleId | null;
  onHoverRole?: (role: HighlightRoleId | null) => void;
  /** 点击预览文字的定位请求：展开自定义列表并选中对应角色。 */
  reveal?: { role: HighlightRoleId; n: number } | null;
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
  activeRole = null,
  onHoverRole,
  reveal = null,
}: HighlightColorSettingsProps) {
  const [customOpen, setCustomOpen] = useState(() => appearance.highlightOverrides !== '');
  const [selectedRole, setSelectedRole] = useState<HighlightRoleId>(HIGHLIGHT_ROLES[0].id);
  const lastRevealRef = useRef(0);

  // 预览点选联动：展开自定义列表并选中对应角色（等待展开渲染完成后再滚）。
  useEffect(() => {
    if (!reveal || reveal.n === lastRevealRef.current) return;
    lastRevealRef.current = reveal.n;
    const openTimer = window.setTimeout(() => {
      setCustomOpen(true);
      setSelectedRole(reveal.role);
    }, 0);
    const scrollTimer = window.setTimeout(() => {
      document
        .getElementById(`highlight-role-${reveal.role}`)
        ?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    }, 80);
    return () => {
      window.clearTimeout(openTimer);
      window.clearTimeout(scrollTimer);
    };
  }, [reveal]);

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

  const setRoleSlot = (slot: number) => {
    const base = HIGHLIGHT_PRESETS[appearance.highlightPreset][selectedRole];
    const next = { ...overrides };
    // 选回预设槽位即视为取消微调，保持存储最小。
    if (slot === base) delete next[selectedRole];
    else next[selectedRole] = slot;
    onChange({ ...appearance, highlightOverrides: encodeHighlightOverrides(next) });
  };

  const resetRole = (role: HighlightRoleId) => {
    const next = { ...overrides };
    delete next[role];
    onChange({ ...appearance, highlightOverrides: encodeHighlightOverrides(next) });
  };

  // 角色行右侧的当前色小方块：与预览同一映射规则（加粗基色经亮色变体呈现）。
  const roleSwatchColor = (id: HighlightRoleId) => {
    const role = resolved[id];
    const slot = role.bold && role.slot <= 7 ? role.slot + 8 : role.slot;
    return palette.slots[slot] ?? 'transparent';
  };

  // 顶部共享色板只服务选中角色：同色提示与重置也只在这一处出现，
  // 18 行各挂一套色板与提示的重复噪音由主从结构整体消除。
  const active = resolved[selectedRole];
  const activeSharers = conflicts[active.slot]?.filter((id) => id !== selectedRole) ?? [];
  const activeHint =
    activeSharers.length > 3
      ? `与${activeSharers
          .slice(0, 3)
          .map((id) => getHighlightRole(id).label)
          .join('、')}等 ${activeSharers.length} 项同色`
      : `与${activeSharers.map((id) => getHighlightRole(id).label).join('、')}同色`;

  const renderGroup = (group: 'prompt' | 'files' | 'keywords') => (
    <div className={styles.roleGroup} key={group}>
      <div className={styles.groupTitle}>{GROUP_LABELS[group]}</div>
      {HIGHLIGHT_ROLES.filter((role) => role.group === group).map((role) => {
        const current = resolved[role.id];
        return (
          <button
            className={`${styles.roleRow} ${activeRole === role.id ? styles.roleRowActive : ''} ${selectedRole === role.id ? styles.roleRowSelected : ''}`}
            id={`highlight-role-${role.id}`}
            key={role.id}
            onClick={() => setSelectedRole(role.id)}
            onMouseEnter={() => onHoverRole?.(role.id)}
            onMouseLeave={() => onHoverRole?.(null)}
            type="button"
          >
            <span className={styles.roleLabel}>
              {role.label}
              {current.overridden ? <i className={styles.overrideDot} /> : null}
            </span>
            <span
              aria-hidden="true"
              className={styles.roleSwatch}
              style={{ background: roleSwatchColor(role.id) }}
            />
          </button>
        );
      })}
    </div>
  );

  return (
    <div className={styles.highlightPanel}>
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
              <strong>{HIGHLIGHT_PRESET_LABELS[preset]}</strong>
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

      {/* 收起态的轻说明：填住面板高度落差，也交代预设与自定义两个入口的关系。 */}
      {!customOpen ? (
        <p className={styles.presetHint}>
          预设一键切换 18 个角色的整套颜色；展开自定义颜色可逐角色微调。
        </p>
      ) : null}

      {customOpen ? (
        <div aria-label="自定义颜色" className={styles.customArea}>
          <div className={styles.masterHeader}>
            <span className={styles.masterLabel}>
              正在修改：
              <strong>{getHighlightRole(selectedRole).label}</strong>
            </span>
            {activeSharers.length > 0 ? (
              <span
                className={styles.conflictHint}
                title={`这些内容当前显示同一颜色：${activeSharers
                  .map((id) => getHighlightRole(id).label)
                  .join('、')}。颜色够用时无需处理；想让它们区分开，给对应行换一个颜色槽位即可。`}
              >
                {activeHint}
              </span>
            ) : null}
            {active.overridden ? (
              <button
                className={styles.resetButton}
                disabled={disabled}
                onClick={() => resetRole(selectedRole)}
                type="button"
              >
                重置
              </button>
            ) : null}
          </div>
          <div
            className={styles.sharedPalette}
            role="radiogroup"
            aria-label={`${getHighlightRole(selectedRole).label}颜色`}
          >
            {SLOT_DISPLAY_ORDER.map((slot) => {
              const hex = palette.slots[slot] ?? 'transparent';
              const enabled = slotEnabled[slot];
              const selected = active.slot === slot;
              const ratio = contrastRatio(hex, palette.background);
              return (
                <button
                  aria-checked={selected}
                  aria-label={`色槽 ${slot}${enabled ? '' : '（对比度不足）'}`}
                  className={`${styles.slotChip} ${selected ? styles.selectedChip : ''} ${enabled ? '' : styles.disabledChip}`}
                  disabled={disabled || !enabled}
                  key={slot}
                  onClick={() => setRoleSlot(slot)}
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
          </div>
          <div className={styles.roleColumns}>
            {ROLE_COLUMNS.map((column) => (
              <div className={styles.roleColumn} key={column.groups[0]}>
                {column.groups.map((group) => renderGroup(group))}
              </div>
            ))}
          </div>
        </div>
      ) : null}
    </div>
  );
}
