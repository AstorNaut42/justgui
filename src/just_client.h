// Talks to the `just` CLI: introspects a justfile via `just --dump
// --dump-format json` and builds the shell command to run a recipe.
// `just` itself is the parser — we never read justfile syntax ourselves.
#pragma once

#include <string>
#include <vector>

struct JustParam {
    std::string name;
    std::string default_value;  // only meaningful if has_default
    bool has_default = false;
    bool variadic = false;  // `*args` / `+args`: zero-or-more / one-or-more
};

struct JustRecipe {
    std::string name;
    std::string doc;
    std::vector<JustParam> params;
    bool is_private = false;
};

struct JustModel {
    std::string dir;             // directory just was run in
    std::string justfile_path;   // resolved justfile path, if found
    std::vector<JustRecipe> recipes;
    std::string error;           // set if the dump failed (parse/run error)
};

// Runs `just --dump --dump-format json` in `dir` and parses the result.
// Blocks briefly; intended for use on load/reload, not every frame.
JustModel load_justfile(const std::string& dir);

// Builds a `just <recipe> <args...>` command line, shell-quoting each
// argument. `param_values[i]` corresponds to `recipe.params[i]`; variadic
// parameters are split on whitespace into multiple CLI arguments.
std::string build_run_command(const JustRecipe& recipe,
                               const std::vector<std::string>& param_values);
