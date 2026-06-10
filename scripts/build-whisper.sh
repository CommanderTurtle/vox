#!/bin/bash
# Build whisper.cpp for vox (requires libclang)
# Usage: bash scripts/build-whisper.sh
set -euo pipefail

echo "=== Downloading whisper.cpp ==="
WHISPER_VERSION="1.7.4"
WHISPER_DIR="extern/whisper.cpp"

if [ ! -d "$WHISPER_DIR" ]; then
    mkdir -p extern
    git clone --depth 1 --branch "v$WHISPER_VERSION" https://github.com/ggerganov/whisper.cpp.git "$WHISPER_DIR"
fi

echo "=== Building whisper.cpp ==="
cd "$WHISPER_DIR"
cmake -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build --config Release -j4
cd ../..

echo "=== Downloading tiny model ==="
MODEL_DIR="models"
mkdir -p "$MODEL_DIR"
if [ ! -f "$MODEL_DIR/ggml-tiny.bin" ]; then
    curl -L "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin" -o "$MODEL_DIR/ggml-tiny.bin"
fi

echo "=== Building vox with whisper-local ==="
cargo build --release --features whisper-local

echo "=== Done ==="
echo "Model path: $(pwd)/$MODEL_DIR/ggml-tiny.bin"
echo "Set 'model_path' in config.toml to the above path to use whisper-local."
