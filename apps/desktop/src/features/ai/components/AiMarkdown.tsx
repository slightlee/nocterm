import ReactMarkdown, { type Components } from 'react-markdown';
import remarkGfm from 'remark-gfm';

import { openAiExternalLink } from '../api/ai-client';
import styles from './AiPanel.module.css';

const markdownComponents: Components = {
  // 链接统一拦截走系统浏览器，桌面端 WebView 不做站内导航。
  a: ({ children, href }) => (
    <a
      href={href}
      onClick={(event) => {
        if (!href) return;
        event.preventDefault();
        void openAiExternalLink(href);
      }}
      rel="noreferrer"
    >
      {children}
    </a>
  ),
};

/** 助手消息按 Markdown 渲染；默认不解析原始 HTML，Provider 输出无法注入脚本。 */
export function AiMarkdown({ content }: { content: string }) {
  return (
    <div className={styles.markdown}>
      <ReactMarkdown components={markdownComponents} remarkPlugins={[remarkGfm]}>
        {content}
      </ReactMarkdown>
    </div>
  );
}
