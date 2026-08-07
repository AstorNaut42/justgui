#include "process.h"

#ifdef _WIN32
#define POPEN _popen
#define PCLOSE _pclose
#else
#include <sys/wait.h>
#define POPEN popen
#define PCLOSE pclose
#endif

Process::~Process() {
    if (thread_.joinable()) thread_.join();
}

std::string Process::shell_quote(const std::string& arg) {
#ifdef _WIN32
    std::string out = "\"";
    for (char c : arg) {
        if (c == '"') out += '\\';
        out += c;
    }
    out += '"';
    return out;
#else
    std::string out = "'";
    for (char c : arg) {
        if (c == '\'')
            out += "'\\''";
        else
            out += c;
    }
    out += '\'';
    return out;
#endif
}

bool Process::start(const std::string& command, const std::string& cwd) {
    if (running_.load()) return false;
    if (thread_.joinable()) thread_.join();

    {
        std::lock_guard<std::mutex> lock(mutex_);
        buffer_.clear();
    }
    exit_code_ = 0;

#ifdef _WIN32
    std::string full = "cd /d " + shell_quote(cwd) + " && " + command + " 2>&1";
#else
    std::string full = "cd " + shell_quote(cwd) + " && " + command + " 2>&1";
#endif

    FILE* pipe = POPEN(full.c_str(), "r");
    if (!pipe) return false;

    running_.store(true);
    thread_ = std::thread(&Process::reader_main, this, pipe);
    return true;
}

void Process::reader_main(FILE* pipe) {
    char chunk[4096];
    while (fgets(chunk, sizeof(chunk), pipe)) {
        std::lock_guard<std::mutex> lock(mutex_);
        buffer_ += chunk;
    }
    int status = PCLOSE(pipe);
#ifndef _WIN32
    exit_code_ = WIFEXITED(status) ? WEXITSTATUS(status) : -1;
#else
    exit_code_ = status;
#endif
    running_.store(false);
}

void Process::wait() {
    if (thread_.joinable()) thread_.join();
}

bool Process::poll(std::string& out) {
    {
        std::lock_guard<std::mutex> lock(mutex_);
        if (!buffer_.empty()) {
            out += buffer_;
            buffer_.clear();
        }
    }
    return !running_.load();
}
