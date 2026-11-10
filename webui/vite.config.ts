import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// The control plane binds loopback and validates Host plus same-origin JSON
// requests. The dev proxy therefore keeps the control plane's Host/Origin
// instead of forwarding the Vite origin.
const target = process.env.ASCOUT_WEB_TARGET ?? 'http://127.0.0.1:8080'
const origin = new URL(target).origin

export default defineConfig({
  plugins: [react()],
  base: '/',
  server: {
    port: 5173,
    proxy: {
      '/api': {
        target,
        changeOrigin: true,
        headers: { Origin: origin },
      },
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    sourcemap: false,
    rollupOptions: {
      output: {
        entryFileNames: 'assets/app.js',
        chunkFileNames: 'assets/[name].js',
        assetFileNames: (asset) =>
          asset.names?.some((name) => name.endsWith('.css'))
            ? 'assets/app.css'
            : 'assets/[name][extname]',
      },
    },
  },
})
