import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { resolve } from 'node:path';

const host = process.env.TAURI_DEV_HOST;

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  // Tauri 生产构建使用相对路径，嵌入 exe 后资源才能正确加载
  base: process.env.TAURI_ENV_PLATFORM ? './' : '/',
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? { protocol: 'ws', host, port: 1421 }
      : undefined,
    watch: {
      ignored: ['**/src-tauri/**'],
    },
  },
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    target: process.env.TAURI_ENV_PLATFORM === 'windows' ? 'chrome105' : 'safari13',
    minify: !process.env.TAURI_ENV_DEBUG ? 'esbuild' : false,
    sourcemap: Boolean(process.env.TAURI_ENV_DEBUG),
    // 精灵 41 张 WebP 必须是离散文件（SCR-006 D04 与 Tauri 打包都按文件核对）。
    // v1.3.2 纯缩放后部分 poster 小于 Vite 默认 4096 B 内联阈值（实测
    // su-stretch.webp 4032 B），会被转成 data URL 导致 dist 只有 40 张。
    // 这里对精灵素材禁用内联，其它资源仍走默认策略。
    assetsInlineLimit: (filePath: string) =>
      filePath.replace(/\\/g, '/').includes('/assets/pet/') ? false : undefined,
    // 主窗口 + 流量悬浮窗两个入口
    rollupOptions: {
      input: {
        main: resolve(__dirname, 'index.html'),
        overlay: resolve(__dirname, 'overlay.html'),
      },
    },
  },
});
