import { assertAlchemyRuntime } from "./scripts/check-alchemy-runtime.mjs";

assertAlchemyRuntime();

const [{ default: alchemy }, { Website }] = await Promise.all([
  import("alchemy"),
  import("alchemy/cloudflare"),
]);

const app = await alchemy("planr");

export const presetCatalog = await Website("preset-catalog", {
  name: `planr-${app.stage}-catalog`,
  assets: "./dist/website",
  build: {
    command: "pnpm site:build",
    memoize: false,
  },
  dev: "pnpm site:serve",
  spa: false,
});

console.log({ url: presetCatalog.url });

await app.finalize();
