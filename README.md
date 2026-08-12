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

- **Recipes tab**: one entry per recipe, with its doc comment, an input
  field for each parameter (pre-filled with its default value if it has
  one), and a "Run" button. Output streams live below as the recipe runs.
  Variadic parameters (`*args` / `+args`) are entered as a single
  space-separated field.
- **Edit justfile tab**: the raw justfile as text, with Save and "Reload
  from disk" buttons.

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
save the file and watch the window update.

## Notes / limitations

- Editing the justfile writes the raw text back to disk with no validation;
  if the result doesn't parse, `just`'s error shows up in the Recipes tab.
- The recipe list is fetched fresh from `just` on load/reload, not
  file-watched — hit "Reload" after editing the justfile externally (this is
  separate from styling, which *is* watched live; see [Styling](#styling)).
