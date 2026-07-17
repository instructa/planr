// Cloudflare binding types derived from the Alchemy deployment resource.
import type { website } from "./alchemy.run.ts";

declare global {
  type CloudflareEnv = typeof website.Env;
}

declare module "cloudflare:workers" {
  namespace Cloudflare {
    export type Env = CloudflareEnv;
  }
}
