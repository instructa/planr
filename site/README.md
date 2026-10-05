# planr.so

The planr website: a landing page and the docs, built as static files from this repository.

```sh
cd site
npm ci
npm run build     # writes dist/ and checks every link
npm run preview   # serves dist/ on http://127.0.0.1:4321
```

Requires Node.js 20 or newer & Git. There are no dependencies.

## Where things come from

- **Docs** render at build time from `README.md`, `docs/**/*.md` & `skills/planr/references/format.md`.
  Nothing is copied into `site/`. Links between those files become site routes; links to any other
  repository path go to `https://github.com/instructa/planr/blob/main/<path>`. A link to a missing
  file stops the build. The page list is `DOCS` in `src/docs.mjs`.
- **Markdown for agents.** Every docs page is also written as Markdown at its route with `.md`
  (`/docs/` is `/docs/index.md`, `/docs/plugins/codex/` is `/docs/plugins/codex.md`; `mdRoute` in
  `src/docs.mjs`). Links in those copies are absolute: docs to their `.md` on planr.so, other
  repository files to GitHub. Code & the quickstart's show marks stay as written. `/llms.txt` lists
  the copies by the sidebar's groups with the notes in `content/llms.mjs`. The deploy serves them
  as `text/markdown` & `text/plain` (`deploy/alchemy.run.ts`). The agent setup guide is one of
  those pages, `docs/setup.md`, at `/docs/setup/` & `/docs/setup.md`.
- **The quickstart chooser** comes from `docs/quickstart.md`, which reads on GitHub with every path.
  A comment `<!-- show: key=value -->` on the line before a heading makes that heading's section depend
  on a choice; `content/quickstart.mjs` holds the questions & answers. The build stops when a mark
  names an unknown answer or an answer shows nothing. Without JavaScript the chooser stays hidden &
  every section shows.
- **Landing copy & install commands** live in `content/landing.mjs`, the one file to edit for a copy
  pass or when the plugin install commands change. The hero has two modes: one setup prompt for an
  agent, or the install tabs for people. Prompts that call the skill have a `skill` & a `plain` form;
  `agents` lists each agent's invocation. Picking an agent on the How it works page, an install tab or the
  quickstart rewrites every such prompt & its copy text, and the browser remembers the choice
  (`planr-agent`). Without JavaScript the prompts read for the first agent.
- **The schematics, the task file & the terminal output** come from the synthetic sample in
  `fixtures/board-data.json`. `src/sample.mjs` replays its runs as commits in a temporary planning
  repo; the build runs `session.setup` (which writes the sample board page), reads the task file and
  runs the "See what is ready" commands with the real CLI. The schematic board pieces on the How it
  works page, "See what is ready" & the orchestration picture (`src/schematic.mjs`) draw from the board's own
  data for that repo (`lib/board.mjs`), never from hand-typed statuses. So does the plan's side of the
  repository picture; the project's side is named in the copy. Commits use a fixed identity & dates, so every build
  prints the same hashes. The only difference between two builds is the board's `generatedAt`,
  which the board does not show.
- **The brand** lives in `assets/brand` (logo, mark & their reveal, see `DESIGN.md`, Brand) and
  `assets/favicon.svg`. The nav inlines `planr-logo.svg` at build time so it takes the text color; the
  landing inlines `planr-logo-animated.svg` instead, which reveals once on load. `brand/MOTION.md` & any
  dotfile stay out of `dist/`.
- **Two WebGL scenes**, raw WebGL2 without dependencies. `assets/header.js` is the hero field: a
  halftone of the board's status marks where each cell grows an open ring, fills it from the right &
  closes it into a disc as a slow noise field drifts right. It runs on every width: phones get smaller
  cells & a fainter field with no pointer response; on wide screens a fine pointer moves nearby marks
  a step forward. Below the hero the same file draws the field on, in a second canvas fixed to the
  window (see Design). `assets/graph.js` floats the sample's tasks as board rows around the closing band;
  `src/scene.mjs` places them and records the status changes it plays by asking the real engine what
  is ready next, so the browser never applies a planning rule of its own.
- **Videos** render only when their files exist in `assets/media`: the landing's short teaser
  (`planr-teaser.mp4` & `planr-teaser.jpg`) and the tutorial on the docs start page (`planr-intro.mp4`
  & `planr-intro.jpg`). A `.vtt` of the same name adds captions. Without the files there is no slot.
- **The sample** is the budget app's planning repo `budget-planning` in `fixtures/board-data.json`: the
  goal `mobile-app` (MOB-01 to MOB-08) and a few background goals. After editing its source fields,
  `npm run fixture` replays it & refreshes every derived field with the real engine.
- **The board page** is that real output with two additions: it references the site's font files
  instead of inlining them, and it reads the site's stored theme before it paints.
- **Fonts** are the board's own files from `skills/planr/assets/board/fonts/`, with their licenses.

`src/check.mjs` checks every internal link, asset & `#fragment` in `dist/`, that every GitHub link
into this repository names a path that exists, and that every planr.so URL, in the pages, the `.md`
copies & `llms.txt`, names a page or file of the build. The agent files may only link absolutely.

## Design

The site uses the board's design system (`DESIGN.md`): the same tokens in light & dark, Mona Sans for
prose & Monaspace Neon for anything planr prints, ink as the only emphasis, status color only on
drawn marks & the board itself. For reading it scales the type up: display heading on the landing,
16–18px body, labels at 11px.

The landing goes hero & install, teaser, then four blocks (see what is ready, the plan beside your
code, tasks in Markdown, several agents at once), a link to How it works & the closing band. How it
works (`/how-it-works/`) follows the shared journey (set up, plan with a prompt, approve, agents work,
a decision waits on you, check, runs, keep planning): prompts first, schematic board pieces as the
pictures, with the agent picker the prompts follow. Install commands appear once, in the hero tabs.

**The mark field below the hero:** the hero's halftone of status marks continues down the landing,
ink-only & quiet. It is one fixed canvas behind the content (`assets/header.js`, the same mark
drawing as the hero): marks sit in the page margins, about 2% ink behind text, with a soft halo
around each schematic. Marks further down the page are fuller, and each fills a little more as it
scrolls past the reading line, so the page works itself off while you read. It redraws only on
scroll, resize or a theme change, fades out before the closing band & keeps one still state under
reduced motion. Both
scenes stay ink-only or keep color on status marks, keep their text clear, never take focus or
pointer events, pause off-screen or in a hidden tab and draw one still frame under reduced motion.
The hero field runs on all widths; the graph scene runs only on screens 980px or wider and starts
when the closing band comes near. The logo reveal shows its final frame under reduced motion. The
one terminal prints its lines once when it comes into view, and not at all under reduced motion.

## Deploying

`dist/` is plain static files with a `404.html`, `robots.txt` & `sitemap.xml`; no application runtime,
analytics, external fonts or CDNs. Alchemy lives in the separate private package `site/deploy/`,
so the site build and the published planr package have no deployment dependencies.
Effect's Node adapter and its transitive overrides stay on the same pinned version to avoid
mixing the older Alchemy release with newer, incompatible Effect internals.

Deploy from the repository root with Node 22.22.2+, 24.15.0+ or 26+ (the deploy dependencies' requirement).
First authenticate on the machine with
`alchemy login` (use `npx alchemy login` in `site/deploy/` after installing), or configure Cloudflare
credentials, including `CLOUDFLARE_ACCOUNT_ID` and `CLOUDFLARE_API_TOKEN`.

```sh
(cd site/deploy && npm ci && npm run deploy:preview)
(cd site/deploy && npm run deploy:prod)
```

Each deploy runs `npm ci && npm run build` in `site/` and uploads `site/dist`. A small Worker
(`deploy/media.js`) runs only for `/assets/media/*` and answers byte ranges, which Safari needs to
play the videos; every other path is served by static assets alone.
Alchemy knows few file types and does not upload a `_headers` file, so the stack passes Cloudflare
`_headers` rules as asset config: `.md` copies as `text/markdown; charset=utf-8`, `/llms.txt` as
`text/plain; charset=utf-8`, plus videos, posters, fonts & the sitemap (`CONTENT_TYPES`).

The `PlanrDocs` stack uses Cloudflare state and adopts the existing `Website` resource,
named `planr-docs-<stage>`. Production replaces the old site at `planr.so`; preview uses a
Cloudflare URL without a custom domain. To check the config without contacting Cloudflare,
run `npm run typecheck` in `site/deploy/` after `npm ci`.
