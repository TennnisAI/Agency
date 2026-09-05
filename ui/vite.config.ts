import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";

const NOTICES = fileURLToPath(new URL("../THIRD-PARTY.md", import.meta.url));

/**
 * Ship THIRD-PARTY.md inside the app, where the licences it reproduces have to
 * be: MIT wants its notice in every copy, and the .app is a copy. Settings ▸
 * About fetches it at `/THIRD-PARTY.md`.
 *
 * It is emitted rather than kept in `ui/public/`, because a second tracked copy
 * of a 650 KB generated file is a second copy to fall out of date, and the CI
 * check only regenerates the one at the repo root. The dev middleware exists so
 * the About panel is not a 404 under `./dev.sh`, which is where it gets looked
 * at.
 */
function thirdPartyNotices(): Plugin {
  return {
    name: "agency-third-party-notices",
    configureServer(server) {
      server.middlewares.use("/THIRD-PARTY.md", (_req, res) => {
        res.setHeader("Content-Type", "text/markdown; charset=utf-8");
        res.end(readFileSync(NOTICES));
      });
    },
    generateBundle() {
      this.emitFile({
        type: "asset",
        fileName: "THIRD-PARTY.md",
        source: readFileSync(NOTICES, "utf8"),
      });
    },
  };
}

export default defineConfig({
  plugins: [react(), thirdPartyNotices()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  // Explicit rather than relying on the default: a release bundle must never
  // carry sourcemaps, which would ship readable frontend source inside the .app.
  build: { sourcemap: false },
});
