# Planr Docs App

Next.js 16 and Fumadocs on Cloudflare Workers through Alchemy and OpenNext.

## Local commands

```sh
pnpm dev                 # credential-free Next.js content development
pnpm alchemy:dev         # Cloudflare-local development through Alchemy
pnpm build               # standard Next.js production build
pnpm verify:deployment   # OpenNext Worker artifact without deployment
pnpm deploy              # Alchemy deploy; STAGE=prod attaches docs.planr.so
pnpm destroy             # destructive; always confirm the stage first
```

## Deployment contract

- `alchemy.run.ts` is the infrastructure source of truth.
- `docs.planr.so` belongs only to stage `prod`.
- Required secrets: `ALCHEMY_PASSWORD`, `CLOUDFLARE_API_TOKEN`, and `CLOUDFLARE_ACCOUNT_ID`.
- The `planr.so` Cloudflare zone must already exist in the authenticated account.
- Never commit `.env.local`, `.alchemy`, `.open-next`, or generated `wrangler.jsonc` state.
- Run the static, maintenance, release, deployment, and browser gates before production deploys.
- Do not use `alchemy destroy` as rollback; redeploy the last known-good commit to the same stage.
