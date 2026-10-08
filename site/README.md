# getagency.dev

The marketing site. Static, hand-written, no build step, no framework.

The design record it implements is kept privately, outside this repo, and the
copy is final there rather than here — edit it there first and here second. The
same goes for tokens, type scale, spacing and components: this stylesheet
follows that record, so changing a token here without changing it there puts
the two out of step silently.

## Preview

```sh
npx wrangler pages dev site
```

Then open the URL it prints. A plain `open site/index.html` does not work:
links and assets are root-relative.

`wrangler pages dev` rather than `python3 -m http.server`, because it is the
only local server that behaves like the thing we deploy to. Two ways that
matters. Links are extensionless (`/download`, not `/download.html`), which
Pages resolves to the `.html` file and a plain static server 404s. And it
applies `_headers`, so the CSP is actually exercised: under `http.server` the
policy is never sent, and a stale script hash looks fine locally and breaks the
theme toggle in production.

## What is here

| Path | What |
|---|---|
| `index.html` | Home: hero with the looping app video, agent strip, the four-step workflow on its spine, local-first block, the six-cell grid, the tour video, the commit-history proof, FAQ, closing CTA |
| `download.html` | The DMG and the six Linux packages, requirements and install steps |
| `run-coding-agents-in-parallel.html` | The guide: worktrees and tmux by hand, then what Agency does for each step. Answer-first, dated, with Article JSON-LD |
| `privacy.html` | The privacy page, copy verbatim from the deck |
| `404.html` | The one joke |
| `assets/css/site.css` | Tokens (Catppuccin Mocha dark, lifted Paperback light) and all components |
| `assets/js/site.js` | Theme toggle and the download page's notice for visitors on neither macOS nor Linux; the only external script |
| `assets/fonts/` | Geist and JetBrains Mono, variable, latin subsets, self-hosted (69 KB total) |
| `assets/img/` | Screenshots as trimmed WebP, plus the app icon |
| `assets/video/` | The hero loop and the one-minute tour, each as WebM and MP4 with a WebP poster. Self-hosted, so `media-src 'self'` covers them |
| `_headers` | Cloudflare Pages headers, including the CSP that enforces the privacy claims |
| `robots.txt`, `sitemap.xml` | Allow every crawler; the sitemap's `lastmod` is the date a page's content changed, so update it with the page |

Each page carries one inline script in `<head>`: the pre-paint theme snippet.
It is byte-identical on every page on purpose, because the CSP in `_headers`
allows it by hash. Change it on one page and you must change it on all pages
and recompute the hash (command in `_headers`).

## The design

The page is drawn as a git graph: six agent lanes fork off one trunk and land
in the hero window, the workflow runs down a spine with a node per step, and
the lanes merge back above the closing call to action. That is the one loud
idea; keep everything else quiet. The lane colours are the app's accent tokens,
one per agent, and the fork drawing in on load is the page's only unprompted
motion.

The home page and the guide are also written to be quoted by AI search: each
opens with a plain answer, the FAQ is visible text, and the proof is numbers
with dates. `docs/ai-search-plan.md` has the reasoning and the queue of pages
to write next. The commit-history figures in the proof block are dated; refresh
them with `git rev-list --count origin/main` and a count of merged `agent/`
branches when you touch the page.

## Theme

Three states: explicit dark, explicit light (both via the `◐` toggle, persisted
in `localStorage.theme`, the only storage the site uses), and no choice, which
follows `prefers-color-scheme`. Dark tokens live on bare `:root`; light is a
media query guarded by `:root:not([data-theme="dark"])` plus an explicit
`[data-theme="light"]` block, so the toggle wins in both directions.

Screenshots are dark-theme captures. They sit directly on the page with a
`filter: drop-shadow`, which traces the window silhouette in the image's alpha;
there is no light-mode mat. Do not put the shadow back into `box-shadow`: the
element is a rectangle, the window in it is not, and on the dark ground the
mismatch reads as a box rather than a shadow.

## The release switch

Thrown on 2026-09-06, when v0.1.0 was published. All three buttons are live:
the hero and closing CTA in `index.html` link to `/download.html`, and
`download.html` links straight at the asset. The hero meta line carries the
version, and the sub-label carries the filename and size.

The download link is the `releases/latest/download/` form, which resolves only
on a public repo with a **published** release; a draft returns 404, as does a
private repo, which is the same pair of conditions the app's update check needs.
The filename in that URL carries the version, so it is not self-updating:
**every release has to edit `download.html`** for the URL, the filename and the
size of the DMG and of each of the six Linux packages (linked since 0.2.0, the
first release to carry them), plus the three install commands that repeat an
x86_64 filename, and `index.html` for the version in the meta line.
`grep -n 'Agency[-_][0-9]' download.html` lists every one. Read each size off the
built file rather than the release page, in decimal MB.

## Before launch, in order

1. **AGE-87**: the public releases repository. The download URL, the privacy
   page's "Questions" link and the update-check constants in the app all point
   at it.
2. Swap the release-state markup in (see above) and link the privacy page's
   Questions section to the releases repo's issue tracker.
3. The docs section. The nav deliberately omits "Docs" and the hero omits "Read
   the docs" until it exists; both are specified in `03-pages.md` and go back in
   with it. The privacy page's line "The docs explain where Agency keeps
   everything on disk" also assumes it exists.
4. Verify the deployed site against `07-build-and-delivery.md`'s definition of
   done: network tab shows only same-origin requests on every page in both
   themes, Cloudflare Web Analytics is OFF in the dashboard, an accessibility
   pass, and a final claims check against the shipped app.
5. Security contact address (open question 9) on the privacy page.

## Screenshots and video

Since 2026-10-07 every image and both videos come from the real React UI running
against **demo data**: fictional projects (`orbit-api`, `lumen-web`, `tidepool`,
`ledger-cli`), `/Users/demo/...` paths and invented people, recorded by the rig
in `scripts/promo/`. Its README has the commands, from recording a scene to
writing the files below. Agent output is scripted ANSI, not a recording of a
real CLI, which would have put an account name and real token spend on screen. The earlier
set came from the author's working machine, with real project names, home
paths and a hostname in it, under a deliberate exception to the exclusion list
in `05-assets.md`. That exception no longer applies to anything on the site.

| Site file | Content |
|---|---|
| `img/shot-overview.webp` | All-projects overview, 11 agents (also the repo README's image) |
| `img/shot-issues.webp` | Issues board with an issue open |
| `img/shot-docs.webp` | An architecture note with properties, outline and backlinks |
| `img/shot-grid.webp` | Agents grid, six different agent CLIs on one repo |
| `img/shot-merge.webp` | Source Control Changes, side-by-side diff |
| `video/hero.*` | 20-second silent loop for the hero window: overview, grid, a race, a merge |
| `video/tour.*` | One-minute tour with title cards: overview, dispatch, race, loop, review and merge |

Captures are 3200x2000 (1600x1000 at 2x). `scripts/promo/edit/stills.py`
gives the stills rounded corners in their alpha so the `drop-shadow` filter
traces the window, downscales them to 2400 wide (2560 for the overview), and
runs `cwebp -q 82 -alpha_q 100 -m 6`. The `width`/`height` attributes in `index.html`
reserve layout space, so update them with the image or buy a layout shift on a
lazy-loaded image.

The videos are encoded twice: H.264 MP4 with `+faststart`, first in the
`<source>` list because it came out smaller than VP9 on this mostly static
footage, and WebM as the fallback for a browser without H.264. Keep the hero loop
under about 6 MB; it autoplays on every visit.

Whatever replaces these, recheck it for a competitor's product name, legible
anywhere in the frame. Using a rival's name as the label for your own category
concedes the category in your own marketing.
