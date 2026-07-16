import { redirect } from "@sveltejs/kit";

// There is no "home" — the arc starts at `pick` (#36).
export function load() {
  redirect(307, "/pick");
}
