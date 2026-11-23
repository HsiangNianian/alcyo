#!/bin/sh
# On an Apple-silicon Mac: time llama-server and mistral.rs on the same recorded board requests, with the same model.
#   sh compare.sh <path to mistralrs-ios-spike> [model.gguf] [llama-server]
# The model defaults to the one Alcyo downloaded, llama-server to the one inside /Applications/Alcyo.app.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
spike=$1
model=${2:-"$HOME/Library/Application Support/app.alcyo.board/models/Qwen3-4B-Instruct-2507-Q4_K_M.gguf"}
server=${3:-/Applications/Alcyo.app/Contents/Resources/alcyo/llama/llama-server}
[ -f "$model" ] || { echo "no model at $model: download Qwen3-4B in Alcyo, or pass its path" >&2; exit 1; }
[ -x "$server" ] || { echo "no llama-server at $server: install Alcyo, or pass its path" >&2; exit 1; }
out="$here/results-$(date +%Y%m%d-%H%M)"
mkdir -p "$out"
chmod +x "$spike"

echo "== llama-server"
DYLD_LIBRARY_PATH=$(dirname "$server") "$server" -m "$model" --host 127.0.0.1 --port 18790 -c 6144 -np 1 --jinja --cache-ram 1024 -ngl 99 > "$out/llama-server.log" 2>&1 &
pid=$!
until curl -sf http://127.0.0.1:18790/health > /dev/null; do sleep 1; done
python3 "$here/llama_replay.py" http://127.0.0.1:18790 "$here/replay.jsonl" "$out/llama.jsonl" 2> "$out/llama-progress.log" | tee "$out/llama-summary.txt"
kill $pid; wait $pid 2>/dev/null || true

echo "== mistral.rs"
"$spike" "$(dirname "$model")" "$(basename "$model")" replay "$here/replay.jsonl" "$out/mistralrs.jsonl" 2> "$out/mistralrs-progress.log" | tee "$out/mistralrs-summary.txt"
echo "results in $out"
