# Architecture

marp-order is a Rust binary that starts an HTTP server on `127.0.0.1` and serves a
single-page interface. The contents of the open files live in the process; the browser
keeps, per tab, the order and the edits that are not saved yet and sends them on save.

```
presentation.md ──► deck.rs (parse) ──► main.rs (state + HTTP) ◄──► ui.html (browser)
        ▲                                   │         │
        └──────── deck.rs (compose) ◄───────┘         ├──► thumbs.rs ──► marp-cli ──► JPEGs
                                                      └──► preview.rs ─► marp --watch ─► HTML
```

| File | Role |
|---|---|
| `src/deck.rs` | Splits the file into head + slides and composes it again in another order. No I/O. |
| `src/main.rs` | Command line, state, HTTP routes, backup and atomic write. |
| `src/thumbs.rs` | Thread that calls marp-cli and maps each image to a slide. |
| `src/cache.rs` | Thumbnails kept on disk between runs. |
| `src/links.rs` | Rewrites relative paths of slides copied between folders. |
| `src/preview.rs` | Live preview for the editor, with a persistent `marp --watch`. |
| `src/ui.html` | Interface (plain HTML, CSS and JavaScript), embedded in the binary. |

Dependencies: `tiny_http` and `serde_json`.

## Data model

```rust
App   { docs: Vec<Doc>, preview, .. }   // one Doc per open file (tab)
Doc   { id, path, deck, disk_order, generation, thumb_dir, .. }
Deck  { source, head, slides }          // head = BOM + front matter
Slide { lead, text, heading_start, lead_comment }
```

- `text` is the exact stretch of the file between two separators.
- `lead` is the separator line that preceded the slide (`---`, `***`…), as written.
- `heading_start` marks a slide opened by a `headingDivider` heading, without a separator.

Slides are identified by the index they had when the file was read. The browser sends
the list of positions it wants, each with the index of a slide and, if it was edited, the
new text (`Entry = (usize, Option<String>)`).

Saving only a reordering does not change the indices. Saving edits is different: since
the new text can create or merge slides, the written file is re-read and becomes the new
base (`Doc::adopt`), with renumbered indices. The thumbnails of untouched slides are
reused, and the browser translates its undo history to the new numbering (`makeRemap`):
each old item becomes the new slide with the same origin or the same text, or an entry
with its own text.

Every write increments `save_seq`. The browser sends the value it knew; if another
window wrote in between, the server answers 409 (`stale`) and the interface asks before
overwriting.

## Several files

Each open file is a `Doc` with a number (`doc`) that the routes take in the query string
or in the `POST` body. Each one has its own thumbnail folder; the renderer receives the
number of the file to render over a channel. There is a single editor preview, restarted
when the file being edited is in another folder.

`~/.config/marp-order/recent.json` holds the most recently opened files and
`session.json` the ones open right now; without arguments, the program reopens the
session. On Windows the folder is `%APPDATA%\marp-order`.

The comparison runs in the browser: the slides of the current tab (with its pending
changes) are aligned with those of the other file by the longest common subsequence of
their texts; in the gaps, slides with the same title or with at least half of their lines
in common form a "different" pair. A slide copied from the other file enters the list as
an entry with new text, like an edit, after going through `POST /api/import`, which
adjusts relative image paths when the two files are in different folders.

## Splitting into slides

The parser is not a full Markdown parser: it walks the file line by line with the minimum
state needed to decide what is a separator. The rules reproduce the behaviour of
[Marpit](https://github.com/marp-team/marpit), which splits the deck at markdown-it's
top-level thematic breaks (`hr`).

A line is a **separator** if it is a thematic break (`---`, `***`, `___`, `- - -`; three
or more marks, indented up to 3 spaces) and is **not** in any of these cases:

| Case | Example |
|---|---|
| Inside a fenced code block | ` ``` ` … `---` … ` ``` ` |
| Inside a comment, `<style>`, `<script>` or `<pre>` | `<!--` … `---` … `-->` |
| Underline of a setext heading | `Title` followed by `---` |
| Nested in a list or block quote | `- item` / blank line / `  ---` |
| Indented 4 or more spaces (indented code) | `    ---` |

The setext case is the subtlest: `---` right below a paragraph is a level-2 heading, but
right below a list item, a `#` heading or a comment it is a separator.

### `headingDivider`

Read from the front matter or from a comment (`<!-- headingDivider: 2 -->`); it accepts a
number (levels 1 to N) or a list (`[1, 3]`). It follows Marpit's rule:

- A heading of a configured level opens a new slide if **there has already been visible
  content in the deck**. Comments do not count as visible content.
- Setext headings (`===`, `---`) divide too.
- A `---` followed by a divider heading yields an empty slide between the two — Marp
  behaves this way, and marp-order shows that slide.

## Composing (`Deck::compose`)

It takes a list of entries; indices may be missing (removed slide) or repeated
(duplicated).

1. The original order, without edits, returns `source` untouched.
2. If a slide comes right after the same neighbour it had in the file, the pair is copied
   as it was, with the original separator and spacing.
3. Between slides that were not neighbours, a blank line is ensured before the separator.
   Without it, `text` followed by `---` would become a setext heading and the two slides
   would merge.
4. `heading_start` slides get no separator: the heading already divides.
5. The end of the file (with or without a trailing line break) is kept as in the original.
6. An edited slide gets its new text between the blank lines that surrounded the
   original, with line breaks in the file's style (LF or CRLF). It is always treated as
   "not a neighbour" (rule 3), and `heading_start` is recomputed from the new text.

## Thumbnails

`thumbs.rs` runs, in the deck's folder:

```
marp --no-stdin --allow-local-files --images jpeg --image-scale 0.5 -o <tmp>/s.jpeg deck.md
```

The k-th image corresponds to the k-th slide written on disk. If the number of images
differs from the number of slides, the mapping is discarded and the interface warns that
the split may be wrong — the safety net for cases the parser does not cover.

A render is triggered on open, on reload and after every save. If the file changes while
rendering, the result is discarded and the render is redone.

After a successful render the images are also copied to the cache
(`~/.cache/marp-order/thumbs/<hash of path and marp-cli command>/`), with the hash of the
deck content they came from. When a file is opened or reloaded and that hash still
matches, the cached images are shown at once; the render still runs, since an image or
a theme file may have changed without the deck changing.

Edited or added slides are not on disk yet. For those the interface sends
`POST /api/draft` with the pending state; the server writes the draft to a hidden file
next to the deck (`.<name>.marp-order.draft.md`, deleted right after), renders it and
keeps only the requested images, identified by the key of their text (`d-<key>.jpeg`).

## Editor preview

Converting the whole deck on every keystroke with a one-off marp-cli run takes seconds,
almost all of it Node starting up. So `preview.rs` keeps a `marp --watch --template bare`
running on a draft (`draft.md`) in a temporary folder:

1. The browser sends the current state (order + edits) and the position of the slide
   being edited.
2. The server composes the draft with `compose`, writes `draft.md` and waits for
   `draft.html`.
3. The editor loads `/preview/slide.html?page=N` in a hidden `<iframe>` and swaps it for
   the visible one when it finishes loading, so the slide does not flicker.

The draft is the whole deck, not just the slide, so that global styles, numbering and
inherited directives come out as in the final result; the served page hides the other
slides with CSS. Details that are not obvious:

- **Version marker.** marp-cli may write the HTML of an earlier version right after our
  write. Each draft ends with a `<style>` carrying a numbered class, and the server only
  answers when that marker shows up in the HTML.
- **Lost changes.** marp-cli ignores writes made while it is starting; if the marker does
  not appear within 700 ms, the draft is written again.
- **Relative assets.** The draft is outside the deck's folder, so `/preview/<path>` serves
  images, fonts and CSS from it (known media types only, and never above the deck's
  folder).
- **Auto reload.** The HTML of `--watch` mode carries a script that reloads the page over
  a websocket; it is removed, and the response has
  `Content-Security-Policy: connect-src 'none'`.
- **Orphan process.** On Unix, marp-cli is started by a small `sh` script that stops it
  when standard input closes, that is, when marp-order exits, even if it is killed.
  On other systems marp-cli is started directly and killed when the preview is dropped.

marp-cli in `--watch` mode opens its own websocket server for reload notifications, on
the first free port from 37717 up, on all network interfaces. marp-cli has no option to
change that. marp-order does not use the websocket, and it carries no deck content.

## HTTP API

| Route | Purpose |
|---|---|
| `GET /` | Interface |
| `GET /api/docs` | Open files and the list of recent ones |
| `GET /api/state?doc=N` | Slides (title, text, warnings) and the full state of the file |
| `GET /api/status?doc=N` | Periodic poll: external edits and thumbnail status |
| `GET /thumb/<doc>/<id>` | Thumbnail of a slide (`d-<key>` for one that is not saved yet) |
| `GET /preview/slide.html?page=N` | Slide N of the draft being edited |
| `GET /preview/<path>` | Images and fonts from the deck's folder |
| `POST /api/open` | `{path}` — opens a file in a tab |
| `POST /api/close` | `{doc}` — closes the tab |
| `POST /api/browse` | `{dir}` — folders and Markdown files, for the open dialog |
| `POST /api/save` | `{doc, generation, save_seq, slides: [{id, text?}], force}` — writes order and edits |
| `POST /api/preview` | `{doc, generation, slides, index, head?}` — converts the draft; returns the page |
| `POST /api/draft` | `{doc, generation, slides}` — requests thumbnails of unsaved slides |
| `POST /api/head` | `{doc, head, force}` — writes a new front matter |
| `POST /api/import` | `{from, to, texts}` — adjusts relative paths of copied slides |
| `POST /api/reload` | `{doc}` — re-reads the file from disk |
| `POST /api/rethumb` | `{doc}` — renders the thumbnails again |

The server refuses requests whose `Host` header is not `127.0.0.1` or `localhost`
(against DNS rebinding) and requires the `X-Marp-Order` header on every `POST`, which
keeps another page open in the browser from triggering a write.

## Interface language

Every text of the interface goes through `t('English text', …args)`. English is the key;
the `PT` table in `ui.html` holds the Portuguese translation, and static elements carry
their key in `data-t`, `data-tt` (title) or `data-tp` (placeholder). A test fails if a
key has no translation or a translation is no longer used. Error messages that come from
the server or from marp-cli are in English.

## Tests

`cargo test` runs the tests of the parser (`deck.rs`), the server (`main.rs`, including
one that starts the HTTP server and makes real requests) and `links.rs`. In the parser,
the main pattern is the round trip: compose the deck in some order, re-read the result
and check that the slides came out in the requested order.

The interface logic that does not depend on the browser (comparison alignment, line
diff, speaker notes, highlighting, history remapping, translations) lives in a `<pure>`
block of `ui.html` and is tested with `node --test tests/ui_logic.test.js`.

`cargo test -- --ignored` runs `slide_count_matches_marp_cli`, which compares the
parser's slide count with marp-cli's on edge cases. GitHub Actions
(`.github/workflows/ci.yml`) runs all of this plus clippy on every push, on Linux, macOS
and Windows, and builds with the minimum supported Rust version.
