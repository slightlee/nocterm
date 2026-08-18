import { describe, expect, it } from 'vitest';

import {
  connectionRequiresReconnect,
  getNextGroupName,
  groupConnections,
  isConnectionReady,
  markCredentialBound,
  resolveConnectionIndicator,
  resolveGroupDropSortOrder,
  sortConnectionGroups,
  UNGROUPED_CONNECTION_GROUP_ID,
} from './connection-list-model';
import type { ConnectionGroup, ConnectionProfile } from '../types/connection-types';

function connection(
  id: number,
  groupId: string | null,
  sortOrder: number | null
): ConnectionProfile {
  return {
    id,
    name: `连接 ${id}`,
    host: '127.0.0.1',
    port: 22,
    username: 'tester',
    authentication: 'private_key',
    createdAt: 0,
    updatedAt: 0,
    groupId,
    groupName: null,
    sortOrder,
    credentialStatus: 'missing',
  };
}

describe('connection list model', () => {
  it('creates the next available default group name', () => {
    expect(getNextGroupName([])).toBe('新建分组');
    expect(getNextGroupName([{ name: '新建分组' }, { name: '新建分组 2' }])).toBe('新建分组 3');
  });

  it('groups connections and keeps explicit sort order', () => {
    const groups: ConnectionGroup[] = [{ id: 'prod', name: '生产环境', sortOrder: 0 }];
    const result = groupConnections(
      [connection(2, 'prod', 20), connection(1, 'prod', 10), connection(3, null, null)],
      groups
    );

    expect(result.map((group) => group.id)).toEqual(['prod', UNGROUPED_CONNECTION_GROUP_ID]);
    expect(result[0].connections.map((item) => item.id)).toEqual([1, 2]);
    expect(result[1].connections.map((item) => item.id)).toEqual([3]);
  });

  it('does not render an empty ungrouped section', () => {
    const result = groupConnections(
      [connection(1, 'prod', null)],
      [{ id: 'prod', name: '生产环境', sortOrder: 0 }]
    );

    expect(result).toHaveLength(1);
    expect(result[0].id).toBe('prod');
  });

  it('keeps explicitly created empty groups visible', () => {
    const result = groupConnections([], [{ id: 'new', name: '新建分组', sortOrder: 0 }]);

    expect(result).toEqual([{ id: 'new', name: '新建分组', connections: [] }]);
  });

  it('keeps groups sorted after an in-memory update', () => {
    const sorted = sortConnectionGroups([
      { id: 'ops', name: '运维', sortOrder: 20 },
      { id: 'prod', name: '生产', sortOrder: 10 },
    ]);

    expect(sorted.map((group) => group.id)).toEqual(['prod', 'ops']);
  });

  it('calculates a stable sort value when a group is dropped between groups', () => {
    const groups: ConnectionGroup[] = [
      { id: 'first', name: '一组', sortOrder: 1000 },
      { id: 'second', name: '二组', sortOrder: 2000 },
      { id: 'third', name: '三组', sortOrder: 3000 },
    ];

    expect(resolveGroupDropSortOrder(groups, 'third', 'second', 'before')).toBe(1500);
    expect(resolveGroupDropSortOrder(groups, 'first', 'third', 'after')).toBe(4000);
    expect(resolveGroupDropSortOrder(groups, 'first', 'first', 'before')).toBeNull();
  });

  it('allows interactive password and agent while requiring a bound private key', () => {
    expect(isConnectionReady(connection(1, null, null))).toBe(false);
    expect(isConnectionReady({ ...connection(2, null, null), authentication: 'password' })).toBe(
      true
    );
    expect(
      isConnectionReady({
        ...connection(2, null, null),
        authentication: 'password',
        credentialKind: 'password',
        credentialStatus: 'metadata_only',
      })
    ).toBe(true);
    expect(isConnectionReady({ ...connection(2, null, null), authentication: 'ssh_agent' })).toBe(
      false
    );
    expect(
      isConnectionReady({
        ...connection(2, null, null),
        authentication: 'ssh_agent',
        credentialKind: 'ssh_agent',
        credentialStatus: 'bound',
      })
    ).toBe(true);
    expect(
      isConnectionReady({
        ...connection(3, null, null),
        credentialKind: 'private_key',
        credentialStatus: 'bound',
      })
    ).toBe(true);
  });

  it('separates incomplete, offline, connecting, connected and failed indicators', () => {
    const incomplete = connection(6, null, null);
    const ready = markCredentialBound(incomplete, 'private_key');

    expect(resolveConnectionIndicator(incomplete)).toBe('readiness');
    expect(resolveConnectionIndicator(ready)).toBe('offline');
    expect(resolveConnectionIndicator(ready, 'closed')).toBe('offline');
    expect(resolveConnectionIndicator(ready, 'connecting')).toBeNull();
    expect(resolveConnectionIndicator(ready, 'connected')).toBe('connected');
    expect(resolveConnectionIndicator(ready, 'error', '连接失败')).toBe('error');
  });

  it('marks a newly stored credential using the IPC field names', () => {
    const bound = markCredentialBound(connection(4, null, null), 'private_key');

    expect(bound.credentialKind).toBe('private_key');
    expect(bound.credentialStatus).toBe('bound');
    expect(isConnectionReady(bound)).toBe(true);
  });

  it('requires reconnect only when the SSH target or startup path changes', () => {
    const current = connection(5, null, null);
    expect(
      connectionRequiresReconnect(current, {
        host: current.host,
        port: current.port,
        username: current.username,
        authentication: current.authentication,
        remoteInitialPath: current.remoteInitialPath ?? undefined,
      })
    ).toBe(false);
    expect(
      connectionRequiresReconnect(current, {
        host: 'other.example.com',
        port: current.port,
        username: current.username,
        authentication: current.authentication,
        remoteInitialPath: current.remoteInitialPath ?? undefined,
      })
    ).toBe(true);
  });
});
