import { describe, expect, it } from 'vitest';

import { resolveConnectionDropSortOrder } from './use-connection-drag-sort';
import type { ConnectionProfile } from '../types/connection-types';

function connection(id: number, sortOrder: number | null): ConnectionProfile {
  return {
    id,
    name: `连接 ${id}`,
    host: '127.0.0.1',
    port: 22,
    username: 'tester',
    authentication: 'password',
    createdAt: 0,
    updatedAt: 0,
    sortOrder,
  };
}

describe('connection drag sort', () => {
  it('calculates an insertion order for another group without retaining its source group order', () => {
    const targetGroup = [connection(2, 1000), connection(3, 2000)];

    expect(resolveConnectionDropSortOrder(targetGroup, 1, 2, 'before')).toBe(0);
    expect(resolveConnectionDropSortOrder(targetGroup, 1, 2, 'after')).toBe(1500);
    expect(resolveConnectionDropSortOrder(targetGroup, 1, undefined, 'end')).toBe(3000);
  });

  it('handles an empty target group with a deterministic first sort order', () => {
    expect(resolveConnectionDropSortOrder([], 1, undefined, 'end')).toBe(0);
  });
});
