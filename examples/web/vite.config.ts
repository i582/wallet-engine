import path from "node:path"

import tailwindcss from "@tailwindcss/vite"
import react from "@vitejs/plugin-react"
import {defineConfig, type PluginOption} from "vite"

export default defineConfig({
  plugins: [tailwindcss(), react() as PluginOption],
  resolve: {
    alias: {
      "@": path.resolve(import.meta.dirname, "src"),
    },
  },
  server: {
    port: 3000,
  },
})
