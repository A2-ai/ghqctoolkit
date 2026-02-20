import { defineConfig } from 'vite'
import tsconfigPaths from 'vite-tsconfig-paths'
import viteReact from '@vitejs/plugin-react'
import { tanstackStart } from '@tanstack/react-start/plugin/vite'

export default defineConfig({
  server: {
    port: 3103,
    proxy: {
      '/api': 'http://localhost:3104',
    },
  },
  plugins: [
    tsconfigPaths({
      projects: ['./tsconfig.json'],
    }),
    tanstackStart({
      srcDirectory: 'src',
    }),
    // React plugin must come after Start
    viteReact(),
  ],
})
