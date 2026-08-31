import { createContext } from 'react';

import type { AppTheme, TerminalAppearance } from '../types/settings-types';

export interface SettingsContextValue {
  appTheme: AppTheme;
  terminalAppearance: TerminalAppearance;
  loading: boolean;
  saving: boolean;
  persistenceAvailable: boolean;
  error: string | null;
  updateAppTheme: (theme: AppTheme) => Promise<void>;
  updateTerminalAppearance: (appearance: TerminalAppearance) => Promise<void>;
}

export const SettingsContext = createContext<SettingsContextValue | null>(null);
