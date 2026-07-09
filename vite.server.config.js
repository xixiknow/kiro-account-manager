import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import path from 'path'
import fs from 'fs'
import pkg from './package.json'

const serverTauri = (file) => path.resolve(__dirname, `./src/server/tauri/${file}`)
const reactAriaExport = (id) => path.resolve(__dirname, `./node_modules/react-aria/dist/exports/${id}.js`)

function reactAriaSubpathResolver() {
  return {
    name: 'react-aria-subpath-resolver',
    resolveId(id) {
      if (!id.startsWith('react-aria/')) return null
      const subpath = id.slice('react-aria/'.length)
      const candidate = reactAriaExport(subpath)
      return fs.existsSync(candidate) ? candidate : null
    },
  }
}

export default defineConfig({
  root: path.resolve(__dirname, './server-web-src'),
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
      '@tauri-apps/api/core': serverTauri('core.ts'),
      '@tauri-apps/api/event': serverTauri('event.ts'),
      '@tauri-apps/api/app': serverTauri('app.ts'),
      '@tauri-apps/api/window': serverTauri('window.ts'),
      '@tauri-apps/api/path': serverTauri('path.ts'),
      '@tauri-apps/plugin-dialog': serverTauri('plugin-dialog.ts'),
      '@tauri-apps/plugin-fs': serverTauri('plugin-fs.ts'),
      '@tauri-apps/plugin-opener': serverTauri('plugin-opener.ts'),
      '@tauri-apps/plugin-process': serverTauri('plugin-process.ts'),
      '@tauri-apps/plugin-shell': serverTauri('plugin-shell.ts'),
      '@tauri-apps/plugin-updater': serverTauri('plugin-updater.ts'),
    },
  },
  plugins: [reactAriaSubpathResolver(), react(), tailwindcss()],
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
    __KAM_SERVER_WEB__: JSON.stringify(true),
  },
  clearScreen: false,
  envPrefix: ['VITE_', 'KAM_'],
  build: {
    outDir: path.resolve(__dirname, 'src-tauri/server-web-dist'),
    emptyOutDir: true,
    target: 'es2020',
    minify: 'terser',
    sourcemap: false,
    reportCompressedSize: false,
    rollupOptions: {
      input: path.resolve(__dirname, 'server-web-src/index.html'),
      output: {
        manualChunks: {
          vendor: ['react', 'react-dom'],
          icons: ['lucide-react'],
          i18n: ['i18next', 'react-i18next'],
        },
      },
    },
  },
  optimizeDeps: {
    exclude: [
      '@tauri-apps/api',
      '@tauri-apps/plugin-dialog',
      '@tauri-apps/plugin-fs',
      '@tauri-apps/plugin-opener',
      '@tauri-apps/plugin-process',
      '@tauri-apps/plugin-shell',
      '@tauri-apps/plugin-updater',
    ],
  },
  server: {
    fs: {
      allow: [__dirname],
    },
  },
})
