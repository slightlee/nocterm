import { describe, expect, it } from 'vitest';

import {
  extractAiActivities,
  extractAiStreamDelta,
  extractAiText,
  isAiFullTextEvent,
} from './ai-output';

describe('extractAiText', () => {
  it('hides lifecycle events and extracts Codex agent messages', () => {
    expect(extractAiText('{"type":"thread.started","thread_id":"1"}')).toBeNull();
    expect(
      extractAiText('{"type":"item.completed","item":{"type":"agent_message","text":"已完成分析"}}')
    ).toBe('已完成分析');
    expect(
      extractAiText(
        '{"method":"item/completed","params":{"item":{"type":"agentMessage","text":"持续会话完成"}}}'
      )
    ).toBe('持续会话完成');
  });

  it('extracts Grok ACP message chunks', () => {
    expect(
      extractAiText(
        '{"method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"检查完成"}}}}'
      )
    ).toBe('检查完成');
  });

  it('extracts Claude stream-json assistant content', () => {
    expect(
      extractAiText(
        '{"type":"assistant","message":{"content":[{"type":"text","text":"检查完成"}]}}'
      )
    ).toBe('检查完成');
  });

  it('skips Claude result events that repeat the streamed answer', () => {
    expect(
      extractAiText('{"type":"result","subtype":"success","result":"检查完成","is_error":false}')
    ).toBeNull();
  });

  it('extracts Grok streaming-json text and ignores thought events as final text', () => {
    expect(extractAiText('{"type":"text","data":"检查完成"}')).toBe('检查完成');
    expect(extractAiText('{"type":"thought","data":"先检查 Docker"}')).toBeNull();
  });

  it('keeps plain text diagnostics readable', () => {
    expect(extractAiText('Provider 启动失败')).toBe('Provider 启动失败');
  });
});

describe('extractAiActivities', () => {
  it('extracts Grok ACP tool calls without exposing protocol names', () => {
    expect(
      extractAiActivities(
        '{"method":"session/update","params":{"update":{"sessionUpdate":"tool_call","title":"查看 Docker 容器","kind":"other"}}}'
      )
    ).toEqual([{ kind: 'tool', text: '查看 Docker 容器' }]);
  });

  it('extracts Claude thinking and tool use as activity entries', () => {
    const activities = extractAiActivities(
      '{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"先确认\\nDocker 状态"},{"type":"tool_use","name":"Bash","input":{"command":"docker ps"}}]}}'
    );

    expect(activities).toEqual([
      { kind: 'thinking', text: '先确认 Docker 状态' },
      { kind: 'tool', text: 'Bash docker ps' },
    ]);
  });

  it('summarizes tool calls without a recognizable input', () => {
    expect(
      extractAiActivities(
        '{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Task"}]}}'
      )
    ).toEqual([{ kind: 'tool', text: 'Task' }]);
  });

  it('extracts Codex command execution and reasoning entries', () => {
    expect(
      extractAiActivities(
        '{"type":"item.started","item":{"type":"command_execution","command":"docker ps -a"}}'
      )
    ).toEqual([{ kind: 'tool', text: 'docker ps -a' }]);
    expect(
      extractAiActivities(
        '{"type":"item.completed","item":{"type":"reasoning","text":"需要检查端口"}}'
      )
    ).toEqual([{ kind: 'thinking', text: '需要检查端口' }]);
  });

  it('shows Codex MCP tool calls while the terminal command is running', () => {
    expect(
      extractAiActivities(
        '{"type":"item.started","item":{"type":"mcp_tool_call","server":"nocterm","tool":"list_docker_containers","arguments":{"includeStopped":false}}}'
      )
    ).toEqual([{ kind: 'tool', text: '查看 Docker 容器' }]);
    expect(
      extractAiActivities(
        '{"method":"item/started","params":{"item":{"type":"mcpToolCall","server":"nocterm","tool":"get_service_status","arguments":{"service":"docker.service"}}}}'
      )
    ).toEqual([{ kind: 'tool', text: '检查服务状态：docker.service' }]);
    expect(
      extractAiActivities(
        '{"method":"item/started","params":{"item":{"type":"mcpToolCall","server":"nocterm","tool":"read_docker_logs","arguments":{"container":"new-api","lines":100}}}}'
      )
    ).toEqual([{ kind: 'tool', text: '读取容器日志：new-api' }]);
  });

  it('ignores answer text, lifecycle events and plain lines', () => {
    expect(
      extractAiActivities(
        '{"type":"assistant","message":{"content":[{"type":"text","text":"回答正文"}]}}'
      )
    ).toEqual([]);
    expect(extractAiActivities('{"type":"system","subtype":"init"}')).toEqual([]);
    expect(extractAiActivities('Provider 启动失败')).toEqual([]);
    expect(extractAiActivities('')).toEqual([]);
  });

  it('truncates overlong thinking summaries', () => {
    const activities = extractAiActivities(
      `{"type":"item.completed","item":{"type":"reasoning","text":"${'思'.repeat(300)}"}}`
    );

    expect(activities[0]?.text).toHaveLength(161);
    expect(activities[0]?.text.endsWith('…')).toBe(true);
  });
});

describe('extractAiStreamDelta', () => {
  it('extracts Grok ACP answer and thought chunks', () => {
    expect(
      extractAiStreamDelta(
        '{"method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"容器正常"}}}}'
      )
    ).toEqual({ answer: '容器正常' });
    expect(
      extractAiStreamDelta(
        '{"method":"session/update","params":{"update":{"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"读取状态"}}}}'
      )
    ).toEqual({ thinking: '读取状态' });
  });

  it('extracts Claude text and thinking deltas', () => {
    expect(
      extractAiStreamDelta(
        '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"你好"}}}'
      )
    ).toEqual({ answer: '你好' });
    expect(
      extractAiStreamDelta(
        '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"先查容器"}}}'
      )
    ).toEqual({ thinking: '先查容器' });
    expect(
      extractAiStreamDelta(
        '{"type":"stream_event","event":{"type":"content_block_start","index":0}}'
      )
    ).toBeNull();
    expect(extractAiStreamDelta('{"type":"system","subtype":"init"}')).toBeNull();
    expect(extractAiStreamDelta('not json')).toBeNull();
  });

  it('extracts Codex app-server answer and reasoning deltas', () => {
    expect(
      extractAiStreamDelta('{"method":"item/agentMessage/delta","params":{"delta":"检查完成"}}')
    ).toEqual({ answer: '检查完成' });
    expect(
      extractAiStreamDelta(
        '{"method":"item/reasoning/summaryTextDelta","params":{"delta":"先检查状态"}}'
      )
    ).toEqual({ thinking: '先检查状态' });
  });

  it('extracts Grok streaming-json answer and thought deltas', () => {
    expect(extractAiStreamDelta('{"type":"text","data":"容器正常"}')).toEqual({
      answer: '容器正常',
    });
    expect(extractAiStreamDelta('{"type":"thought","data":"读取状态"}')).toEqual({
      thinking: '读取状态',
    });
  });
});

describe('isAiFullTextEvent', () => {
  it('marks assistant and result events as full-text duplicates of streamed deltas', () => {
    expect(
      isAiFullTextEvent(
        '{"type":"assistant","message":{"content":[{"type":"text","text":"全文"}]}}'
      )
    ).toBe(true);
    expect(isAiFullTextEvent('{"type":"result","result":"全文"}')).toBe(true);
    expect(
      isAiFullTextEvent('{"type":"stream_event","event":{"type":"content_block_delta"}}')
    ).toBe(false);
    expect(isAiFullTextEvent('not json')).toBe(false);
    expect(
      isAiFullTextEvent(
        '{"method":"item/completed","params":{"item":{"type":"agentMessage","text":"全文"}}}'
      )
    ).toBe(true);
  });
});
