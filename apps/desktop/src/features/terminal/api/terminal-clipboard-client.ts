import {
  readText as readNativeClipboardText,
  writeText as writeNativeClipboardText,
} from '@tauri-apps/plugin-clipboard-manager';

/** 只在明确的右键粘贴动作中读取文本，不缓存、记录或转发剪贴板内容。 */
export function readTerminalClipboardText(): Promise<string> {
  return readNativeClipboardText();
}

/** 原生写入失败转换为稳定布尔结果，由终端界面统一提示并保留选区。 */
export async function writeTerminalClipboardText(text: string): Promise<boolean> {
  try {
    await writeNativeClipboardText(text);
    return true;
  } catch {
    return false;
  }
}
