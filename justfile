@_default:
				just --list

# Configure the build (first run only, or after editing CMakeLists.txt)
configure:
    cmake -S . -B build -DCMAKE_BUILD_TYPE=Release

# Build justgui
build:
    cmake --build build -j

# Build and launch, optionally pointed at another directory's justfile
run dir=".":
    cmake --build build -j
    ./build/justgui {{dir}}

# Remove build output
clean:
    rm -rf build
