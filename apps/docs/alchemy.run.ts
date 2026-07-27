import * as Alchemy from "alchemy";
import * as AdoptPolicy from "alchemy/AdoptPolicy";
import * as Cloudflare from "alchemy/Cloudflare";
import * as Effect from "effect/Effect";
// @ts-expect-error The executable redirect inventory is intentionally shared with Node verification scripts.
import { legacyRedirects } from "./redirects.mjs";

const productionDomain = "planr.so";

const Website = Cloudflare.Website.StaticSite(
  "Website",
  Alchemy.Stack.useSync(({ stage }) => {
    if (stage === "prod" && process.env.PLANR_DOCS_RECEIPT_VALIDATED !== "1") {
      throw new Error("production docs deployment requires a validated exact-revision receipt; use pnpm docs:deploy");
    }
    const siteUrl =
      stage === "prod"
        ? `https://${productionDomain}`
        : (process.env.NEXT_PUBLIC_SITE_URL ?? "http://localhost:3000");

    return {
      name: `planr-docs-${stage}`,
      // Production promotion validates the exact-revision receipt before
      // Alchemy starts. This command deliberately consumes the reviewed output
      // instead of starting a second Next production build.
      command: "node scripts/use-existing-output.mjs",
      outdir: "out",
      main: "worker.mjs",
      domain: stage === "prod" ? productionDomain : undefined,
      compatibility: {
        date: "2026-04-24",
        flags: ["nodejs_compat", "global_fetch_strictly_public"],
      },
      assets: {
        htmlHandling: "auto-trailing-slash",
        notFoundHandling: "404-page",
        runWorkerFirst: [
          ...legacyRedirects.map(({ source }: { source: string }) => source),
          "/docs/*.md",
          "/api/search",
          "/llms.txt",
          "/llms-full.txt",
        ],
      },
      env: {
        NEXT_PUBLIC_SITE_URL: siteUrl,
      },
      dev: {
        command: "pnpm dev",
        url: "http://localhost:3000",
        env: {
          NEXT_PUBLIC_SITE_URL: siteUrl,
        },
      },
      memo: {
        include: [
          "app/**",
          "components/**",
          "content/**",
          "lib/**",
          "public/**",
          "scripts/**",
          "*.ts",
          "*.tsx",
          "*.mjs",
          "package.json",
          "../../pnpm-lock.yaml",
        ],
      },
    };
  }),
).pipe(AdoptPolicy.adopt(true));

export default Alchemy.Stack(
  "PlanrDocs",
  {
    providers: Cloudflare.providers(),
    state: Cloudflare.state(),
  },
  Effect.gen(function* () {
    const website = yield* Website;

    return {
      url: website.url,
    };
  }),
);
