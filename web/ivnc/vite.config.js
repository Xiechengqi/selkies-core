import { defineConfig } from 'vite';
import envCompatible from 'vite-plugin-env-compatible';
import { ViteMinifyPlugin } from 'vite-plugin-minify';
import ViteRestart from 'vite-plugin-restart'

export default defineConfig({
  base: '',
  server: {
    host: '0.0.0.0',
    allowedHosts: true,
  },
  plugins: [
    envCompatible(),
    ViteMinifyPlugin(),
    ViteRestart({restart: ['ivnc-core.js', 'lib/**','ivnc-version.txt']}),
  ],
  build: {
    target: 'chrome94',
    rollupOptions: {
      input: {
        main: './index.html',
      },
      output: {
        entryFileNames: 'ivnc-core.js'
      }
    }
  },
})
