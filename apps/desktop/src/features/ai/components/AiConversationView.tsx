import { Fragment, useId, useState } from 'react';

import { presentAiMessageParts, type AiProcessEntry } from '../model/ai-message-presentation';
import type { AiMessage, AiMessagePart, AiToolApprovalEvent } from '../model/ai-types';
import { AiMarkdown } from './AiMarkdown';
import styles from './AiConversationView.module.css';

interface AiConversationViewProps {
  messages: AiMessage[];
  providerName: string;
  running: boolean;
  streamText: string;
  runParts: AiMessagePart[];
  pendingApproval: AiToolApprovalEvent | null;
  approvalSubmitting: boolean;
  notice: string | null;
  lastUserMessageId?: string;
  onRetry: () => void;
  onResolveApproval: (approved: boolean) => void;
}

/** 过程区只展示可审计的动作摘要，不把 Provider 的私有推理当作回答正文。 */
function ActivityDisclosure({
  activityCount,
  entries,
  running,
}: {
  activityCount: number;
  entries: AiProcessEntry[];
  running: boolean;
}) {
  const [expanded, setExpanded] = useState(running);
  const detailsId = useId();

  return (
    <section className={styles.activityDisclosure}>
      <button
        aria-controls={detailsId}
        aria-expanded={expanded}
        className={styles.activityToggle}
        onClick={() => setExpanded((current) => !current)}
        type="button"
      >
        <span
          className={`${styles.activityStatus} ${running ? styles.activityStatusRunning : styles.activityStatusComplete}`}
        />
        <span className={styles.activityTitle}>执行过程</span>
        <span className={styles.activityMeta}>{running ? '进行中' : `${activityCount} 项`}</span>
        <svg
          className={`${styles.activityChevron} ${expanded ? styles.activityChevronExpanded : ''}`}
          viewBox="0 0 20 20"
          aria-hidden="true"
        >
          <path d="m6.75 8.25 3.25 3.5 3.25-3.5" />
        </svg>
      </button>
      {expanded ? (
        <div className={styles.activityList} id={detailsId}>
          {entries.map((entry, index) => (
            <div className={styles.activityLine} key={`${entry.kind}-${index}-${entry.content}`}>
              <span
                aria-hidden="true"
                className={`${styles.activityMarker} ${
                  entry.kind === 'tool' ? styles.activityMarkerTool : styles.activityMarkerProgress
                }`}
              />
              <span className={styles.activityText}>{entry.content}</span>
            </div>
          ))}
        </div>
      ) : null}
    </section>
  );
}

/** 已完成与流式消息共用同一结构，执行过程和最终结论始终保持两个视觉层级。 */
function AssistantContent({
  content,
  parts,
  running = false,
}: {
  content: string;
  parts?: AiMessagePart[];
  running?: boolean;
}) {
  const presentation = presentAiMessageParts(content, parts);
  const processEntries = presentation.processEntries;

  return (
    <>
      {processEntries.length ? (
        <ActivityDisclosure
          activityCount={presentation.activityCount}
          entries={processEntries}
          running={running}
        />
      ) : null}
      {presentation.answer ? (
        <div className={processEntries.length ? styles.answerContent : undefined}>
          <AiMarkdown content={presentation.answer} />
        </div>
      ) : null}
    </>
  );
}

/** 纯展示会话区域；Provider 生命周期与持久化仍由 AiPanel 统一编排。 */
export function AiConversationView({
  messages,
  providerName,
  running,
  streamText,
  runParts,
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
            <AssistantContent content={streamText} parts={runParts} running={running} />
            {running && runParts.length === 0 ? (
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
