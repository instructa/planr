import { z } from "zod";
import { principalSchema, type Principal, type Session } from "./contract.ts";

// Preserve exact identifiers; reject padding and control characters, including trailer newlines.
const identifier = z.string().min(1).refine(value =>
  value === value.trim() && !/[\x00-\x1f\x7f]/.test(value)
);
const pairSchema = z.object({ issuer: identifier, subject: identifier });
const requestPrincipalSchema = principalSchema.extend(pairSchema.shape);
type Identity = Pick<Session, "mode" | "principal">;
interface ForkServer {
  experimental_requestPrincipal?: unknown;
  experimental_trustedIdentityPort?: unknown;
}
const key = (principal: Pick<Principal, "issuer" | "subject">) =>
  JSON.stringify([principal.issuer, principal.subject]);

// Call synchronously at handler entry, before settings/storage awaits or any other request work.
export function requestIdentity(server: ForkServer, warn: (message: string) => void): Identity {
  const lookup = server.experimental_requestPrincipal;
  if (typeof lookup !== "function") {
    return { mode: "convention", principal: null };
  }
  const mode = server.experimental_trustedIdentityPort === null ? "unavailable" : "enforced";
  try {
    const parsed = requestPrincipalSchema.safeParse(lookup.call(server));
    return { mode, principal: parsed.success ? parsed.data : null };
  } catch {
    warn("Request identity lookup failed; approval refused.");
    return { mode, principal: null };
  }
}

export function parseApprovers(text: string, warn: (message: string) => void) {
  const identities = new Set<string>();
  let restricted = false;
  for (const [index, line] of text.split(/\r?\n/).entries()) {
    if (!line.trim()) {
      continue;
    }
    restricted = true;
    const colon = line.indexOf(":");
    const parsed = pairSchema.safeParse({
      issuer: colon < 0 ? "" : line.slice(0, colon),
      subject: colon < 0 ? "" : line.slice(colon + 1),
    });
    if (!parsed.success) {
      warn(`Invalid approvers line ${index + 1}; use issuer:subject without empty or padded identifiers.`);
      continue;
    }
    identities.add(key(parsed.data));
  }
  return { identities, restricted };
}

export function approvalSession(identity: Identity, approvers: ReturnType<typeof parseApprovers>): Session {
  let reason: string | null = null;
  if (identity.mode === "unavailable") {
    reason = "Approval is unavailable: bb's trusted identity listener is not configured.";
  } else if (identity.mode === "enforced") {
    if (!identity.principal) {
      reason = "Sign in through bb's identity listener to approve planning changes.";
    } else if (identity.principal.kind !== "person") {
      reason = "Only people can approve planning changes.";
    } else if (approvers.restricted && !approvers.identities.has(key(identity.principal))) {
      reason = "This identity is not listed in Planr's approvers setting.";
    }
  }
  return { ...identity, canApprove: reason === null, reason };
}
