import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from 'react';

import type { GroupDropPosition } from './connection-list-model';

interface GroupDragState {
  id: string;
  startX: number;
  startY: number;
  started: boolean;
}

interface GroupDropTarget {
  id: string;
  position: GroupDropPosition;
}

interface UseGroupDragSortOptions {
  onMove: (sourceId: string, targetId: string, position: GroupDropPosition) => Promise<void>;
  onError: (error: unknown) => void;
}

const DRAG_START_THRESHOLD = 4;

/** Pointer Events 在 WKWebView 中比原生 HTML DragEvent 稳定，并与旧客户端拖拽手感一致。 */
export function useGroupDragSort({ onMove, onError }: UseGroupDragSortOptions) {
  const dragRef = useRef<GroupDragState | null>(null);
  const targetRef = useRef<GroupDropTarget | null>(null);
  const moveRef = useRef(onMove);
  const errorRef = useRef(onError);
  const suppressClickRef = useRef(false);
  const [draggingGroupId, setDraggingGroupId] = useState<string | null>(null);
  const [dropTarget, setDropTarget] = useState<GroupDropTarget | null>(null);

  useEffect(() => {
    moveRef.current = onMove;
    errorRef.current = onError;
  }, [onError, onMove]);

  const setCurrentTarget = useCallback((target: GroupDropTarget | null) => {
    targetRef.current = target;
    setDropTarget(target);
  }, []);

  useEffect(() => {
    const updateTargetFromPoint = (clientX: number, clientY: number) => {
      const drag = dragRef.current;
      if (!drag) return;
      const element = document.elementFromPoint(clientX, clientY);
      const group =
        element instanceof HTMLElement
          ? element.closest<HTMLElement>('[data-connection-group-id]')
          : null;
      const targetId = group?.dataset.connectionGroupId;
      if (!group || !targetId || targetId === drag.id || targetId === '__ungrouped__') {
        setCurrentTarget(null);
        return;
      }

      const handle = group.querySelector<HTMLElement>('[data-group-drag-handle]');
      const bounds = (handle ?? group).getBoundingClientRect();
      setCurrentTarget({
        id: targetId,
        position: clientY < bounds.top + bounds.height / 2 ? 'before' : 'after',
      });
    };

    const handlePointerMove = (event: PointerEvent) => {
      const drag = dragRef.current;
      if (!drag) return;
      const distance = Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY);
      if (!drag.started && distance < DRAG_START_THRESHOLD) return;

      event.preventDefault();
      if (!drag.started) {
        drag.started = true;
        setDraggingGroupId(drag.id);
      }
      updateTargetFromPoint(event.clientX, event.clientY);
    };

    const finishDrag = (event: PointerEvent) => {
      const drag = dragRef.current;
      if (drag?.started) updateTargetFromPoint(event.clientX, event.clientY);
      const target = targetRef.current;
      dragRef.current = null;
      setDraggingGroupId(null);
      setCurrentTarget(null);
      if (!drag?.started) return;

      suppressClickRef.current = true;
      if (target) {
        void moveRef.current(drag.id, target.id, target.position).catch(errorRef.current);
      }
    };

    const cancelDrag = () => {
      const drag = dragRef.current;
      dragRef.current = null;
      setDraggingGroupId(null);
      setCurrentTarget(null);
      if (drag?.started) suppressClickRef.current = true;
    };

    window.addEventListener('pointermove', handlePointerMove, { passive: false });
    window.addEventListener('pointerup', finishDrag);
    window.addEventListener('pointercancel', cancelDrag);
    return () => {
      window.removeEventListener('pointermove', handlePointerMove);
      window.removeEventListener('pointerup', finishDrag);
      window.removeEventListener('pointercancel', cancelDrag);
    };
  }, [setCurrentTarget]);

  const beginDrag = useCallback(
    (groupId: string, event: ReactPointerEvent<HTMLElement>) => {
      if (event.button !== 0 || (event.target as HTMLElement).closest('button, input')) return;
      dragRef.current = {
        id: groupId,
        startX: event.clientX,
        startY: event.clientY,
        started: false,
      };
      setCurrentTarget(null);
    },
    [setCurrentTarget]
  );

  const shouldSuppressClick = useCallback(() => {
    if (!suppressClickRef.current) return false;
    suppressClickRef.current = false;
    return true;
  }, []);

  return { beginDrag, draggingGroupId, dropTarget, shouldSuppressClick };
}
