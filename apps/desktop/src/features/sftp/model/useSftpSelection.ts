import { useCallback, useEffect, useRef, useState } from 'react';
import type { FileRow } from '../types/sftp-types';
import { selectRange, summarizeSelection } from './sftp-view-model';

type SelectionSummary = {
  scope: 'local' | 'remote' | null;
  count: number;
  totalSize: number | null;
};

interface UseSftpSelectionOptions {
  localRows: FileRow[];
  remoteRows: FileRow[];
  setSelectionSummary: (summary: SelectionSummary) => void;
}

export function useSftpSelection({
  localRows,
  remoteRows,
  setSelectionSummary,
}: UseSftpSelectionOptions) {
  const [localSelectedNames, setLocalSelectedNames] = useState<Set<string>>(new Set());
  const [remoteSelectedNames, setRemoteSelectedNames] = useState<Set<string>>(new Set());
  const lastLocalSelectedRef = useRef<string | null>(null);
  const lastRemoteSelectedRef = useRef<string | null>(null);

  useEffect(() => {
    const remoteSummary = summarizeSelection(remoteRows, remoteSelectedNames);
    if (remoteSummary.count > 0) {
      setSelectionSummary({ scope: 'remote', ...remoteSummary });
      return;
    }

    const localSummary = summarizeSelection(localRows, localSelectedNames);
    if (localSummary.count > 0) {
      setSelectionSummary({ scope: 'local', ...localSummary });
      return;
    }

    setSelectionSummary({ scope: null, count: 0, totalSize: null });
  }, [localRows, localSelectedNames, remoteRows, remoteSelectedNames, setSelectionSummary]);

  const resetLocalSelection = useCallback(() => {
    setLocalSelectedNames(new Set());
    lastLocalSelectedRef.current = null;
  }, []);

  const resetRemoteSelection = useCallback(() => {
    setRemoteSelectedNames(new Set());
    lastRemoteSelectedRef.current = null;
  }, []);

  const selectLocal = useCallback(
    (name: string, additive: boolean, range: boolean) => {
      setLocalSelectedNames((current) => {
        if (range) {
          const ranged = selectRange(localRows, lastLocalSelectedRef.current, name);
          if (ranged) {
            if (!additive) return ranged;
            const next = new Set(current);
            for (const item of ranged) next.add(item);
            return next;
          }
        }
        lastLocalSelectedRef.current = name;
        if (!additive) return new Set([name]);
        const next = new Set(current);
        if (next.has(name)) next.delete(name);
        else next.add(name);
        return next;
      });
    },
    [localRows]
  );

  const selectRemote = useCallback(
    (name: string, additive: boolean, range: boolean) => {
      setRemoteSelectedNames((current) => {
        if (range) {
          const ranged = selectRange(remoteRows, lastRemoteSelectedRef.current, name);
          if (ranged) {
            if (!additive) return ranged;
            const next = new Set(current);
            for (const item of ranged) next.add(item);
            return next;
          }
        }
        lastRemoteSelectedRef.current = name;
        if (!additive) return new Set([name]);
        const next = new Set(current);
        if (next.has(name)) next.delete(name);
        else next.add(name);
        return next;
      });
    },
    [remoteRows]
  );

  const selectAllLocal = useCallback(() => {
    setLocalSelectedNames(new Set(localRows.map((row) => row.name)));
    lastLocalSelectedRef.current = localRows[localRows.length - 1]?.name ?? null;
  }, [localRows]);

  const selectAllRemote = useCallback(() => {
    setRemoteSelectedNames(new Set(remoteRows.map((row) => row.name)));
    lastRemoteSelectedRef.current = remoteRows[remoteRows.length - 1]?.name ?? null;
  }, [remoteRows]);

  return {
    localSelectedNames,
    remoteSelectedNames,
    selectLocal,
    selectRemote,
    selectAllLocal,
    selectAllRemote,
    resetLocalSelection,
    resetRemoteSelection,
  };
}
