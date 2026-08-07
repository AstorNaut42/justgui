# justgui

A tiny, dependency-free GUI for [`just`](https://github.com/casey/just). It
reads a justfile and turns each recipe into a button, with input fields for
any parameters, live streamed output, and a plain-text editor for the
justfile itself.

`just` itself is used to introspect the justfile (`just --dump --dump-format
json`), so justgui never has to parse justfile syntax and stays correct as
`just` evolves.

## Requirements

- The `just` CLI, already on your `PATH` (you have this if you use justfiles).
- To build: a C++17 compiler, CMake ≥ 3.16, and network access on first
  configure (CMake `FetchContent` pulls GLFW and Dear ImGui source and
  builds them locally — nothing is installed system-wide).
- On Linux, GLFW needs X11/Wayland development headers to compile
  (`libx11-dev libxrandr-dev libxinerama-dev libxcursor-dev libxi-dev
  libxkbcommon-dev`, or your distro's Wayland equivalents). Most desktop
  dev machines already have these.

The resulting binary has **no runtime dependencies** beyond what's on any
desktop (libc, libstdc++, OpenGL) — no bundled browser, no installed
libraries, nothing to `pip install`.

## Build

```
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build -j
```

## Run

```
./build/justgui [directory]
```

Defaults to the current directory if none is given. Use the "Directory"
field at the top of the window (and "Reload") to point it at a different
justfile without restarting.

## Notes / limitations

- Parameters for variadic recipes (`*args` / `+args`) are entered as a
  single space-separated field — there's no shell-style quoting inside it,
  each whitespace-separated token becomes one argument.
- Editing writes the raw file back to disk with no validation; if the
  result doesn't parse, the error from `just` is shown in the Recipes tab.
- The recipe list is fetched from `just` fresh on load/reload, not
  file-watched — hit "Reload" after external edits.
