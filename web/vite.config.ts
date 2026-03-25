import { defineConfig, mergeConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { defineConfig as defineVitestConfig } from 'vitest/config'

const vitestConfig = defineVitestConfig({
  test: {
    environment: 'jsdom',
    globals: true,
  },
})

export default mergeConfig(
  defineConfig({
    plugins: [react()],
    server: {
      port: 5173,
      proxy: {
        '/api': 'http://localhost:3030',
      },
    },
  }),
  vitestConfig,
)
