import { fileURLToPath } from "node:url";
import * as Alchemy from "alchemy";
import * as AdoptPolicy from "alchemy/AdoptPolicy";
import * as Cloudflare from "alchemy/Cloudflare";
import * as Effect from "effect/Effect";

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
    assets: {
      htmlHandling: "auto-trailing-slash",
      notFoundHandling: "404-page",
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
