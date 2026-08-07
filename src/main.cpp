// justgui — a tiny cross-platform GUI shell around `just`.
// Reads a justfile via `just --dump --dump-format json`, renders one
// button per recipe (with input fields for its parameters), streams the
// recipe's output live, and offers a plain-text editor for the justfile.
#include <GLFW/glfw3.h>
#include <imgui.h>
#include <backends/imgui_impl_glfw.h>
#include <backends/imgui_impl_opengl3.h>
#include <misc/cpp/imgui_stdlib.h>

#include <cstdio>
#include <filesystem>
#include <fstream>
#include <sstream>
#include <string>
#include <vector>

#include "just_client.h"
#include "process.h"

namespace fs = std::filesystem;

struct AppState {
    std::string dir;
    JustModel model;
    std::vector<std::vector<std::string>> param_values;  // per recipe, per param
    bool show_private = false;

    Process run_proc;
    std::string run_log;
    std::string running_recipe;  // non-empty while `run_proc` output belongs to a run

    std::string edit_buffer;
    bool edit_dirty = false;
    std::string edit_status;
};

static void reload(AppState& app) {
    app.model = load_justfile(app.dir);

    app.param_values.clear();
    app.param_values.resize(app.model.recipes.size());
    for (size_t i = 0; i < app.model.recipes.size(); ++i) {
        const JustRecipe& recipe = app.model.recipes[i];
        app.param_values[i].resize(recipe.params.size());
        for (size_t j = 0; j < recipe.params.size(); ++j)
            if (recipe.params[j].has_default) app.param_values[i][j] = recipe.params[j].default_value;
    }

    app.edit_dirty = false;
    app.edit_status.clear();
    app.edit_buffer.clear();
    if (!app.model.justfile_path.empty()) {
        std::ifstream f(app.model.justfile_path, std::ios::binary);
        if (f) {
            std::ostringstream ss;
            ss << f.rdbuf();
            app.edit_buffer = ss.str();
        }
    }
}

static void run_recipe(AppState& app, size_t idx) {
    if (app.run_proc.running()) return;
    const JustRecipe& recipe = app.model.recipes[idx];
    std::string cmd = build_run_command(recipe, app.param_values[idx]);
    app.run_log = "$ " + cmd + "\n";
    app.running_recipe = recipe.name;
    app.run_proc.start(cmd, app.dir);
}

static void save_edit(AppState& app) {
    if (app.model.justfile_path.empty()) {
        app.edit_status = "no justfile path known; cannot save";
        return;
    }
    std::ofstream f(app.model.justfile_path, std::ios::binary | std::ios::trunc);
    if (!f) {
        app.edit_status = "failed to write " + app.model.justfile_path;
        return;
    }
    f << app.edit_buffer;
    f.close();

    std::string saved = app.edit_buffer;
    reload(app);
    app.edit_buffer = saved;
    app.edit_dirty = false;
    app.edit_status = app.model.error.empty() ? "saved" : "saved, but justfile now fails to parse";
}

static void draw_log(AppState& app) {
    std::string chunk;
    bool finished = app.run_proc.poll(chunk);
    app.run_log += chunk;
    if (finished && !app.running_recipe.empty()) {
        app.run_log += "\n[exit code " + std::to_string(app.run_proc.exit_code()) + "]\n";
        app.running_recipe.clear();
    }

    ImGui::Text("Output%s", app.run_proc.running() ? " (running...)" : "");
    ImGui::SameLine();
    if (ImGui::SmallButton("Clear")) app.run_log.clear();

    ImGui::BeginChild("log", ImVec2(0, 0), true, ImGuiWindowFlags_HorizontalScrollbar);
    ImGui::TextUnformatted(app.run_log.c_str(), app.run_log.c_str() + app.run_log.size());
    if (ImGui::GetScrollY() >= ImGui::GetScrollMaxY() - 1.0f) ImGui::SetScrollHereY(1.0f);
    ImGui::EndChild();
}

static void draw_recipes_tab(AppState& app) {
    if (!app.model.error.empty()) {
        ImGui::TextColored(ImVec4(1, 0.4f, 0.4f, 1), "Could not load justfile:");
        ImGui::TextWrapped("%s", app.model.error.c_str());
        return;
    }
    if (app.model.recipes.empty()) {
        ImGui::TextDisabled("No recipes found in this directory.");
        return;
    }

    ImGui::Checkbox("Show private recipes", &app.show_private);
    ImGui::Separator();

    ImGui::BeginChild("recipe_list", ImVec2(0, ImGui::GetContentRegionAvail().y * 0.55f), true);
    for (size_t i = 0; i < app.model.recipes.size(); ++i) {
        const JustRecipe& recipe = app.model.recipes[i];
        if (recipe.is_private && !app.show_private) continue;

        ImGui::PushID(static_cast<int>(i));
        ImGui::TextColored(ImVec4(0.45f, 0.8f, 1.0f, 1.0f), "%s", recipe.name.c_str());
        if (!recipe.doc.empty()) {
            ImGui::SameLine();
            ImGui::TextDisabled("- %s", recipe.doc.c_str());
        }

        for (size_t j = 0; j < recipe.params.size(); ++j) {
            const JustParam& param = recipe.params[j];
            std::string label = param.name;
            if (param.variadic)
                label += " (space-separated)";
            else if (param.has_default)
                label += " [" + param.default_value + "]";
            else
                label += " (required)";
            ImGui::SetNextItemWidth(240);
            ImGui::InputText(label.c_str(), &app.param_values[i][j]);
        }

        ImGui::BeginDisabled(app.run_proc.running());
        if (ImGui::Button("Run")) run_recipe(app, i);
        ImGui::EndDisabled();

        ImGui::Separator();
        ImGui::PopID();
    }
    ImGui::EndChild();

    draw_log(app);
}

static void draw_editor_tab(AppState& app) {
    ImGui::TextDisabled("%s", app.model.justfile_path.empty() ? "(no justfile found)"
                                                                : app.model.justfile_path.c_str());
    if (app.edit_dirty) {
        ImGui::SameLine();
        ImGui::TextColored(ImVec4(1, 0.7f, 0.2f, 1), "(unsaved changes)");
    }

    ImGui::BeginDisabled(app.model.justfile_path.empty());
    if (ImGui::Button("Save")) save_edit(app);
    ImGui::EndDisabled();
    ImGui::SameLine();
    if (ImGui::Button("Reload from disk")) reload(app);
    if (!app.edit_status.empty()) {
        ImGui::SameLine();
        ImGui::TextDisabled("%s", app.edit_status.c_str());
    }

    ImGui::Separator();
    ImVec2 avail = ImGui::GetContentRegionAvail();
    if (ImGui::InputTextMultiline("##editor", &app.edit_buffer, ImVec2(-1, avail.y),
                                   ImGuiInputTextFlags_AllowTabInput)) {
        app.edit_dirty = true;
    }
}

int main(int argc, char** argv) {
    AppState app;
    app.dir = (argc > 1) ? argv[1] : fs::current_path().string();

    glfwSetErrorCallback([](int err, const char* desc) { fprintf(stderr, "GLFW error %d: %s\n", err, desc); });
    if (!glfwInit()) return 1;

    glfwWindowHint(GLFW_CONTEXT_VERSION_MAJOR, 3);
    glfwWindowHint(GLFW_CONTEXT_VERSION_MINOR, 0);

    GLFWwindow* window = glfwCreateWindow(1000, 700, "justgui", nullptr, nullptr);
    if (!window) {
        glfwTerminate();
        return 1;
    }
    glfwMakeContextCurrent(window);
    glfwSwapInterval(1);

    IMGUI_CHECKVERSION();
    ImGui::CreateContext();
    ImGuiIO& io = ImGui::GetIO();
    io.IniFilename = nullptr;  // no layout file on disk; keep the tool stateless
    ImGui::StyleColorsDark();

    ImGui_ImplGlfw_InitForOpenGL(window, true);
    ImGui_ImplOpenGL3_Init("#version 130");

    reload(app);

    while (!glfwWindowShouldClose(window)) {
        glfwPollEvents();

        ImGui_ImplOpenGL3_NewFrame();
        ImGui_ImplGlfw_NewFrame();
        ImGui::NewFrame();

        ImGuiViewport* viewport = ImGui::GetMainViewport();
        ImGui::SetNextWindowPos(viewport->WorkPos);
        ImGui::SetNextWindowSize(viewport->WorkSize);
        ImGui::Begin("justgui", nullptr,
                      ImGuiWindowFlags_NoDecoration | ImGuiWindowFlags_NoMove | ImGuiWindowFlags_NoResize |
                          ImGuiWindowFlags_NoBringToFrontOnFocus);

        ImGui::SetNextItemWidth(520);
        ImGui::InputText("Directory", &app.dir);
        ImGui::SameLine();
        if (ImGui::Button("Reload")) reload(app);

        ImGui::Separator();

        if (ImGui::BeginTabBar("tabs")) {
            if (ImGui::BeginTabItem("Recipes")) {
                draw_recipes_tab(app);
                ImGui::EndTabItem();
            }
            if (ImGui::BeginTabItem("Edit justfile")) {
                draw_editor_tab(app);
                ImGui::EndTabItem();
            }
            ImGui::EndTabBar();
        }

        ImGui::End();

        ImGui::Render();
        int display_w, display_h;
        glfwGetFramebufferSize(window, &display_w, &display_h);
        glViewport(0, 0, display_w, display_h);
        glClearColor(0.1f, 0.1f, 0.1f, 1.0f);
        glClear(GL_COLOR_BUFFER_BIT);
        ImGui_ImplOpenGL3_RenderDrawData(ImGui::GetDrawData());
        glfwSwapBuffers(window);
    }

    ImGui_ImplOpenGL3_Shutdown();
    ImGui_ImplGlfw_Shutdown();
    ImGui::DestroyContext();
    glfwDestroyWindow(window);
    glfwTerminate();
    return 0;
}
