import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import * as Alchemy from "alchemy";
import * as AdoptPolicy from "alchemy/AdoptPolicy";
import * as Cloudflare from "alchemy/Cloudflare";
import * as Effect from "effect/Effect";

const CONTENT_TYPES = {
  "/*.md": "text/markdown; charset=utf-8",
  "/llms.txt": "text/plain; charset=utf-8",
  "/*.mp4": "video/mp4",
  "/*.jpg": "image/jpeg",
  "/*.woff2": "font/woff2",
  "/*.xml": "application/xml",
};

const Website = Cloudflare.Website.StaticSite(
  "Website",
  Alchemy.Stack.useSync(({ stage }) => ({
    name: `planr-docs-${stage}`,
    domain: stage === "prod" ? "planr.so" : undefined,
    cwd: fileURLToPath(new URL("../", import.meta.url)),
    command: "npm ci && npm run build",
    outdir: "dist",
    // Docs and shared assets live outside site/. Rebuild them on every deploy.
    memo: false,
    // Videos need byte ranges for Safari; every other path is served by static assets alone.
    script: readFileSync(new URL("./media.js", import.meta.url), "utf8"),
    assets: {
      htmlHandling: "auto-trailing-slash",
      notFoundHandling: "404-page",
      runWorkerFirst: ["/assets/media/*"],
      // Alchemy uploads unknown file types as application/octet-stream and does not send a
      // dist/_headers file, so the content types go here as Cloudflare _headers rules.
      headers: Object.entries(CONTENT_TYPES)
        .map(([path, type]) => `${path}\n  Content-Type: ${type}`)
        .join("\n"),
    },
  })),
).pipe(AdoptPolicy.adopt(true));

export default Alchemy.Stack(
  "PlanrDocs",
  {
    providers: Cloudflare.providers(),
    state: Cloudflare.state(),
  },
  Effect.gen(function* () {
    const website = yield* Website;
    return { url: website.url };
  }),
);
