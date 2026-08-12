# justgui

A native, dependency-light GUI for [`just`](https://github.com/casey/just). Point
it at a directory with a `justfile` and it turns every recipe into a button:
input fields for parameters, live streamed output when you run one, and a
plain-text editor for the justfile itself.

`just` itself is used to introspect the justfile (`just --dump --dump-format
json`), so justgui never parses justfile syntax and stays correct as `just`
evolves.

This is a Rust + [Slint](https://slint.dev) rewrite of an earlier C++/Dear
ImGui prototype (kept in [`../src`](../src) for reference). Colors, fonts and
corner radius are all driven by a small `justgui.toml` config file, and are
re-applied live while the app is running — edit the file, save, and the
window restyles itself with no restart.

## Introduction: the workflow this is for

You have a project with a `justfile` and want a quick, clickable front end
for it instead of memorizing recipe names and flags. The intended loop is:

1. Install `justgui` once (see below).
2. `cd` into any project that has a `justfile`.
3. Run `justgui`.
4. A window opens listing that project's recipes as buttons. Click one, watch
   its output stream live, adjust parameters, edit the justfile in-app if you
   need to.

No project-specific setup, no config file required to get started — it just
reads whatever justfile is in the current directory.

## Requirements

- The `just` CLI on your `PATH` (you have this already if you use justfiles).
  If not: `cargo install just`, or your OS package manager.
- The Rust toolchain (`cargo`/`rustc`), via [rustup](https://rustup.rs), to
  build it. There is no separate runtime dependency once built — the result
  is a single native binary (no bundled browser, no Python/Node runtime).
- On Linux, a desktop with the usual X11/Wayland + OpenGL libraries that
  almost every desktop distro already ships with.

## Setup: build and install

From this directory (`justgui-rs/`):

```sh
just install
```

This runs `cargo install --path . --locked`, which builds a release binary
and places it at `~/.cargo/bin/justgui`. If you installed Rust via rustup,
`~/.cargo/bin` is already on your `PATH`, so `justgui` becomes available
from **any directory, in any terminal**, immediately — that's the "system
wide" part.

If it's not yet on your `PATH`, add this to your shell profile
(`~/.bashrc`, `~/.zshrc`, etc.) and restart your shell:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
```

Verify it worked:

```sh
which justgui
```

Don't have `just` installed yet to run `just install`? The justfile here is
just a thin, convenient wrapper — you can run the exact same steps directly:

```sh
cargo install --path . --locked
```

### Other build recipes

```sh
just build      # release binary at target/release/justgui, without installing
just run        # build + run against the current directory, without installing
just run dir    # build + run against another directory
just test       # run the test suite
just clean      # remove build artifacts
just uninstall  # cargo uninstall justgui
```

## How to use it

```sh
cd /path/to/your/project   # any directory containing a justfile
justgui
```

That's it — a window opens with:

- **Recipes tab**: a grid of buttons, one per recipe you've chosen to show
  (see [The recipe grid](#the-recipe-grid) below).
- **Edit justfile tab**: the raw justfile as text, with Save and "Reload
  from disk" buttons.

A "Transparency" slider and a "Theme" button sit above the tabs — see
[Transparency and the theme editor](#transparency-and-the-theme-editor).

Output from whichever recipe you last ran streams live in the panel at the
bottom of the window, with an input field below it for recipes that prompt
for confirmation or other input — see
[Live output and input](#live-output-and-input).

## The recipe grid

Each recipe is a plain button showing just its name. The grid reflows to
fit however wide you make the window.

- **Run it**: click a button. If the recipe takes no parameters, it runs
  immediately. If it does, clicking opens a small popover with a field per
  parameter (pre-filled with its default, if any) and a "Run" button inside.
  Variadic parameters (`*args` / `+args`) are entered as a single
  space-separated field.
- **See what it does**: hover over a button for its doc comment as a
  tooltip.
- **Choose which recipes are shown**: click "Select recipes" above the
  grid — a checklist of every recipe in the justfile, plus a "Show hidden
  recipes" toggle to include private (`_`-prefixed or `[private]`) ones in
  that list. Unchecked recipes just aren't in the grid at all — nothing sits
  there hidden-but-reserving-space.
- **Reorder**: click a button's ⇄ handle (top-right corner) to pick it up,
  then click another button to swap their positions. Click the picked-up
  button again to cancel.
- **Color a button**: click its 🎨 handle (top-left corner) and pick a
  swatch.

Selection, order, and color are saved automatically to
`justgui.layout.toml` next to the justfile — a separate file from
`justgui.toml` so that saving never touches (or reformats away the comments
in) your hand-edited theme config. It's fully managed by justgui: don't
hand-edit it while the app is running, and it's safe to delete to reset
back to showing every recipe in justfile order.

## Transparency and the theme editor

The "Transparency" slider blends the window background toward see-through
live. Whether that reads as "you can see the desktop behind it" or just "a
lighter/darker panel" depends on what your OS/window manager negotiates for
window compositing — worth a quick look on your setup rather than assumed.

The "Theme" button opens a panel with the same settings as
`justgui.toml`'s `[theme]` table (colors as swatches, dark/light mode,
corner radius, font size, font family) as live controls — changes apply to
the window immediately. Nothing is written to disk until you click "Save"
inside that panel, which overwrites `justgui.toml` (same tradeoff as the
"Edit justfile" tab's Save: a full-file rewrite, so any comments you've
added to `justgui.toml` by hand won't survive a Save here). Until you Save
or hit the top-level "Reload" button, live edits aren't clobbered by the
usual once-a-second config file watch (see [Styling](#styling)).

## Live output and input

Recipes run attached to a real pseudo-terminal (via
[`portable-pty`](https://docs.rs/portable-pty)), not just a plain pipe —
that's what makes `sudo`, `ssh`, and anything else that insists on a real
controlling terminal before prompting actually work here. The output panel
at the bottom streams a running recipe's output live. Below it, the input
field (enabled only while something is running) sends a line of text when
you press Enter or click "Send" — this is what lets a recipe that does
something like `read -p "Continue? [y/N] "`, or `sudo` asking for your
password, actually receive your answer. What you send shows up in the
output panel because the pty itself echoes it back, the same way a real
terminal would — justgui doesn't synthesize that.

- **Collapse it**: click the "Output" row to hide/show the panel.
- **Resize it**: drag the thin bar just above the panel up or down.

Output is plain text: ANSI color and cursor-control codes are stripped
rather than rendered (no colored text, and a full-screen/curses-style
recipe won't render correctly — this isn't a full terminal emulator), and
`\r`-based progress-bar redraws are collapsed down to just the latest line
rather than showing every intermediate update. A recipe's stdin stays open
for the life of the process instead of getting an immediate EOF, so a
recipe that reads-to-EOF without prompting (e.g. piping through `cat` with
no input) can hang where it wouldn't have before — click "Close input" to
send EOF and unstick it; there is currently no way to kill a running
process outright.

You can also point it at a different directory without `cd`-ing:

```sh
justgui /path/to/other/project
```

Or retarget it live, without restarting, using the "Directory" field and
"Reload" button at the top of the window.

## Styling

Copy [`justgui.example.toml`](justgui.example.toml) to `justgui.toml` and
adjust it — see the file for every available key (colors, dark/light mode,
corner radius, font). justgui looks for a config file in this order:

1. `justgui.toml` in the directory you pointed it at (per-project theme).
2. `~/.config/justgui/config.toml` (or `$XDG_CONFIG_HOME`/`%APPDATA%` on
   Windows) as a global default.
3. Built-in defaults if neither exists.

Any key you omit falls back to the default, so a config file can be as
small as one line. Changes to whichever file is active are picked up
automatically about once a second — no need to restart or hit Reload, just
save the file and watch the window update. You can also edit these same
settings from inside the running app — see
[Transparency and the theme editor](#transparency-and-the-theme-editor).

## Notes / limitations

- Editing the justfile writes the raw text back to disk with no validation;
  if the result doesn't parse, `just`'s error shows up in the Recipes tab.
- The recipe list is fetched fresh from `just` on load/reload, not
  file-watched — hit "Reload" after editing the justfile externally (this is
  separate from styling, which *is* watched live; see [Styling](#styling)).
- The recipe grid layout (`justgui.layout.toml`) is likewise only read on
  load/reload, not live-watched.
- The in-app theme editor's live edits are session-only until you click its
  "Save" button; the top-level "Reload" button discards them back to
  whatever's on disk (there's no separate "discard" action).
- The output/input panel is a plain text stream, not a real terminal — see
  [Live output and input](#live-output-and-input) for what that does and
  doesn't support.
