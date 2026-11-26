import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import { AiReasoningOptions } from './AiReasoningOptions';

describe('思考强度展示', () => {
  it('模型默认标记与用户选中档位独立，不额外生成默认选项', () => {
    const markup = renderToStaticMarkup(
      <AiReasoningOptions
        model={{
          id: 'test-model',
          name: 'Test',
          source: 'cli',
          reasoningEfforts: ['low', 'high'],
          defaultReasoningEffort: 'low',
        }}
        selected="high"
        disabled={false}
        onBack={() => undefined}
        onSelect={() => undefined}
      />
    );
    const options = markup.match(/<button\b[^>]*>[\s\S]*?<\/button>/g) ?? [];
    expect(options).toHaveLength(2);
    expect(options[0]).toContain('默认');
    expect(options[0]).toContain('aria-selected="false"');
    expect(options[1]).toContain('aria-selected="true"');
    expect(options[1]).not.toContain('默认');
  });

  it('未选模型时提示选择，不展示隐式模型或虚构档位', () => {
    const markup = renderToStaticMarkup(
      <AiReasoningOptions disabled={false} onBack={() => undefined} onSelect={() => undefined} />
    );
    expect(markup).toContain('选择模型');
    expect(markup).not.toContain('Agent 默认模型');
    expect(markup).not.toContain('role="option"');
  });

  it('别名缺少默认档位时提示由 Agent 决定，不标注任何档位为默认', () => {
    const markup = renderToStaticMarkup(
      <AiReasoningOptions
        model={{
          id: 'fable',
          name: 'glm-5.3-flash',
          source: 'cli',
          reasoningEfforts: ['low', 'medium'],
        }}
        disabled={false}
        onBack={() => undefined}
        onSelect={() => undefined}
      />
    );
    expect(markup).toContain('未选择档位时，由 Agent 决定。');
    expect(markup).not.toContain('默认');
    expect(markup).not.toContain('aria-selected="true"');
  });

  it('思考强度面板不混入会话速度设置', () => {
    const markup = renderToStaticMarkup(
      <AiReasoningOptions
        model={{
          id: 'fast-model',
          name: 'Fast Model',
          source: 'cli',
          supportsFastMode: true,
          reasoningEfforts: ['medium'],
        }}
        disabled={false}
        onBack={() => undefined}
        onSelect={() => undefined}
      />
    );
    expect(markup).toContain('思考强度');
    expect(markup).not.toContain('响应速度');
    expect(markup).not.toContain('标准');
    expect(markup).not.toContain('快速');
  });
});
