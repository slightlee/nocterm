import type { AiExecutionPresentation, AiMessagePart, AiToolExecutionEvent } from './ai-types';

/**
 * 执行事件归并前的不可变快照。
 * `executions` 按 `executionId` 索引，只保留 sequence 更大的权威投影。
 */
export interface AiExecutionSnapshot {
  executions: Map<string, AiExecutionPresentation>;
}

/**
 * 单个执行事件的归并结果。
 * `changed` 表示执行卡展示状态发生变化；乱序或迟到事件被丢弃。
 */
export interface AiExecutionReduction extends AiExecutionSnapshot {
  changed: boolean;
}

/** 从后端事件投影出执行卡展示模型；不包含命令、参数或输出。 */
export function executionEventToPresentation(event: AiToolExecutionEvent): AiExecutionPresentation {
  return {
    executionId: event.executionId,
    sequence: event.sequence,
    phase: event.phase,
    approvalState: event.approvalState,
    executionMode: event.executionMode,
    fallbackReason: event.fallbackReason,
    resultState: event.resultState,
    dispatchReceipt: event.dispatchReceipt,
    exitCode: event.exitCode,
    durationMs: event.durationMs,
    errorCode: event.errorCode,
    tool: event.tool,
    targetKind: event.targetKind,
    updatedAtMs: event.updatedAtMs,
  };
}

/**
 * 把一个后端执行事件归并为不可变快照。
 * 只接受 sequence 更大的事件，防止乱序、重复或迟到事件覆盖最新状态。
 */
export function reduceAiExecutionEvent(
  event: AiToolExecutionEvent,
  current: AiExecutionSnapshot
): AiExecutionReduction {
  const presentation = executionEventToPresentation(event);
  const existing = current.executions.get(event.executionId);
  if (existing && existing.sequence >= event.sequence) {
    return { ...current, changed: false };
  }
  const executions = new Map(current.executions);
  executions.set(event.executionId, presentation);
  return { executions, changed: true };
}

/**
 * 把执行快照投影为有序消息 parts。
 * 按 `updatedAtMs` 降序排列，最新的执行卡排在最后（与工具活动顺序一致）。
 */
export function executionSnapshotToParts(snapshot: AiExecutionSnapshot): AiMessagePart[] {
  return [...snapshot.executions.values()]
    .sort((a, b) => a.updatedAtMs - b.updatedAtMs)
    .map((execution) => ({ type: 'execution' as const, execution }));
}

/** 执行卡的面向用户状态标签；与后端 phase 对应但不暴露内部状态机名。 */
export function executionPhaseLabel(phase: AiExecutionPresentation['phase']): string {
  switch (phase) {
    case 'requested':
      return '已接收';
    case 'awaiting_approval':
      return '等待确认';
    case 'authorized':
      return '已授权';
    case 'terminal_reserved':
      return '正在占用终端';
    case 'dispatching':
    case 'dispatched':
      return '正在执行';
    case 'completed':
      return '执行成功';
    case 'failed':
      return '执行失败';
    case 'denied':
      return '已拒绝';
    case 'cancelled':
      return '已取消';
    case 'timed_out':
      return '已超时';
    case 'result_unknown':
      return '结果未知';
    default:
      return '未知状态';
  }
}

/** 执行模式的面向用户标签；独立 exec 降级时需要明确提示不继承可见终端状态。 */
export function executionModeLabel(mode: AiExecutionPresentation['executionMode']): string {
  switch (mode) {
    case 'local_visible_osc':
    case 'ssh_visible_osc':
      return '当前终端 · Shell Integration';
    case 'local_visible_marker':
      return '当前终端 · Marker 探针';
    case 'ssh_visible_marker':
      return '当前终端 · Shell Integration + Marker 恢复';
    case 'ssh_independent_exec':
      return '隔离执行 · SSH exec channel';
    case 'unselected':
    default:
      return '执行模式待定';
  }
}

/** 稳定降级码的用户提示；未知新码仍保留可诊断信息。 */
export function executionFallbackLabel(reason: string | null): string | null {
  switch (reason) {
    case null:
      return null;
    case 'SSH_VISIBLE_NOT_ENABLED':
      return '当前连接未启用终端可见执行';
    case 'SSH_PROMPT_NOT_READY':
      return '终端提示符未就绪，已安全回退';
    case 'SSH_SHELL_UNSUPPORTED':
      return '当前远程 Shell 不支持终端可见执行，已隔离执行';
    case 'SSH_INTEGRATION_UNAVAILABLE':
      return 'Shell Integration 初始化失败，已隔离执行';
    case 'SSH_CONTROL_PLANE_UNAVAILABLE':
      return '终端控制面不可用，已隔离执行';
    case 'SSH_PROMPT_CHANGED':
      return '检测到用户输入，已安全回退';
    default:
      return `已回退：${reason}`;
  }
}

/** 判断执行卡是否处于需要用户注意的高风险状态。 */
export function isExecutionHighRisk(presentation: AiExecutionPresentation): boolean {
  return (
    presentation.phase === 'result_unknown' ||
    presentation.phase === 'failed' ||
    presentation.phase === 'timed_out'
  );
}

/** 判断执行卡是否已到达终态，不再需要实时更新。 */
export function isExecutionTerminal(presentation: AiExecutionPresentation): boolean {
  return ['completed', 'failed', 'denied', 'cancelled', 'timed_out', 'result_unknown'].includes(
    presentation.phase
  );
}
