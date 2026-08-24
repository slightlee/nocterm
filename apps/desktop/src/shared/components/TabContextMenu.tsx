import type { ReactNode } from 'react';

import { ContextMenu } from './ContextMenu';
import { resolveTabIdsToClose, type TabCloseAction } from '../lib/tab-close-actions';

interface TabContextMenuProps<T> {
  ids: readonly T[];
  targetId: T;
  onClose: (ids: T[]) => void;
  children: ReactNode;
}

/** 统一终端与文件页的标签关闭菜单，避免两套交互和文案逐渐漂移。 */
export function TabContextMenu<T>({ ids, targetId, onClose, children }: TabContextMenuProps<T>) {
  const close = (action: TabCloseAction) => onClose(resolveTabIdsToClose(ids, targetId, action));

  return (
    <ContextMenu
      compact
      items={[
        { label: '关闭当前标签', onSelect: () => close('current') },
        {
          label: '关闭其他标签',
          disabled: ids.length <= 1,
          onSelect: () => close('others'),
        },
        'separator',
        { label: '关闭全部标签', onSelect: () => close('all') },
      ]}
    >
      {children}
    </ContextMenu>
  );
}
