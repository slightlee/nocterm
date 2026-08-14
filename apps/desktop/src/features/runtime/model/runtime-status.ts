import type { RuntimeStatus } from './runtime-types';

export function runtimeStatusLabel(status: RuntimeStatus): string {
  const labels: Record<RuntimeStatus, string> = {
    checking: 'checking runtime',
    preview: 'browser preview',
    ready: 'desktop core ready',
    error: 'runtime unavailable',
  };

  return labels[status];
}
