/** The fixed words the conversation's next turn starts from after a
    login was added. Mirrors `ADDED_A_LOGIN` in
    crates/niles-bin/src/browsing/login.rs. */
export const ADDED_A_LOGIN = "Added a login";

/** The said line of the turn after a code was entered on its card.
    Mirrors `ENTERED_A_CODE` in crates/niles-bin/src/browsing/code.rs. */
export const ENTERED_A_CODE = "Entered a code";

/** The sites held in a textarea: one per line, trimmed, blanks dropped. */
export function sitesOf(text: string): string[] {
  return text
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line !== "");
}

/**
 * The origin the form will save a site as: `https` and the host, with
 * an IDN in its `xn--` form and a trailing dot dropped. A preview only;
 * Niles checks again and may refuse what this passes.
 */
export function originOf(site: string): string | null {
  const given = site.trim();
  if (!given) return null;
  let url: URL;
  try {
    url = new URL(given.includes("://") ? given : `https://${given}`);
  } catch {
    return null;
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") return null;
  if (url.port !== "" && url.port !== "443") return null;
  return `https://${url.hostname.replace(/\.$/, "")}`;
}
