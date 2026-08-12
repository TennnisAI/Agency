import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  // Explicit rather than relying on the default: a release bundle must never
  // carry sourcemaps, which would ship readable frontend source inside the .app.
  build: { sourcemap: false },
});
