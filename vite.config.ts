import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import path from 'node:path'

// Tauri 期望固定端口，且不应清屏（否则会盖住 Rust 侧日志）
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: { '@': path.resolve(__dirname, './src') },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: false,
    watch: {
      // 别让 Vite 监听 Rust 侧，否则 cargo 编译会触发前端热重载风暴
      ignored: ['**/src-tauri/**'],
    },
  },
  build: {
    // Windows 上 WebView2 已经是现代内核，不需要兼容老浏览器
    target: 'chrome105',
    minify: 'esbuild',
    sourcemap: false,
  },
})
