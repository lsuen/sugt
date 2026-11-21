import { defineConfig, type UserConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { resolve } from 'node:path';
import { existsSync, renameSync } from 'node:fs';

const host = process.env.TAURI_DEV_HOST;

// https://vitejs.dev/config/
export default defineConfig(({ mode }): UserConfig => {
  // 局域网 WebUI：独立入口，产物直接落入 CLI 嵌入式资源目录。
  // 只影响 `vite build --mode web`，下面桌面三入口构建保持原样。
  if (mode === 'web') {
    return {
      plugins: [
        react(),
        // axum 静态层固定查 index.html：把 web.html 入口产物改名，避免根路径 404。
        {
          name: 'webui-entry-index',
          closeBundle() {
            const out = resolve(__dirname, 'src-tauri/webui-dist');
            const from = resolve(out, 'web.html');
            if (existsSync(from)) renameSync(from, resolve(out, 'index.html'));
          },
        },
      ],
      clearScreen: false,
      // WebUI 由独立 axum 服务挂在根路径，必须用绝对路径
      base: '/',
      build: {
        outDir: 'src-tauri/webui-dist',
        emptyOutDir: true,
        // 与桌面构建一致：精灵 WebP 不内联，避免 dist 少文件
        assetsInlineLimit: (filePath: string) =>
          filePath.replace(/\\/g, '/').includes('/assets/pet/') ? false : undefined,
        rollupOptions: {
          input: {
            web: resolve(__dirname, 'web.html'),
          },
        },
      },
    };
  }

  return {
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
      // 精灵 51 张 WebP 必须是离散文件（v1.4 为 49 张、v1.3.2 为 41 张，均属历史；SCR-006 D04 与 Tauri 打包都按文件核对）。
      // v1.3.2 纯缩放后部分 poster 小于 Vite 默认 4096 B 内联阈值（实测
      // su-stretch.webp 4032 B），会被转成 data URL 导致该时期 dist 少一张。
      // 这里对精灵素材禁用内联，其它资源仍走默认策略。
      assetsInlineLimit: (filePath: string) =>
        filePath.replace(/\\/g, '/').includes('/assets/pet/') ? false : undefined,
      // 主窗口、精灵透明窗、精灵右键菜单三入口
      rollupOptions: {
        input: {
          main: resolve(__dirname, 'index.html'),
          overlay: resolve(__dirname, 'overlay.html'),
          menu: resolve(__dirname, 'menu.html'),
        },
      },
    },
  };
});
