@_default:
    just --list

# Build a release binary (target/release/justgui)
build:
    cargo build --release

# Install `justgui` onto your PATH via `cargo install` (~/.cargo/bin)
install:
    cargo install --path . --locked

# Remove the installed `justgui` binary
uninstall:
    cargo uninstall justgui

# Build and run against a directory (defaults to the current one), without installing
run dir=".":
    cargo run --release -- {{dir}}

# Run the test suite
test:
    cargo test

# Remove build artifacts
clean:
    cargo clean
