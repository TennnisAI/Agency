# Being the answer when someone asks an AI

How getagency.dev gets quoted when people ask ChatGPT, Perplexity, Google's AI
Mode or Claude about running coding agents in parallel. Researched 7 October
2026.

The repo rule on competitor names holds here as everywhere: this plan names no
rival product, and neither may any page it proposes. That rule shapes the
strategy more than anything else below. The "best tool for X" and "alternatives
to Y" questions, which are where AI answers name products, can't be won on our
own site, because answering them honestly means naming rivals. They are won
off-site, in directories and lists that already name everyone. Our own pages
target the questions we can answer completely without naming anyone: product
questions about Agency, and how-to questions about the job itself.

## Where things stand

- **Nobody mentions Agency yet.** On 7 October 2026 there were no Hacker News
  hits for the domain, no Reddit threads, no Product Hunt launch, no directory
  listing and no awesome-list entry. The repo had 10 stars and about 111 release
  downloads. Every competitor-category query is answered by a rival's own
  comparison page ranking itself first.
- **The name collides.** A popular GitHub repo of agent personas owns "agency"
  plus "agents" in search. Every page should say "Agency" together with
  "desktop app", "getagency.dev" or "Tennnis" often enough that an engine can
  tell the two apart.
- **The site was readable, but said little a crawler could quote.** Pages are
  static HTML, AI crawlers get a 200 (checked with the OAI-SearchBot, ClaudeBot,
  GPTBot and PerplexityBot user agents), and Cloudflare's managed robots.txt
  blocks nothing. But there was no sitemap, no structured data, no FAQ, and no
  page answering a single how-to question.

## Done in this pass

- Home page rewritten answer-first: the opening paragraph says what Agency is,
  what it costs, where it runs and what it does, in plain words.
- A visible FAQ on the home page, in buyers' own phrasings: what Agency is,
  whether it's free, which agents, how worktrees prevent conflicts, whether code
  leaves the machine, why not tmux, Windows, permissions.
- Quotable proof on the home page: 237 agent branches merged into main and
  1,056 commits between 18 June and 6 October 2026, linked to the public history.
- `/run-coding-agents-in-parallel`: the by-hand method with git and tmux first,
  then what Agency does for each step, a disclosure, and a "who should not use
  it" line. Dated, with Article structured data.
- `robots.txt` (allows everyone, points at the sitemap), `sitemap.xml` with real
  `lastmod` dates, and `SoftwareApplication` plus `Organization` JSON-LD on the
  home page. The JSON-LD is hygiene, not a lever: a 2026 controlled study found
  no citation uplift from structured data.

## Content queue

One page per line, in priority order. Each answers its question in the first two
sentences and names no rival.

| Page | Question it answers | Proof it uses |
|---|---|---|
| Git worktrees for AI coding agents | "git worktree claude code", "stop agents overwriting each other" | The guide's commands, plus how Agency names and cleans up its worktrees |
| Run coding agents in parallel on Linux | "run Claude Code agents in parallel on Linux" | The six Linux packages and install steps from the download page |
| Race: one prompt, several agents | "send one prompt to several coding agents and keep the best" | A real race from this repo's history |
| Loop an agent until the tests pass | "make an agent keep going until tests pass" | The loop caps (attempts, wall clock, tokens) from `docs/agentic-loops.md` |
| A markdown issue tracker agents can work | "issue tracker coding agents can work from" | The `.agency/issues` format and the status transitions |
| The agents Agency supports | "run Kimi Code, Crush or Pi in parallel" | One short section per built-in CLI, with how Agency detects and installs it |
| Choosing a tool to run agents in parallel | "best tool to run coding agents in parallel" | The criteria (isolation, licence, platforms, privacy, which agents), with no rivals named; links out to the directories that do name them |

Write fewer, better pages. A dozen thin near-duplicates read as spam to engines
and to people.

## Off-site, which only the team can do

All of it openly, as ourselves. No sock puppets, no paid placements that aren't
disclosed.

1. **Submit to the open-source alternatives directory** that lists the leading
   Mac-only, closed-source agent runner; it takes submissions.
2. **Open a PR to the awesome list of agent orchestrators** on GitHub (about
   2,100 stars), which takes PRs.
3. **List Agency on the alternatives site** under that same runner.
4. **Launch on Product Hunt.** A comparable open-source runner placed third of
   the day in May 2026.
5. **Post a Show HN** with the "Agency built Agency" numbers as the hook.
6. **Answer existing Reddit and forum threads** about running agents in parallel,
   where Agency genuinely fits, saying who we are.
7. **Put the promo video on YouTube** with a plain title ("Run Claude Code,
   Codex and Gemini CLI in parallel, each on its own branch").

## Measuring it

The site has no analytics, and the privacy page says "we do not read [the host's
logs] for anything". So the usual measures (an AI-referrals channel in analytics,
crawler hits in server logs) are off the table unless that sentence changes, and
changing it is a product decision, not an SEO one.

That leaves the manual tracker: `docs/ai-search-tracker.csv`. Once a month, paste
each question into ChatGPT (search on), Perplexity and Google AI Mode, and record
whether Agency is named and whether a getagency.dev page is cited. Judge a page
only after six to eight weeks.

The tracker in the repo holds only the questions that name no rival. The
rival-named questions are worth checking too ("open-source alternative to …"),
but they stay out of the tree.
