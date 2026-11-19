import { describe, expect, it } from 'vitest';

import {
  createWorkspaceSessionTabKeys,
  editorWorkspaceTabKey,
  expandWorkspaceTabCloseKeys,
  preserveWorkspaceEditorParent,
  resolveWorkspaceTabCloseFallback,
  formatWorkspaceSessionNumber,
  groupWorkspaceSessionIndexItems,
  resolveWorkspaceTabOverflow,
  sftpWorkspaceTabKey,
  terminalWorkspaceTabKey,
} from './workspace-session-tabs';

describe('workspace session tabs', () => {
  it('keeps terminal and file sessions visible in one ordered tab strip', () => {
    expect(createWorkspaceSessionTabKeys(['remote:1:first', 'remote:1:second'], ['1'])).toEqual([
      'terminal:remote:1:first',
      'terminal:remote:1:second',
      'sftp:1',
    ]);
  });

  it('keeps equal raw ids distinct across terminal and file sessions', () => {
    expect(terminalWorkspaceTabKey('1')).not.toBe(sftpWorkspaceTabKey('1'));
  });

  it('appends editors without colliding with manager or terminal ids', () => {
    expect(createWorkspaceSessionTabKeys(['1'], ['1'], ['1'])).toEqual([
      'terminal:1',
      'sftp:1',
      'editor:1',
    ]);
    expect(editorWorkspaceTabKey('1')).not.toBe(sftpWorkspaceTabKey('1'));
  });

  const editors = [
    { id: 'remote', source: { kind: 'remote' as const, connectionId: '1' } },
    { id: 'other', source: { kind: 'remote' as const, connectionId: '2' } },
    { id: 'local', source: { kind: 'local' as const } },
  ];

  it('expands parent closure only to dependent remote editors and deduplicates keys', () => {
    expect(expandWorkspaceTabCloseKeys(['sftp:1', 'editor:remote'], editors)).toEqual([
      'sftp:1',
      'editor:remote',
    ]);
    expect(expandWorkspaceTabCloseKeys(['editor:local'], editors)).toEqual(['editor:local']);
  });

  it('retains a remote editor parent only when the target editor survives', () => {
    expect(preserveWorkspaceEditorParent(['sftp:1', 'sftp:2'], 'editor:remote', editors)).toEqual([
      'sftp:2',
    ]);
    expect(
      preserveWorkspaceEditorParent(['sftp:1', 'editor:remote'], 'editor:remote', editors)
    ).toEqual(['sftp:1', 'editor:remote']);
    expect(preserveWorkspaceEditorParent(['sftp:1'], 'editor:local', editors)).toEqual(['sftp:1']);
  });

  it('falls back to the previous surviving visual tab, then next, then empty', () => {
    const keys = createWorkspaceSessionTabKeys(['1'], ['1'], ['remote', 'local']);
    expect(
      resolveWorkspaceTabCloseFallback(keys, 'editor:remote', ['sftp:1', 'editor:remote'])
    ).toBe('terminal:1');
    expect(resolveWorkspaceTabCloseFallback(keys, 'terminal:1', ['terminal:1', 'sftp:1'])).toBe(
      'editor:remote'
    );
    expect(resolveWorkspaceTabCloseFallback(keys, 'editor:local', keys)).toBeNull();
    expect(resolveWorkspaceTabCloseFallback(keys, 'sftp:1', ['editor:remote'])).toBe('sftp:1');
  });

  it('formats visual terminal numbers with a stable two-digit width', () => {
    expect(formatWorkspaceSessionNumber(1)).toBe('01');
    expect(formatWorkspaceSessionNumber(12)).toBe('12');
    expect(formatWorkspaceSessionNumber(128)).toBe('128');
  });

  it('shows scroll controls only while more tabs exist in that direction', () => {
    expect(
      resolveWorkspaceTabOverflow({ scrollLeft: 0, scrollWidth: 800, clientWidth: 500 })
    ).toEqual({ left: false, right: true });
    expect(
      resolveWorkspaceTabOverflow({ scrollLeft: 150, scrollWidth: 800, clientWidth: 500 })
    ).toEqual({ left: true, right: true });
    expect(
      resolveWorkspaceTabOverflow({ scrollLeft: 300, scrollWidth: 800, clientWidth: 500 })
    ).toEqual({ left: true, right: false });
  });

  it('filters the complete session index and keeps server groups stable', () => {
    const entries = [
      {
        key: terminalWorkspaceTabKey('remote:1:first'),
        groupId: 'remote:1',
        groupLabel: '生产服务器',
        label: '终端 01',
        searchText: 'root 10.0.0.1',
      },
      {
        key: sftpWorkspaceTabKey('1'),
        groupId: 'remote:1',
        groupLabel: '生产服务器',
        label: '文件',
        searchText: 'root 10.0.0.1',
      },
      {
        key: terminalWorkspaceTabKey('local:first'),
        groupId: 'local',
        groupLabel: '本地',
        label: '本地终端',
        searchText: 'local shell',
      },
    ];

    expect(groupWorkspaceSessionIndexItems(entries, '')).toEqual([
      { id: 'remote:1', label: '生产服务器', entries: entries.slice(0, 2) },
      { id: 'local', label: '本地', entries: entries.slice(2) },
    ]);
    expect(groupWorkspaceSessionIndexItems(entries, '10.0.0.1')).toHaveLength(1);
    expect(groupWorkspaceSessionIndexItems(entries, '文件')[0]?.entries).toEqual([entries[1]]);
  });
});
