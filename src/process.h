// Runs a shell command in the background and streams its combined
// stdout+stderr back to the caller. Backed by popen(), so it works
// unmodified on Linux, macOS and Windows without extra dependencies.
#pragma once

#include <atomic>
#include <cstdio>
#include <mutex>
#include <string>
#include <thread>

class Process {
public:
    ~Process();

    // Starts `command` (a full shell command line) with working directory
    // `cwd`. Returns false if a process is already running.
    bool start(const std::string& command, const std::string& cwd);

    // Blocks until the process finishes. Used for short-lived, synchronous
    // calls (e.g. `just --dump`) where we want the result immediately.
    void wait();

    // Non-blocking: moves any output collected since the last call into
    // `out` (appending). Returns true if the process has exited.
    bool poll(std::string& out);

    bool running() const { return running_.load(); }
    int exit_code() const { return exit_code_; }

    // Quotes `arg` so it survives being passed through /bin/sh -c (POSIX)
    // or cmd.exe /c (Windows) as a single token.
    static std::string shell_quote(const std::string& arg);

private:
    void reader_main(FILE* pipe);

    std::thread thread_;
    std::mutex mutex_;
    std::string buffer_;
    std::atomic<bool> running_{false};
    int exit_code_ = 0;
};
