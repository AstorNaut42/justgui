#include "just_client.h"

#include <algorithm>
#include <sstream>

#include "json.hpp"
#include "process.h"

JustModel load_justfile(const std::string& dir) {
    JustModel model;
    model.dir = dir;

    Process proc;
    if (!proc.start("just --dump --dump-format json", dir)) {
        model.error = "failed to launch `just`";
        return model;
    }
    proc.wait();
    std::string output;
    proc.poll(output);

    if (proc.exit_code() != 0) {
        model.error = output.empty() ? "`just` exited with an error" : output;
        return model;
    }

    json::Value root;
    try {
        root = json::parse(output);
    } catch (const std::exception& e) {
        model.error = std::string("could not parse `just` output: ") + e.what();
        return model;
    }

    if (const json::Value* src = root.find("source")) {
        model.justfile_path = src->as_string();
    }

    const json::Value* recipes = root.find("recipes");
    if (!recipes || !recipes->is_object()) {
        model.error = "unexpected `just --dump` output (no recipes object)";
        return model;
    }

    for (auto& kv : recipes->obj) {
        const json::Value& r = kv.second;
        JustRecipe recipe;
        recipe.name = r.find("name") ? r.find("name")->as_string() : kv.first;
        if (const json::Value* doc = r.find("doc")) recipe.doc = doc->as_string();
        if (const json::Value* priv = r.find("private")) recipe.is_private = priv->as_bool();

        if (const json::Value* params = r.find("parameters"); params && params->is_array()) {
            for (const json::Value& p : params->arr) {
                JustParam param;
                if (const json::Value* n = p.find("name")) param.name = n->as_string();
                if (const json::Value* def = p.find("default"); def && def->is_string()) {
                    param.has_default = true;
                    param.default_value = def->as_string();
                }
                if (const json::Value* kind = p.find("kind")) {
                    std::string k = kind->as_string();
                    param.variadic = (k == "star" || k == "plus");
                }
                recipe.params.push_back(std::move(param));
            }
        }
        model.recipes.push_back(std::move(recipe));
    }

    std::sort(model.recipes.begin(), model.recipes.end(),
              [](const JustRecipe& a, const JustRecipe& b) { return a.name < b.name; });

    return model;
}

std::string build_run_command(const JustRecipe& recipe,
                               const std::vector<std::string>& param_values) {
    std::ostringstream cmd;
    cmd << "just " << Process::shell_quote(recipe.name);

    size_t n = std::min(param_values.size(), recipe.params.size());

    // Recipe arguments are positional, so a default-valued param can only
    // be omitted if every param after it is also left at its default.
    // Find the last param that actually needs to be passed on the CLI.
    size_t last_needed = 0;
    bool any_needed = false;
    for (size_t i = 0; i < n; ++i) {
        const JustParam& param = recipe.params[i];
        bool overridden = param.variadic ? !param_values[i].empty()
                                          : (!param.has_default || param_values[i] != param.default_value);
        if (overridden) {
            last_needed = i;
            any_needed = true;
        }
    }
    if (!any_needed) return cmd.str();

    for (size_t i = 0; i <= last_needed; ++i) {
        const JustParam& param = recipe.params[i];
        const std::string& value = param_values[i];
        if (param.variadic) {
            std::istringstream tokens(value);
            std::string tok;
            while (tokens >> tok) cmd << ' ' << Process::shell_quote(tok);
        } else {
            const std::string& v = value.empty() && param.has_default ? param.default_value : value;
            cmd << ' ' << Process::shell_quote(v);
        }
    }
    return cmd.str();
}
