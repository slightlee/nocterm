import { Fragment } from 'react';

import type { AiMessage, AiMessagePart, AiToolApprovalEvent } from '../model/ai-types';
import { AiMarkdown } from './AiMarkdown';
import styles from './AiPanel.module.css';

interface AiConversationViewProps {
  messages: AiMessage[];
  providerName: string;
  running: boolean;
  streamText: string;
  runParts: AiMessagePart[];
  liveThinking: string;
  pendingApproval: AiToolApprovalEvent | null;
  approvalSubmitting: boolean;
  notice: string | null;
  lastUserMessageId?: string;
  onRetry: () => void;
  onResolveApproval: (approved: boolean) => void;
}

/** 已完成与流式消息共用同一渲染器，工具活动不会在任务结束时改变位置或消失。 */
function AssistantContent({ content, parts }: { content: string; parts?: AiMessagePart[] }) {
  if (!parts?.length) return <AiMarkdown content={content} />;
  return parts.map((part, index) =>
    part.type === 'text' ? (
      <AiMarkdown content={part.content} key={`text-${index}`} />
    ) : (
      <div className={styles.activityLine} key={`activity-${index}-${part.content}`}>
        <span className={styles.activityKind}>{part.kind === 'tool' ? '执行' : '思考'}</span>
        <span>{part.content}</span>
      </div>
    )
  );
}

/** 纯展示会话区域；Provider 生命周期与持久化仍由 AiPanel 统一编排。 */
export function AiConversationView({
  messages,
  providerName,
  running,
  streamText,
  runParts,
  liveThinking,
  pendingApproval,
  approvalSubmitting,
  notice,
  lastUserMessageId,
  onRetry,
  onResolveApproval,
}: AiConversationViewProps) {
  if (messages.length === 0) {
    return (
      <div className={styles.emptyState}>
        <h3>我能帮你做什么？</h3>
      </div>
    );
  }

  return (
    <div className={styles.messageList}>
      {messages.map((message) => (
        <Fragment key={message.id}>
          <article
            className={`${styles.message} ${message.role === 'user' ? styles.userMessage : styles.assistantMessage}`}
          >
            <div className={styles.messageBody}>
              <span className={styles.messageRole}>
                {message.role === 'user' ? '你' : providerName}
              </span>
              {message.role === 'assistant' ? (
                <AssistantContent content={message.content} parts={message.parts} />
              ) : (
                <p>{message.content}</p>
              )}
            </div>
          </article>
          {message.id === lastUserMessageId && notice ? (
            <div className={styles.noticeRow}>
              <span className={styles.notice}>{notice}</span>
              {!running ? (
                <button className={styles.retryButton} onClick={onRetry} type="button">
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path d="M3 12a9 9 0 1 0 2.6-6.4M3 4v5h5" />
                  </svg>
                  重新发送
                </button>
              ) : null}
            </div>
          ) : null}
        </Fragment>
      ))}
      {running || streamText ? (
        <article className={`${styles.message} ${styles.assistantMessage}`}>
          <div className={styles.messageBody}>
            <span className={styles.messageRole}>
              {providerName}
              {running ? ' · 输出中' : ''}
            </span>
            <AssistantContent content={streamText} parts={runParts} />
            {liveThinking ? (
              <div className={styles.activityLine}>
                <span className={styles.activityKind}>思考</span>
                <span>{liveThinking}</span>
              </div>
            ) : null}
            {running && !liveThinking && runParts.length === 0 ? (
              <div aria-live="polite" className={styles.executionStatus} role="status">
                <span className={styles.executionDot} />
                <span>正在思考</span>
              </div>
            ) : null}
          </div>
        </article>
      ) : null}
      {pendingApproval ? (
        <section aria-label="AI 命令确认" className={styles.approvalCard}>
          <div className={styles.approvalHeader}>
            <span>确认执行命令</span>
            <span className={styles.approvalTarget}>
              {pendingApproval.targetKind === 'ssh' ? 'SSH' : '本地'} ·{' '}
              {pendingApproval.targetLabel}
            </span>
          </div>
          <pre className={styles.approvalCommand}>{pendingApproval.command}</pre>
          <p>该命令不属于自动允许的只读范围，仅本次允许后才会执行。</p>
          <div className={styles.approvalActions}>
            <button
              disabled={approvalSubmitting}
              onClick={() => onResolveApproval(false)}
              type="button"
            >
              拒绝
            </button>
            <button
              className={styles.approveButton}
              disabled={approvalSubmitting}
              onClick={() => onResolveApproval(true)}
              type="button"
            >
              {approvalSubmitting ? '提交中…' : '允许一次'}
            </button>
          </div>
        </section>
      ) : null}
      {notice && !lastUserMessageId ? <div className={styles.notice}>{notice}</div> : null}
    </div>
  );
}
