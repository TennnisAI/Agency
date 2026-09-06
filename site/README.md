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
size, and `index.html` for the version in the meta line. Read the size off the
built DMG rather than the release page, in Finder's decimal MB.

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
| `shot-issues.webp` | Issues board and detail (1bit-launcher) | SHOT-04/05 |
| `shot-docs.webp` | a design spec open, with its outline and backlinks panel | SHOT-06 |
| `shot-merge.webp` | Source Control Changes, side-by-side diff | SHOT-11/12 |

Still worth capturing, in value order: SHOT-01 proper (Agents tab, grid layout,
6 tiles, mixed agent types; the hero's real job), SHOT-10 (a race in flight),
SHOT-16 (history graph with a run of `agent/` merges, for the "Agency built
Agency" block, which is typographic until then).

Regenerate site images from new captures with the same treatment: **crop to
the window itself, keeping none of the shadow margin macOS bakes into a window
capture**, then `cwebp -q 82 -alpha_q 100 -m 6`. Downscale to 2560 wide for the
hero and 2400 for the rest only when the capture is larger than that; the
current four are cropped at their captured scale rather than resampled, which is
why the intrinsic widths vary (2346 to 2502) and why `index.html` carries a
different `width`/`height` pair per image. Those attributes reserve layout
space, so a stale pair buys a layout shift on a lazy-loaded image.

Cropping to the window is what makes the shadow work, and "trim the transparent
margin" is what this line used to say. That is not the same instruction: the
margin is not transparent, it holds the baked shadow at alpha 35 to 146, and
trimming to its edge leaves it in. The corner notches need clearing too, or a
dark wedge sits outside the window's rounded corners: the baked shadow is offset
downward, so the residue is roughly 26 at the sides and 73 at the bottom corners
and no single alpha cut removes both. Take the cleaned top-left corner's own
silhouette and mirror it into the other three.

**The exclusion list in `05-assets.md` was reviewed against this set and
partly overruled, deliberately, on 2026-09-06.** The captures are from the real
working machine and show real project names, `<home>/...` paths in agent output,
and the machine hostname in a terminal prompt. Every project shown belongs to
the author, none is client work, and a populated window is most of what makes
the shots persuasive, so the list loses to that here. What was not blessed was
a competitor's product name, legible in the body of the note in the original
`shot-docs` capture; it was reshot rather than cropped, because using a rival's
name as the label for your own product category concedes the category in your
own marketing. Recheck a new capture for that, not for the project names.
