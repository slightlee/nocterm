import { useCallback, useEffect, useMemo, useRef, useState, type PointerEvent } from 'react';

import { type ConnectionGroupView, UNGROUPED_CONNECTION_GROUP_ID } from './connection-list-model';
import type { ConnectionProfile } from '../types/connection-types';

export type ConnectionDropPosition = 'before' | 'after' | 'end';

export interface ConnectionDropTarget {
  groupId: string;
  connectionId?: number;
  position: ConnectionDropPosition;
}

interface ConnectionDragState {
  connectionId: number;
  started: boolean;
  startX: number;
  startY: number;
}

interface UseConnectionDragSortOptions {
  groups: ConnectionGroupView[];
  onMove: (connectionId: number, groupId: string | null, sortOrder: number) => Promise<void>;
  onError: (error: unknown) => void;
}

const DRAG_START_THRESHOLD = 4;

/** 连接没有显式排序时也分配稳定间隔，保证插入前后能生成新的排序权重。 */
function getSortOrder(connection: ConnectionProfile, fallbackIndex: number) {
  return connection.sortOrder ?? fallbackIndex * 1000;
}

/** 根据落点前后的连接计算排序权重，分组归属由调用方单独持久化。 */
export function resolveConnectionDropSortOrder(
  connections: ConnectionProfile[],
  draggingId: number,
  targetConnectionId: number | undefined,
  position: ConnectionDropPosition
) {
  const ordered = connections.filter((connection) => connection.id !== draggingId);
  let insertionIndex = ordered.length;
  if (targetConnectionId !== undefined && position !== 'end') {
    const targetIndex = ordered.findIndex((connection) => connection.id === targetConnectionId);
    if (targetIndex >= 0) insertionIndex = position === 'before' ? targetIndex : targetIndex + 1;
  }
  const previous = ordered[insertionIndex - 1];
  const next = ordered[insertionIndex];
  const previousOrder = previous ? getSortOrder(previous, insertionIndex - 1) : null;
  const nextOrder = next ? getSortOrder(next, insertionIndex) : null;
  if (previousOrder !== null && nextOrder !== null) return (previousOrder + nextOrder) / 2;
  if (previousOrder !== null) return previousOrder + 1000;
  if (nextOrder !== null) return nextOrder - 1000;
  return 0;
}

/**
 * WKWebView 对原生 DragEvent 的跨分组投放不稳定，因此统一以 Pointer Events 根据坐标识别目标。
 * 拖拽完成后只调用一次 onMove，避免 hover 过程误写入 SQLite。
 */
export function useConnectionDragSort({ groups, onMove, onError }: UseConnectionDragSortOptions) {
  const [dragState, setDragState] = useState<ConnectionDragState | null>(null);
  const [dropTarget, setDropTarget] = useState<ConnectionDropTarget | null>(null);
  const dragRef = useRef<ConnectionDragState | null>(null);
  const targetRef = useRef<ConnectionDropTarget | null>(null);
  const onMoveRef = useRef(onMove);
  const onErrorRef = useRef(onError);
  const suppressClickRef = useRef(false);

  useEffect(() => {
    onMoveRef.current = onMove;
    onErrorRef.current = onError;
  }, [onError, onMove]);

  const visibleGroups = useMemo(() => {
    if (!dragState?.started || groups.some((group) => group.id === UNGROUPED_CONNECTION_GROUP_ID)) {
      return groups;
    }
    return [
      {
        id: UNGROUPED_CONNECTION_GROUP_ID,
        name: '未分组',
        connections: [],
      },
      ...groups,
    ];
  }, [dragState?.started, groups]);

  const setCurrentTarget = useCallback((target: ConnectionDropTarget | null) => {
    targetRef.current = target;
    setDropTarget(target);
  }, []);

  const beginDrag = useCallback(
    (connectionId: number, event: PointerEvent<HTMLButtonElement>) => {
      if (event.button !== 0) return;
      const next = {
        connectionId,
        started: false,
        startX: event.clientX,
        startY: event.clientY,
      };
      dragRef.current = next;
      setDragState(next);
      setCurrentTarget(null);
    },
    [setCurrentTarget]
  );

  const shouldSuppressClick = useCallback(() => {
    if (!suppressClickRef.current) return false;
    suppressClickRef.current = false;
    return true;
  }, []);

  useEffect(() => {
    if (!dragState) return;

    const updateTargetFromPoint = (clientX: number, clientY: number) => {
      const drag = dragRef.current;
      if (!drag) return;
      const element = document.elementFromPoint(clientX, clientY);
      if (!(element instanceof HTMLElement)) {
        setCurrentTarget(null);
        return;
      }
      const item = element.closest<HTMLElement>('[data-connection-id]');
      if (item) {
        const connectionId = Number(item.dataset.connectionId);
        if (!Number.isInteger(connectionId) || connectionId === drag.connectionId) {
          setCurrentTarget(null);
          return;
        }
        const groupId = item.closest<HTMLElement>('[data-connection-group-id]')?.dataset
          .connectionGroupId;
        if (!groupId) {
          setCurrentTarget(null);
          return;
        }
        const rect = item.getBoundingClientRect();
        setCurrentTarget({
          groupId,
          connectionId,
          position: clientY < rect.top + rect.height / 2 ? 'before' : 'after',
        });
        return;
      }
      const groupId = element.closest<HTMLElement>('[data-connection-group-id]')?.dataset
        .connectionGroupId;
      setCurrentTarget(groupId ? { groupId, position: 'end' } : null);
    };

    const finish = () => {
      dragRef.current = null;
      setDragState(null);
      setCurrentTarget(null);
    };

    const handlePointerMove = (event: globalThis.PointerEvent) => {
      const drag = dragRef.current;
      if (!drag) return;
      const distance = Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY);
      if (!drag.started && distance < DRAG_START_THRESHOLD) return;
      event.preventDefault();
      if (!drag.started) {
        const started = { ...drag, started: true };
        dragRef.current = started;
        setDragState(started);
      }
      updateTargetFromPoint(event.clientX, event.clientY);
    };

    const handlePointerUp = (event: globalThis.PointerEvent) => {
      const drag = dragRef.current;
      if (!drag) return;
      if (drag.started) updateTargetFromPoint(event.clientX, event.clientY);
      const target = targetRef.current;
      if (!drag.started || !target) {
        if (drag.started) suppressClickRef.current = true;
        finish();
        return;
      }
      const targetGroup = visibleGroups.find((group) => group.id === target.groupId);
      const sortOrder = targetGroup
        ? resolveConnectionDropSortOrder(
            targetGroup.connections,
            drag.connectionId,
            target.connectionId,
            target.position
          )
        : null;
      suppressClickRef.current = true;
      finish();
      if (sortOrder === null) return;
      void onMoveRef
        .current(
          drag.connectionId,
          target.groupId === UNGROUPED_CONNECTION_GROUP_ID ? null : target.groupId,
          sortOrder
        )
        .catch(onErrorRef.current);
    };

    window.addEventListener('pointermove', handlePointerMove, { passive: false });
    window.addEventListener('pointerup', handlePointerUp, { once: true });
    window.addEventListener('pointercancel', handlePointerUp, { once: true });
    return () => {
      window.removeEventListener('pointermove', handlePointerMove);
      window.removeEventListener('pointerup', handlePointerUp);
      window.removeEventListener('pointercancel', handlePointerUp);
    };
  }, [dragState, setCurrentTarget, visibleGroups]);

  return {
    beginDrag,
    draggingConnectionId: dragState?.started ? dragState.connectionId : null,
    dropTarget,
    shouldSuppressClick,
    visibleGroups,
  };
}
