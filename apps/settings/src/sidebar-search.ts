/**
 * The search field at the top of Settings' sidebar, as a Mac's System Settings has one: the pages
 * whose words begin with every word typed, from the words Settings declares to Spotlight
 * (`searchable.json`, docs/adr/0051-search.mdx) and the name the sidebar shows. The search service
 * matches the same way (a word typed is the beginning of a word of the item's title or keywords,
 * accents and case aside), so a page the sidebar finds is the page Spotlight finds. Pure, so it is
 * tested without a renderer.
 */

/** A page Settings declares (`searchable.json`): its URL, and its titles and keywords by language. */
export type DeclaredPage = {
  id: string;
  url: string;
  titles: Record<string, string>;
  keywords: Record<string, readonly string[]>;
};

const SCHEME = "oxyos-settings://";

/** Text as the search compares it: without accents, in lower case. */
export function fold(text: string): string {
  return text.normalize("NFD").replace(/\p{M}/gu, "").toLocaleLowerCase();
}

/** The words a text is found by: each word, and each part of one joined by a hyphen, a dot or a
    stroke ("Wi-Fi" by "wi-fi", "wi" and "fi"). */
function wordsOf(text: string): string[] {
  const words = fold(text).split(/\s+/).filter(Boolean);
  return [...words, ...words.flatMap((word) => word.split(/[-_./]+/).filter(Boolean))];
}

/** The page a declared item opens (`oxyos-settings://wifi` is `wifi`), or null. */
export function declaredPage(item: DeclaredPage): string | null {
  return item.url.startsWith(SCHEME) ? item.url.slice(SCHEME.length) || null : null;
}

/**
 * Whether `query` finds a page: every word typed begins a word of what the page is known by (the
 * name the sidebar shows, its declared title and keywords in `language`, else English's).
 */
export function finds(query: string, known: readonly string[]): boolean {
  const typed = fold(query).split(/\s+/).filter(Boolean);
  if (typed.length === 0) return true;
  const words = known.flatMap(wordsOf);
  return typed.every((word) => words.some((one) => one.startsWith(word)));
}

/**
 * The sidebar's groups as the search leaves them: every group with only the rows `query` finds,
 * in their order. A blank query leaves them as they are (the same array, so nothing redraws).
 */
export function searchedGroups<Item extends { page?: string; label: string }, Group extends { items: Item[] }>(
  groups: Group[],
  query: string,
  declared: readonly DeclaredPage[],
  language: string,
): Group[] {
  if (query.trim().length === 0) return groups;
  const byPage = new Map<string, DeclaredPage>();
  for (const item of declared) {
    const page = declaredPage(item);
    if (page !== null) byPage.set(page, item);
  }
  return groups.map((group) => ({
    ...group,
    items: group.items.filter((item) => {
      if (item.page === undefined) return false;
      const page = byPage.get(item.page);
      const known = [
        item.label,
        ...(page ? [page.titles[language] ?? page.titles.en ?? "", ...(page.keywords[language] ?? page.keywords.en ?? [])] : []),
      ];
      return finds(query, known);
    }),
  }));
}
