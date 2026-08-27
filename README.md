# justgui

A native, dependency-light GUI for [`just`](https://github.com/casey/just). Point
it at a directory with a `justfile` and it turns every recipe into a button:
input fields for parameters, live streamed output when you run one, and a
plain-text editor for the justfile itself.

`just` itself is used to introspect the justfile (`just --dump --dump-format
json`), so justgui never parses justfile syntax and stays correct as `just`
evolves.

This is a Rust + [Slint](https://slint.dev) rewrite of an earlier C++/Dear
ImGui prototype (kept on the `justgui-cpp` branch for reference). Colors,
fonts and corner radius are all driven by a small `justgui.toml` config
file, and are
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

From this directory:

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
- **Edit justfile tab**: the raw justfile as text — see
  [Editing the justfile](#editing-the-justfile).

A "Transparency" slider, a "Theme" button, and a "⚙ Settings" button sit
above the tabs — see [Transparency and the theme editor](#transparency-and-the-theme-editor)
and [Settings (.env)](#settings-env).

Every recipe you run gets its own session, so several can run at once —
see [Live output and input](#live-output-and-input). Output from whichever
session is active streams live in the panel at the bottom of the window,
with an input field below it for recipes that prompt for confirmation or
other input.

## The recipe grid

Recipes that take no parameters are plain buttons showing just their name,
in a grid that reflows to fit however wide you make the window. Recipes
that take one or more parameters get a full-width card below the grid
instead, with a field per parameter shown inline (pre-filled with its
default, if any) and a "Run" button — nothing is hidden behind a popup, so
it's always visible what a recipe expects. Leaving an optional parameter
blank and clicking Run is also how you hand control to a recipe that
prompts for it interactively itself (e.g. a shebang recipe that falls back
to a `select` menu when its parameter is empty) — the input box below the
output panel is what you use to answer that prompt once it appears, same
as any other running recipe. Variadic parameters (`*args` / `+args`) are
entered as a single space-separated field.

- **See what a grid button does**: hover over it for its doc comment as a
  tooltip (parameter cards show the doc comment directly, no hover needed).
- **See that it's running**: a button or card darkens while it has an active
  session, and turns a warning color if that session looks like it's
  blocked on a prompt (see [Live output and input](#live-output-and-input)).
- **Choose which recipes are shown**: click "Select recipes" above the
  grid — a checklist of every recipe in the justfile, plus a "Show hidden
  recipes" toggle to include private (`_`-prefixed or `[private]`) ones in
  that list. Unchecked recipes just aren't shown at all — nothing sits
  there hidden-but-reserving-space.
- **Reorder**: click a button's or card's ⇄ handle to pick it up, then click
  another one to swap their positions. Click the picked-up one again to
  cancel.
- **Color a button or card**: click its 🎨 handle and pick a swatch.

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

## Settings (.env)

The "⚙ Settings" button opens a panel over the project's `.env` file (next
to the justfile) — a place for the kind of project-level toggles/values a
recipe reads (a feature flag, a target host, a port), edited from the GUI
instead of by hand. justgui doesn't invent its own config format for this:
`.env` is what most projects already use for exactly this, and `just` has
native support for loading it (`set dotenv-load := true` in the justfile) —
so justgui only has to read and write the file, and `just` does the actual
work of getting those values into a recipe's environment.

Each line becomes a row: a value that's exactly `true` or `false` (any
case) shows as a toggle — flip it and it saves immediately. Anything else —
text or a number — shows as a plain text field: type your change, then
either press Enter or click its ✓ to save, or press Esc to discard it and
revert to the saved value (a text field doesn't save on every keystroke,
so it doesn't fight you for focus while you're mid-edit). Type a new key
into the bottom field and click "Add" for a new entry (starts out as an
empty text field — type `true`/`false` into it and it becomes a toggle the
next time the panel refreshes); click a row's × to remove it. Comments and
blank lines already in a hand-edited `.env` survive being edited through
the panel — only the specific `KEY=VALUE` lines you touch change.

Recipes only actually see these values if the justfile opts in with `set
dotenv-load := true` (or `set dotenv-required := true`) — that's `just`'s
own setting, not something justgui adds for you, so add it yourself if you
want recipes reading `.env` values via `{{env_var('KEY')}}` or as inherited
environment variables.

## Live output and input

Recipes run attached to a real pseudo-terminal (via
[`portable-pty`](https://docs.rs/portable-pty)), not just a plain pipe —
that's what makes `sudo`, `ssh`, and anything else that insists on a real
controlling terminal before prompting actually work here. The output panel
at the bottom streams a running recipe's output live. Below it, the input
field (enabled only while the active session is running) sends a line of
text when you press Enter or click "Send" — this is what lets a recipe
that does something like `read -p "Continue? [y/N] "`, or `sudo` asking
for your password, actually receive your answer. What you send shows up
in the output panel because the pty itself echoes it back, the same way a
real terminal would — justgui doesn't synthesize that.

Since output can't tell you *when* a recipe is actually blocked waiting on
a reply, justgui makes a best-effort guess: if a session's output ends on
an unterminated line that mentions "password"/"passphrase", or ends in
`:`, `?`, `>` or `]` (the shapes a password prompt, a `[y/N]` confirmation,
or a `select`-menu prompt tend to take), that session's tab, its recipe's
grid button/card, and the input row all switch to a warning color with a
short "may be waiting for input" note. It's a heuristic over plain text,
not real terminal-state introspection, so it can be wrong in both
directions — treat it as a hint to go check, not a guarantee.

- **Collapse it**: click the "Output" row to hide/show the panel.
- **Resize it**: drag the thin bar just above the panel up or down.

Every time you run a recipe it gets its own session, so a recipe that
needs to stay running (starting a background service, an AppImage, a dev
server) doesn't block you from running anything else. A row of session
tabs appears above the output panel once you've run something:

- **Switch which one you're looking at**: click a tab. A dot marks the
  ones still running.
- **Dismiss one**: click its ×. This only stops justgui from tracking and
  showing it — it does **not** kill the underlying process if it's still
  running (there's currently no way to kill a running process outright).
- **Finished ones clean themselves up**: once a session's recipe exits and
  you're no longer looking at it (you've switched to another tab, or
  started a new recipe), its tab disappears on its own — there's no need
  to manually dismiss every one-off recipe you run. The session you're
  currently viewing is the only exception: it sticks around, exit code and
  all, until you switch away or close it yourself.

Output is plain text: ANSI color and cursor-control codes are stripped
rather than rendered (no colored text, and a full-screen/curses-style
recipe won't render correctly — this isn't a full terminal emulator), and
`\r`-based progress-bar redraws are collapsed down to just the latest line
rather than showing every intermediate update. A recipe's stdin stays open
for the life of the process instead of getting an immediate EOF, so a
recipe that reads-to-EOF without prompting (e.g. piping through `cat` with
no input) can hang where it wouldn't have before — click "Close input" to
send EOF and unstick it.

You can also point it at a different directory without `cd`-ing:

```sh
justgui /path/to/other/project
```

Or retarget it live, without restarting, using the "Directory" field and
"Reload" button at the top of the window.

## Editing the justfile

The "Edit justfile" tab is a plain text editor for the raw justfile, with
Save and "Reload from disk" buttons. As you type, justgui pipes the
*unsaved* buffer through `just` on a debounce timer and shows a parse
error inline if it doesn't parse — the same way `just --dump` is already
used everywhere else, just against your in-progress edit instead of what's
on disk.

There's no vim mode or other modal editing here — Slint's text editor
widget doesn't have hooks for that, and there's no existing crate to build
one from, so it isn't a small addition. Instead, "Edit externally" opens
the justfile in your `$VISUAL`/`$EDITOR` (same convention `just --edit`
itself uses) inside a separate terminal window, so a real editor — vim,
whatever you use — handles it natively with no rendering limitations, and
you use "Reload from disk" here once you save and quit. Justgui tries a
short list of common Linux terminal emulators
(`x-terminal-emulator`/`gnome-terminal`/`konsole`/`xterm`) and a
`cmd`-based fallback on Windows; if none of those are found it silently
does nothing. macOS isn't covered.

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

- Live linting (see [Editing the justfile](#editing-the-justfile)) warns
  you as you type, but Save still writes whatever's in the buffer
  regardless — it doesn't block saving invalid syntax.
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
