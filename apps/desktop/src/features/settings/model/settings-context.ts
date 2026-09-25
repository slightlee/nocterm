import { createContext } from 'react';

import type { AppTheme, TerminalAppearance } from '../types/settings-types';
import type { ResolvedTerminalTheme } from './terminal-appearance';

export interface SettingsContextValue {
  appTheme: AppTheme;
  terminalAppearance: TerminalAppearance;
  /** 已解析的终端配色 id（follow_app 已按系统深浅落地），终端工作区与调色板快照共用。 */
  terminalThemeId: ResolvedTerminalTheme;
  loading: boolean;
  saving: boolean;
  persistenceAvailable: boolean;
  error: string | null;
  updateAppTheme: (theme: AppTheme) => Promise<void>;
  updateTerminalAppearance: (appearance: TerminalAppearance) => Promise<void>;
}

export const SettingsContext = createContext<SettingsContextValue | null>(null);
