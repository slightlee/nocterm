import type { AiProviderId } from './ai-types';
import { isReasoningEffort, type AiReasoningEffort } from './ai-reasoning';

export interface AiModelOption {
  id: string;
  name: string;
  source: 'cli' | 'catalog' | 'custom';
  isLegacy?: boolean;
  reasoningEfforts?: AiReasoningEffort[];
  defaultReasoningEffort?: AiReasoningEffort;
  supportsFastMode?: boolean;
  resolvedModel?: string;
  minimumCliVersion?: string;
  unavailableReason?: 'cliUpdateRequired' | 'cliVersionUnknown';
}

/** 缓存只接受目录白名单字段；旧字符串数组升级为 CLI 项，不改变调用 ID。 */
export function parseModelCatalog(value: unknown): AiModelOption[] {
  if (!Array.isArray(value) || value.length > 1024) throw new Error('模型目录格式无效');
  return value.map((item: unknown) => {
    if (isModelId(item)) return { id: item, name: item, source: 'cli' };
    if (!item || typeof item !== 'object') throw new Error('模型目录格式无效');
    const m = item as Record<string, unknown>;
    if (
      !isModelId(m.id) ||
      typeof m.name !== 'string' ||
      !m.name.trim() ||
      m.name.length > 200 ||
      Array.from(m.name).some(
        (char) =>
          char.charCodeAt(0) < 32 || (char.charCodeAt(0) >= 127 && char.charCodeAt(0) <= 159)
      ) ||
      (m.source !== 'cli' && m.source !== 'catalog') ||
      (m.isLegacy !== undefined && typeof m.isLegacy !== 'boolean') ||
      (m.reasoningEfforts !== undefined &&
        (!Array.isArray(m.reasoningEfforts) ||
          m.reasoningEfforts.length > 8 ||
          !m.reasoningEfforts.every(isReasoningEffort))) ||
      (m.defaultReasoningEffort !== undefined &&
        (!isReasoningEffort(m.defaultReasoningEffort) ||
          !Array.isArray(m.reasoningEfforts) ||
          !m.reasoningEfforts.includes(m.defaultReasoningEffort))) ||
      (m.supportsFastMode !== undefined && typeof m.supportsFastMode !== 'boolean') ||
      (m.resolvedModel !== undefined && !isModelId(m.resolvedModel)) ||
      (m.minimumCliVersion !== undefined &&
        (typeof m.minimumCliVersion !== 'string' ||
          !/^\d{1,10}\.\d{1,10}\.\d{1,10}$/.test(m.minimumCliVersion))) ||
      (m.unavailableReason !== undefined &&
        m.unavailableReason !== 'cliUpdateRequired' &&
        m.unavailableReason !== 'cliVersionUnknown')
    ) {
      throw new Error('模型目录格式无效');
    }
    return {
      id: m.id,
      name: m.name,
      source: m.source,
      ...(m.isLegacy === true ? { isLegacy: true } : {}),
      ...(m.reasoningEfforts
        ? { reasoningEfforts: [...new Set(m.reasoningEfforts as AiReasoningEffort[])] }
        : {}),
      ...(m.defaultReasoningEffort
        ? { defaultReasoningEffort: m.defaultReasoningEffort as AiReasoningEffort }
        : {}),
      ...(m.supportsFastMode === true ? { supportsFastMode: true } : {}),
      resolvedModel: m.resolvedModel as string | undefined,
      minimumCliVersion: m.minimumCliVersion as string | undefined,
      unavailableReason: m.unavailableReason as AiModelOption['unavailableReason'],
    };
  });
}

export function modelUnavailableLabel(model?: AiModelOption): string {
  if (model?.unavailableReason === 'cliUpdateRequired')
    return `需要 Claude Code ≥ ${model.minimumCliVersion ?? '新版'}`;
  if (model?.unavailableReason === 'cliVersionUnknown') return '无法核对 Claude Code 版本';
  return '';
}

/** 搜索只改变候选项的可见性；保留 CLI 原始 ID，临时缺失的已选项也能回显。 */
export function filterModelOptions(
  catalog: readonly AiModelOption[],
  custom: readonly string[],
  selected: string | undefined,
  query: string,
  showLegacy = false
): AiModelOption[] {
  const term = query.trim().toLowerCase();
  const models = new Map(catalog.map((m) => [m.id, m]));
  // 自定义项不得覆盖已知目录的元数据或绕过版本限制。
  for (const id of [...custom, ...(selected ? [selected] : [])]) {
    if (!models.has(id)) models.set(id, { id, name: id, source: 'custom' });
  }
  const grouped = ['cli', 'catalog', 'custom'].flatMap((source) => {
    const group = [...models.values()].filter((model) => model.source === source);
    // 只排序维护的 Claude 具体版本，CLI 别名和自定义 ID 没有可推断的发布日期。
    return source === 'catalog' ? group.sort(compareCatalogVersions) : group;
  });
  return grouped.filter(
    (model) =>
      // 搜索和已有选择可直接访问历史模型，折叠只减少默认列表的长度。
      (!model.isLegacy ||
        model.source !== 'catalog' ||
        showLegacy ||
        !!term ||
        model.id === selected) &&
      [model.id, model.name, model.resolvedModel ?? ''].join(' ').toLowerCase().includes(term)
  );
}

/** 数字比较避免 5.10 排到 5.5 后；不拿 CLI 最低版本充当模型发布日期。 */
function compareCatalogVersions(left: AiModelOption, right: AiModelOption): number {
  const version = (id: string) => /^claude-[a-z]+-(\d+)(?:-(\d+))?$/.exec(id);
  const a = version(left.id);
  const b = version(right.id);
  if (!a || !b) return 0;
  return Number(b[1]) - Number(a[1]) || Number(b[2] ?? 0) - Number(a[2] ?? 0);
}

/** 只保存 CLI 模型标识；接口地址与鉴权继续由对应 Agent 管理。 */
export function isModelId(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    value.length <= 200 &&
    /^[a-zA-Z0-9]/.test(value) &&
    !/[^-a-zA-Z0-9._/:@+[\]]/.test(value) &&
    !value.includes('://')
  );
}

function catalogKey(provider: AiProviderId) {
  return 'nocterm.ai.custom-models.' + provider;
}

export function readCustomModels(storage: Storage, provider: AiProviderId): string[] {
  const raw = storage.getItem(catalogKey(provider));
  if (raw === null) return [];
  const values: unknown = JSON.parse(raw);
  if (!Array.isArray(values) || !values.every(isModelId))
    throw new Error('本机自定义模型列表格式无效');
  return values;
}
