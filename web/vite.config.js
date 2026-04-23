import { defineConfig } from 'vite'
import { viteSingleFile } from 'vite-plugin-singlefile'

export default defineConfig({
  root: 'src',
  plugins: [viteSingleFile()],
  build: {
    outDir: '../dist',
    emptyOutDir: true,
    cssCodeSplit: false,
    assetsInlineLimit: 4096 * 1024,
    minify: false,
    sourcemap: true,
    rollupOptions: {
      output: {
        manualChunks: undefined,
        codeSplitting: false,
      },
    },
  },
  server: {
    open: true,
    host: '0.0.0.0',
    // 关键：添加这些配置解决缓存问题
    cors: true,

    // 自定义中间件禁用缓存
    middlewareMode: false,

    // 配置响应头禁用缓存
    headers: {
      'Cache-Control': 'no-cache, no-store, must-revalidate',
      'Pragma': 'no-cache',
      'Expires': '0'
    },

    // 强制禁用 HMR 缓存
    hmr: {
      overlay: false
    }
  },

  // 开发服务器配置
  optimizeDeps: {
    force: true,  // 强制重新预构建依赖
  },
})