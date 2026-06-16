import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { viteSingleFile } from 'vite-plugin-singlefile'

export default defineConfig({
  root: 'src',
  plugins: [
    react(),
    viteSingleFile(),
  ],
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