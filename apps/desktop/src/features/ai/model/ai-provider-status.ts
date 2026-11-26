import type { CatalogSnapshot } from '../api/ai-model-catalog';

interface AiProviderStatus {
  label: string;
  tone: 'muted' | 'error';
}

/** 只展示需要处理的异常或接入方式；目录成功不等同于模型推理可用。 */
export function providerStatus(
  snapshot: CatalogSnapshot,
  desktopRuntime: boolean
): AiProviderStatus | undefined {
  if (!desktopRuntime) return undefined;
  // 本轮目录错误优先于旧接入摘要，避免陈旧状态掩盖当前故障。
  if (snapshot.error === 'not-installed') return { label: '未安装', tone: 'muted' };
  if (snapshot.error === 'unavailable') return { label: '目录读取失败', tone: 'error' };
  if (snapshot.access === 'loginRequired') return { label: '需要登录', tone: 'error' };
  if (snapshot.access === 'customEndpoint') return { label: '自定义端点', tone: 'muted' };
  if (snapshot.access === 'externalProvider') return { label: '云平台接入', tone: 'muted' };
  return undefined;
}
