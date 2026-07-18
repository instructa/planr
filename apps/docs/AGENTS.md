# Planr Docs App

Next.js 16 and Fumadocs exported as direct Cloudflare assets through Alchemy v2.

## Local commands

```sh
pnpm dev                 # credential-free Next.js content development
pnpm alchemy:dev         # Cloudflare-local development through Alchemy
pnpm build               # standard Next.js production build
pnpm verify:deployment   # static Cloudflare artifact without deployment
pnpm deploy              # Alchemy v2 deploys the explicit prod stage
pnpm destroy             # destructive; explicitly targets prod
```

## Deployment contract

- `alchemy.run.ts` is the infrastructure source of truth.
- `planr.so` belongs only to stage `prod`.
- Local credentials live in the Alchemy `default` profile created by `alchemy login`; no `.env` deployment secrets are required.
- CI uses `CLOUDFLARE_ACCOUNT_ID` plus a scoped `CLOUDFLARE_API_TOKEN`.
- The `planr.so` Cloudflare zone must already exist in the authenticated account.
- Never commit `.env.local`, `.alchemy`, `out`, `.wrangler-static`, or generated Wrangler output from local tooling.
- Run the static, maintenance, release, deployment, and browser gates before production deploys.
- Do not use `alchemy destroy` as rollback; redeploy the last known-good commit to the same stage.
