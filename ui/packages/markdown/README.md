# markdown

Untrusted Markdown, such as a model's reply or a captured prompt, read into a
typed tree and rendered as Svelte 5 elements, never as HTML.

## Contract

| Part                    | What it is                                                                                                                                                                                                                                          |
| ----------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Markdown` (`markdown`) | The pure core. `Markdown.parse(text)` reads CommonMark with GitHub's extensions (tables, strikethrough, task lists, autolinks, footnotes) into its `blocks`, a closed tree of `Block` and `Inline`; `Markdown.inline(text)` reads one line's spans. |
| `Markdown.svelte`       | Draws a text's blocks. Props: `text`.                                                                                                                                                                                                               |
| `markdown.css`          | The theme, imported by the view: `--markdown-*` custom properties with defaults at no specificity, which a host overrides on `:root`.                                                                                                               |

The text is made inert as it is read: raw HTML becomes text, a link keeps its
address but is not clickable, an image is named but never loaded, and nesting
deeper than `Markdown.DEPTH` is kept as its source text, which bounds
rendering recursion. Agents wrap injected context in tags on their own lines
(`<system-reminder>`); such a region is a `tagged` block, drawn folded under
its name. Headings are drawn three levels below the page's own, so a reply
never outranks the view it sits in.

## Use

```svelte
<script lang="ts">
  import Markdown from 'markdown/Markdown.svelte';

  let { reply }: { reply: string } = $props();
</script>

<Markdown text={reply} />
```

## Checks

`bun run check` and `bun run test` here. `tests/boundary.test.ts` keeps the
sources importing only each other, Svelte, and the Markdown parser.
