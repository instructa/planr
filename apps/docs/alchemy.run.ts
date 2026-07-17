import * as Alchemy from "alchemy";
import * as AdoptPolicy from "alchemy/AdoptPolicy";
import * as Cloudflare from "alchemy/Cloudflare";
import * as Effect from "effect/Effect";

const productionDomain = "planr.so";

const Website = Cloudflare.Website.StaticSite(
  "Website",
  Alchemy.Stack.useSync(({ stage }) => {
    const siteUrl =
      stage === "prod"
        ? `https://${productionDomain}`
        : (process.env.NEXT_PUBLIC_SITE_URL ?? "http://localhost:3000");

    return {
      name: `planr-docs-${stage}`,
      command: "pnpm run build:deploy",
      outdir: ".open-next/assets",
      main: ".alchemy-worker/worker.js",
      bundle: false,
      domain: stage === "prod" ? productionDomain : undefined,
      compatibility: {
        date: "2026-04-24",
        flags: ["nodejs_compat", "global_fetch_strictly_public"],
      },
      assets: {
        htmlHandling: "auto-trailing-slash",
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
