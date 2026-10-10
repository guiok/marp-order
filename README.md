# marp-order

**English** · [Português](README.pt-BR.md)

Reorder the slides of a [Marp](https://marp.app) presentation by dragging thumbnails
around a grid, edit any slide with a live preview, and have the `.md` file rewritten
with the result.

![marp-order demo](docs/demo.gif)

Reordering a Marp deck by hand means cutting and pasting blocks of Markdown between
`---` lines without seeing the result. `marp-order` shows the whole deck at once, like
the "slide sorter" view of presentation software, and takes care of rewriting the file
without touching the content of the slides.

- **A single binary**, written in Rust; the interface runs in your browser on `localhost`.
- **Real thumbnails**, rendered by marp-cli with your presentation's theme.
- **Editor with live preview**: double-click opens the slide's Markdown and the result
  is redrawn as you type.
- **No reformatting**: unedited slides are moved byte for byte; the front matter stays
  where it is.
- **Safe**: automatic backup, atomic writes and detection of external edits.

## Requirements

| For | You need |
|---|---|
| Building | [Rust](https://rustup.rs) 1.80 or newer |
| Thumbnails and live preview (optional) | [marp-cli](https://github.com/marp-team/marp-cli) and Chrome, Chromium or Edge |

Without marp-cli the program still works, showing the text of each slide instead of a
thumbnail; the editor opens, but without the preview. The browser is opened with
`xdg-open` (Linux); on other systems, open the address printed in the terminal yourself.

## Installation

```bash
git clone https://github.com/guiok/marp-order.git
cd marp-order
cargo install --path .
```

This installs the binary in `~/.cargo/bin`. Alternatively, `make install` builds it and
copies it to `~/.local/bin`.

For thumbnails:

```bash
npm install -g @marp-team/marp-cli
```

If `marp` is not on the PATH, the program falls back to `npx @marp-team/marp-cli`, which
downloads the package on first run.

## Usage

```bash
marp-order presentation.md
```

The browser opens with the grid. Rearrange, save with `Ctrl+S` and stop the program with
`Ctrl+C` in the terminal.

| Action | How |
|---|---|
| Reorder | drag the card (or several selected cards) |
| Select several | `Ctrl`+click, `Shift`+click, `Ctrl+A` |
| Move with the keyboard | `Alt+←` / `Alt+→`, `Alt+Home` / `Alt+End` |
| Duplicate / remove | `Ctrl+D` / `Delete` |
| Undo / redo | `Ctrl+Z` / `Ctrl+Y` |
| Edit the content | double-click |
| Discard changes | **Discard** button |
| Save to the file | `Ctrl+S` |
| Reload the file from disk | `Ctrl+R` |

Each card shows its new position and, if the slide has moved, the position it has in the
file (`was 8`).

### Editing a slide

Double-clicking a card opens the editor: the slide's Markdown on the left and, on the
right, the slide rendered by Marp with the deck's theme, images and directives. The
preview is redone on every change (a few hundred milliseconds).

| Action | How |
|---|---|
| Apply the edit | `Ctrl+Enter` or **Apply** |
| Apply and write the file | `Ctrl+S` |
| Cancel | `Esc` or **Cancel** |

Applying does not write the file: the edit stays pending, like a reordering, and can be
undone with `Ctrl+Z`. The card gets an **edited** tag, and its thumbnail is
refreshed when you save. Writing `---` inside a slide splits it in two; the new slides
show up in the grid after saving.

### Options

```
marp-order [options] <presentation.md> [-- marp-cli arguments]

  --port <n>         local server port (default: chosen by the system)
  --no-open          do not open the browser automatically
  --no-thumbs        do not render thumbnails (show only the slide text)
  --marp-cmd <cmd>   marp-cli command (default: `marp` or `npx @marp-team/marp-cli`)
  -h, --help         show help
  -V, --version      show the version
```

Anything after `--` is passed on to marp-cli. Use it for custom themes:

```bash
marp-order talk.md -- --theme-set theme.css
```

The marp-cli command can also be set with the `MARP_ORDER_MARP_CMD` environment variable.

## What happens to your file

- **Front matter stays put.** The leading YAML block is never moved.
- **Slides are left intact.** The text of every unedited slide — including speaker notes
  and directives such as `<!-- _class: lead -->` — is copied byte for byte. Slides that
  remain neighbours keep even the spacing between them, so `git diff` shows only what
  changed.
- **Exact round trip.** Going back to the original order gives you the identical file.
- **Backup.** Before the first write, `presentation.md.bak` is created (or `.bak1`,
  `.bak2`… if it already exists).
- **Atomic write.** The content goes to a temporary file that is then renamed, so a crash
  halfway through does not leave you with half a deck.
- **External edits.** If the file changes in your editor while marp-order is open, saving
  is blocked until you choose between reloading and overwriting.
- **Cross-check with Marp.** Thumbnails are re-rendered on every save; if Marp sees a
  different number of slides than marp-order does, a warning is shown.

### The ⚠ warning

In Marp, a local directive written **without** `_` applies from the slide where it
appears to the end of the deck:

```markdown
<!-- paginate: true -->   ← this slide and all the following ones
<!-- _class: lead -->     ← this slide only
```

Moving a slide that has `paginate`, `class`, `header`, `footer`, `backgroundColor` and so
on changes which slides are affected. marp-order does not alter these directives; it
marks the card with ⚠ so you can check.

## Limitations

- **`headingDivider` and comments.** With `headingDivider`, Marp does not allow a comment
  before the heading of any slide but the first: it stays in the previous slide. If the
  first slide of the deck has such a comment and is moved, the comment changes slides.
  The card is marked with ⚠.
- **Encoding.** The file must be UTF-8.
- **Undo history.** Saving content edits re-reads the file and resets the undo/redo
  history. Saving only reorderings keeps it.
- **Preview assets.** Images and fonts must be in the deck's folder or below it; paths
  with `../` or absolute paths do not show up in the editor (thumbnails and the final
  output are not affected).
- **Front matter.** The editor changes slides; edit the front matter in your text editor.
- **Local use.** The server only accepts connections from `127.0.0.1` and has no
  authentication; it is not meant to be exposed on a network.
- **Preview port.** While the editor preview is in use, marp-cli's watch mode opens its
  own WebSocket server on port 37717.

## Development

```bash
make test     # parser and writer tests
make build    # release binary in target/release/
make abrir DECK=presentation.md
```

To avoid repeating `DECK`, create a `local.mk` (ignored by git) containing
`DECK := presentation.md`.

The code structure and the slide-splitting rules are described in
[docs/ARQUITETURA.md](docs/ARQUITETURA.md) (in Portuguese). Contributions are welcome:
when changing the parser, add a test to `src/deck.rs` and check the case against marp-cli.

## License

[MIT](LICENSE)
