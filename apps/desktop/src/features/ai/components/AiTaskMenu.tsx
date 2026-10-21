import type {
  AiCompletedTaskResult,
  AiRuntimeTask,
  AiTaskNavigationSource,
  AiTaskOutcome,
} from '../model/ai-runtime-task';
import { AI_PROVIDERS } from '../model/ai-types';
import styles from './AiTaskMenu.module.css';

interface AiTaskMenuProps {
  runningTasks: AiRuntimeTask[];
  completedResults: AiCompletedTaskResult[];
  activeConversationId: string | null;
  onMarkAllRead: () => void;
  onSelect: (task: AiTaskNavigationSource) => void;
}

const outcomeLabels: Record<AiTaskOutcome, string> = {
  completed: '执行完成',
  failed: '执行失败',
  stopped: '已停止',
  timed_out: '执行超时',
};

const outcomeClasses: Record<AiTaskOutcome, string> = {
  completed: styles.taskStatusCompleted,
  failed: styles.taskStatusFailed,
  stopped: styles.taskStatusStopped,
  timed_out: styles.taskStatusFailed,
};

interface TaskItemProps {
  activeConversationId: string | null;
  item: AiTaskNavigationSource;
  onSelect: (task: AiTaskNavigationSource) => void;
  statusClassName: string;
  statusLabel: string;
}

function TaskItem({
  activeConversationId,
  item,
  onSelect,
  statusClassName,
  statusLabel,
}: TaskItemProps) {
  const provider = AI_PROVIDERS.find((candidate) => candidate.id === item.provider);
  return (
    <button
      aria-current={item.conversationId === activeConversationId ? 'true' : undefined}
      className={styles.taskItem}
      onClick={() => onSelect(item)}
      type="button"
    >
      <span aria-hidden="true" className={`${styles.taskStatus} ${statusClassName}`.trim()} />
      <span className={styles.taskCopy}>
        <strong>{item.target.label}</strong>
        <small>
          {statusLabel} · {provider?.name ?? item.provider}
        </small>
      </span>
      <svg viewBox="0 0 20 20" aria-hidden="true">
        <path d="m7.5 5 5 5-5 5" />
      </svg>
    </button>
  );
}

/** 任务入口只保存需要关注的运行态和未读结果，完整记录仍由历史会话负责。 */
export function AiTaskMenu({
  runningTasks,
  completedResults,
  activeConversationId,
  onMarkAllRead,
  onSelect,
}: AiTaskMenuProps) {
  const total = runningTasks.length + completedResults.length;
  return (
    <div aria-label="AI 任务" className={styles.taskPanel} role="dialog">
      <div className={styles.taskHeader}>
        <span>AI 任务</span>
        <span>{total}</span>
      </div>
      <div className={styles.taskList}>
        {runningTasks.length ? (
          <section className={styles.taskSection} aria-label="进行中的任务">
            <div className={styles.taskSectionHeader}>
              <span>进行中</span>
              <span>{runningTasks.length}</span>
            </div>
            {runningTasks.map((task) => {
              const waiting = task.pendingApproval !== null;
              return (
                <TaskItem
                  activeConversationId={activeConversationId}
                  item={task}
                  key={task.sessionId}
                  onSelect={onSelect}
                  statusClassName={waiting ? styles.taskStatusWaiting : ''}
                  statusLabel={waiting ? '等待命令确认' : '正在执行'}
                />
              );
            })}
          </section>
        ) : null}

        {completedResults.length ? (
          <section className={styles.taskSection} aria-label="待查看的任务结果">
            <div className={styles.taskSectionHeader}>
              <span>待查看</span>
              <button onClick={onMarkAllRead} type="button">
                全部已读
              </button>
            </div>
            {completedResults.map((result) => (
              <TaskItem
                activeConversationId={activeConversationId}
                item={result}
                key={result.sessionId}
                onSelect={onSelect}
                statusClassName={outcomeClasses[result.outcome]}
                statusLabel={outcomeLabels[result.outcome]}
              />
            ))}
          </section>
        ) : null}
      </div>
    </div>
  );
}
