# getagency.dev

The marketing site. Static, hand-written, no build step, no framework.

The design record it implements is kept privately, outside this repo, and the
copy is final there rather than here — edit it there first and here second. The
same goes for tokens, type scale, spacing and components: this stylesheet
follows that record, so changing a token here without changing it there puts
the two out of step silently.

## Preview

```sh
python3 -m http.server 5230 -d site
```

Then open http://localhost:5230. A plain `open site/index.html` does not work:
links and assets are root-relative.

## What is here

| Path | What |
|---|---|
| `index.html` | Home: hero, agent strip, local-first block, four feature blocks, card grid, dogfooding block, closing CTA |
| `download.html` | Requirements and install steps |
| `privacy.html` | The privacy page, copy verbatim from the deck |
| `404.html` | The one joke |
| `assets/css/site.css` | Tokens (Catppuccin Mocha dark, lifted Paperback light) and all components |
| `assets/js/site.js` | Theme toggle and the download page's non-Mac notice; the only external script |
| `assets/fonts/` | Geist and JetBrains Mono, variable, latin subsets, self-hosted (69 KB total) |
| `assets/img/` | Screenshots as trimmed WebP, plus the app icon |
| `_headers` | Cloudflare Pages headers, including the CSP that enforces the privacy claims |

Each page carries one inline script in `<head>`: the pre-paint theme snippet.
It is byte-identical on every page on purpose, because the CSP in `_headers`
allows it by hash. Change it on one page and you must change it on all pages
and recompute the hash (command in `_headers`).

## Theme

Three states: explicit dark, explicit light (both via the `◐` toggle, persisted
in `localStorage.theme`, the only storage the site uses), and no choice, which
follows `prefers-color-scheme`. Dark tokens live on bare `:root`; light is a
media query guarded by `:root:not([data-theme="dark"])` plus an explicit
`[data-theme="light"]` block, so the toggle wins in both directions.

Screenshots are dark-theme captures. In light mode they sit on a `--crust` mat
with padding, which is the documented fallback in `04-design-system.md` until a
Paperback capture set exists.

## The release switch

There is no published release yet, so both download buttons render the
documented empty state: an inert `Coming shortly` with `No release published
yet` as the sub-label. The release-state markup sits in an HTML comment
directly above each one, in `index.html` (hero and closing CTA) and
`download.html` (plus its version/size sub-label). When v0.1.0 is published,
swap them in and fill the real DMG URL, filename and size. Per
`07-build-and-delivery.md`, the site does not go live before that day.

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

## Screenshot set

Current images are real captures from `screenshots/` at the repo root, mapped
against the shot list in `05-assets.md`:

| Site image | Capture | Shot-list slot |
|---|---|---|
| `shot-overview.webp` | all-projects overview, 13 agents | stands in for SHOT-01 (hero) |
| `shot-focus.webp` | focus view, agent working an issue | SHOT-02 |
| `shot-issues.webp` | issue board and detail | SHOT-04/05 |
| `shot-docs.webp` | note with an agent in the side panel | SHOT-06 |
| `shot-merge.webp` | approve-and-merge, merged cleanly | SHOT-11/12 |

Still worth capturing, in value order: SHOT-01 proper (Agents tab, grid layout,
6 tiles, mixed agent types; the hero's real job), SHOT-10 (a race in flight),
SHOT-16 (history graph with a run of `agent/` merges, for the "Agency built
Agency" block, which is typographic until then).

Regenerate site images from new captures with the same treatment: trim the
transparent margin, resize to 2560 wide for the hero and 2400 for the rest,
`cwebp -q 82 -alpha_q 100 -m 6`.

**Check every capture against the exclusion list in `05-assets.md` before the
site ships.** The current set is from the real working machine and shows real
project names, `<home>/...` paths in agent output, and the machine
hostname in a terminal prompt. That list rules those out, so either bless them
deliberately or recapture from a demo repo before launch.
