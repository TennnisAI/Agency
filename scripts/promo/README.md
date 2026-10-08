# Promo capture

Records the site's screenshots and videos from the real Agency UI. The React
app runs from the Vite dev server in headless Chrome, and a mock of Tauri's
`invoke` and event channels stands in for the backend, serving fictional
projects (`orbit-api`, `lumen-web`, `tidepool`, `ledger-cli`) under
`/Users/demo`. Nothing here ships, and nothing here touches a real repo, a
daemon or a running app.

Agent output is scripted ANSI streamed over the attach channel, styled after
each CLI. It is not a recording of a real agent: one would put an account name
and real token spend on screen. Keep the scripts plausible, and name no
competitor product anywhere in them, legible in a frame or not.

## Run it

```sh
pnpm --dir scripts/promo install
pnpm --dir scripts/promo exec playwright-core install chromium   # once, or set CHROME
./dev.sh                                   # or just the Vite server on :1420

scripts/promo/run.sh                       # all scenes, about 8 minutes
scripts/promo/run.sh loop race             # or some
node scripts/promo/edit/render-cards.mjs   # title cards and scene frames
python3 scripts/promo/edit/build.py        # cut tour-master.mp4 and hero-master.mp4
scripts/promo/edit/publish.sh              # encode into site/assets/video, posters, og.jpg
python3 scripts/promo/edit/stills.py       # site screenshots from the stills (needs Pillow)
```

Everything lands in `out/`, which is ignored: `clips/` and `stills/` from the
recorder, then `tour-master.mp4` (2560x1440, for uploading elsewhere) and
`hero-master.mp4`. `KEEP_FRAMES=1` keeps each scene's PNG frames. Needs
`ffmpeg` with libx264 and libvpx, and `cwebp`.

A scene that throws exits the run non-zero and leaves `out/fail-<scene>.png`.
It does not encode a clip, so the previous one stays in place.

## How it works

| File | What |
|---|---|
| `boot.mjs` | Launches Chrome at 1600x1000, 2x, and installs the mocks before app code runs |
| `rec.mjs` | The frame-stepped recorder |
| `scenes.mjs` | One function per scene: clicks, typing, holds, stills |
| `mock/core.js` | `__TAURI_INTERNALS__`: `invoke`, callbacks, channels, events |
| `mock/data.js` | Projects, runs, issues, docs, settings, and most command handlers |
| `mock/git.js` | Diffs, history, commit and the merge flow |
| `mock/term.js` | The terminal engine: per-CLI styles, a transcript, a redrawn footer |
| `mock/scripts.js` | What each seeded run's agent has done and goes on to do |
| `mock/actions.js` | Runs created during a scene: dispatching an issue, a race, a loop |
| `edit/cards.html` | Title cards and caption frames, in the site's fonts and lane colours |

Time in the page moves only when the recorder moves it. Each frame advances
Playwright's fake clock by 1/30 s, seeks every live CSS animation by the same
amount, then screenshots, so motion is smooth however slow the capture runs.
Clicks are real Playwright clicks inside the page. The cursor is a drawn
element that glides to each target, because a headless browser has none.

An unhandled command logs `[mock] unhandled <cmd>` and returns `null` (or `[]`
for `list_*`). When the UI grows a command a scene needs, add a handler in the
matching `mock/` file rather than widening that fallback.
