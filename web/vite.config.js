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
  },
})