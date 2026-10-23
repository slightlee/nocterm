import { readText, writeText } from '@tauri-apps/plugin-clipboard-manager';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { readTerminalClipboardText, writeTerminalClipboardText } from './terminal-clipboard-client';

vi.mock('@tauri-apps/plugin-clipboard-manager', () => ({
  readText: vi.fn(),
  writeText: vi.fn(),
}));

describe('terminal clipboard client', () => {
  beforeEach(() => vi.clearAllMocks());

  it('returns text read by the native clipboard plugin', async () => {
    vi.mocked(readText).mockResolvedValue('clipboard text');

    await expect(readTerminalClipboardText()).resolves.toBe('clipboard text');
    expect(readText).toHaveBeenCalledOnce();
  });

  it('reports native write success without exposing clipboard content elsewhere', async () => {
    vi.mocked(writeText).mockResolvedValue(undefined);

    await expect(writeTerminalClipboardText('selected')).resolves.toBe(true);
    expect(writeText).toHaveBeenCalledWith('selected');
  });

  it('maps native write failures to false for the terminal error path', async () => {
    vi.mocked(writeText).mockRejectedValue(new Error('permission denied'));

    await expect(writeTerminalClipboardText('selected')).resolves.toBe(false);
  });
});
