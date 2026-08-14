import { useEffect, useMemo, useState } from 'react';

import { checkRuntimeHealth, isDesktopRuntime, onRuntimeHealth } from '../api/runtime-client';
import { runtimeStatusLabel } from './runtime-status';
import type { RuntimeHealth, RuntimeStatus } from './runtime-types';

interface RuntimeHealthState {
  status: RuntimeStatus;
  health: RuntimeHealth | null;
  error: string | null;
  eventCount: number;
}

const initialState: RuntimeHealthState = {
  status: 'checking',
  health: null,
  error: null,
  eventCount: 0,
};

export function useRuntimeHealth() {
  const [state, setState] = useState<RuntimeHealthState>(() =>
    isDesktopRuntime() ? initialState : { ...initialState, status: 'preview' }
  );

  useEffect(() => {
    if (!isDesktopRuntime()) {
      return;
    }

    let disposed = false;
    let unsubscribe: (() => void) | undefined;

    void onRuntimeHealth((health) => {
      if (disposed) return;
      setState((current) => ({
        ...current,
        status: 'ready',
        health,
        eventCount: current.eventCount + 1,
      }));
    })
      .then((unlisten) => {
        if (disposed) {
          unlisten();
          return;
        }
        unsubscribe = unlisten;
        return checkRuntimeHealth();
      })
      .then((health) => {
        if (!disposed && health) {
          setState((current) => ({ ...current, status: 'ready', health }));
        }
      })
      .catch((error: unknown) => {
        if (disposed) return;
        setState((current) => ({
          ...current,
          status: 'error',
          error: error instanceof Error ? error.message : String(error),
        }));
      });

    return () => {
      disposed = true;
      unsubscribe?.();
    };
  }, []);

  return useMemo(() => ({ ...state, label: runtimeStatusLabel(state.status) }), [state]);
}
