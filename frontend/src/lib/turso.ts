import { createClient, type Client } from "@libsql/client/web";
import { env } from "$env/dynamic/public";

let client: Client | null = null;

/**
 * The Turso read client (browser, read-only token). Configured at runtime via
 * `PUBLIC_TURSO_URL` + `PUBLIC_TURSO_TOKEN` (the read-only token — reads only
 * public recipe data). Used to browse the corpus; not needed for TheMealDB
 * search (that's client-direct).
 */
export function turso(): Client {
  if (!client) {
    const url = env.PUBLIC_TURSO_URL;
    if (!url) throw new Error("PUBLIC_TURSO_URL is not set");
    client = createClient({ url, authToken: env.PUBLIC_TURSO_TOKEN });
  }
  return client;
}
