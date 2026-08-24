import { describe, expect, it } from 'vitest';

import { resolveTabIdsToClose } from './tab-close-actions';

describe('resolveTabIdsToClose', () => {
  const ids = ['first', 'second', 'third'];

  it('closes only the context-menu target', () => {
    expect(resolveTabIdsToClose(ids, 'second', 'current')).toEqual(['second']);
  });

  it('closes every tab except the context-menu target', () => {
    expect(resolveTabIdsToClose(ids, 'second', 'others')).toEqual(['first', 'third']);
  });

  it('closes all tabs in their existing order', () => {
    expect(resolveTabIdsToClose(ids, 'second', 'all')).toEqual(ids);
  });
});
