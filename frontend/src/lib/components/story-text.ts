import { createRawSnippet } from "svelte";

/**
 * A snippet of plain text, for stories of components that take `children`.
 *
 * Svelte 5 children are snippets, not strings. Passing a string renders nothing and
 * throws `e is not a function` — and, because the harness screenshots whatever the
 * page ended up showing, Storybook's error screen gets captured and blessed as though
 * it were the component. That happened: nine baselines of an error page, green,
 * because the shot was taken and never looked at.
 *
 * So text goes through here, and any new story of a `children`-taking component should
 * use it rather than reaching for a cast.
 */
export const text = (content: string) =>
  createRawSnippet(() => ({ render: () => `<span>${content}</span>` }));
