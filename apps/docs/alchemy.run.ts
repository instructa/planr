import alchemy from "alchemy";
import { Nextjs } from "alchemy/cloudflare";

const stage = process.env.STAGE || "dev";
const alchemyPassword = process.env.ALCHEMY_PASSWORD;

if (!alchemyPassword) {
  throw new Error(
    "Missing ALCHEMY_PASSWORD. Set it in apps/docs/.env.local (any strong random string works).",
  );
}

const app = await alchemy("planr-docs", {
  stage,
  password: alchemyPassword,
});

const productionDomain = "docs.planr.so";
const siteUrl =
  app.stage === "prod"
    ? `https://${productionDomain}`
    : (process.env.NEXT_PUBLIC_SITE_URL ?? "http://localhost:3000");

export const website = await Nextjs("website", {
  name: `planr-docs-${app.stage}`,
  domains: app.stage === "prod" ? [productionDomain] : undefined,
  adopt: true,
  build: {
    env: {
      NEXT_PUBLIC_SITE_URL: siteUrl,
    },
  },
  dev: {
    command: "pnpm dev",
    domain: "localhost:3000",
    env: {
      NEXT_PUBLIC_SITE_URL: siteUrl,
    },
  },
});

console.log({ url: website.url });

await app.finalize();
