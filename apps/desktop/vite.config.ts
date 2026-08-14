import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

const devHost = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    host: devHost || '127.0.0.1',
    port: 1420,
    strictPort: true,
    hmr: devHost
      ? {
          protocol: 'ws',
          host: devHost,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ['**/src-tauri/**'],
    },
  },
});
