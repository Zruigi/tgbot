import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { resolve } from 'path'

// Detect Cloudflare tunnel mode via env var (set CF_TUNNEL=1 in tunnel launch script)
const isCfTunnel = !!process.env['CF_TUNNEL']

export default defineConfig({
  plugins: [vue()],
  resolve: {
    alias: { '@': resolve(__dirname, 'src') }
  },
  server: {
    port: 5173,
    host: '0.0.0.0',
    // Allow all hosts including *.trycloudflare.com
    allowedHosts: true,
    hmr: {
      // Cloudflare tunnel: browser must use port 443 (tunnel's public HTTPS port)
      protocol: isCfTunnel ? 'wss' : 'ws',
      clientPort: isCfTunnel ? 443 : 5173,
    },
    proxy: {
      '/api/v1': {
        target: 'http://localhost:8080',
        changeOrigin: true,
      },
      '/api': {
        target: 'http://localhost:8080',
        changeOrigin: true,
      },
      '/ws': {
        target: 'ws://127.0.0.1:8080',
        ws: true,
        // Rewrite Origin header so Cloudflare tunnel origins pass the WS handshake
        // rewriteWsOrigin is available since Vite 5.1
        ...({
          rewriteWsOrigin: true,
        } as Record<string, unknown>),
      },
    },
  },
  build: {
    outDir: '../static/dist',
    emptyOutDir: true,
  },
})
