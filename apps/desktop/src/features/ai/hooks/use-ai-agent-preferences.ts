import { useSyncExternalStore } from 'react';

import { getAiAgentPreferences, subscribeAiAgentPreferences } from '../api/ai-agent-preferences';
import { createAiAgentPreferences } from '../model/ai-agent-preferences';

const SERVER_SNAPSHOT = createAiAgentPreferences();

export function useAiAgentPreferences() {
  return useSyncExternalStore(
    subscribeAiAgentPreferences,
    getAiAgentPreferences,
    () => SERVER_SNAPSHOT
  );
}
