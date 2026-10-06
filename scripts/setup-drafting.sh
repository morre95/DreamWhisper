#!/usr/bin/env bash
# Explicit one-time network/build setup. Never run by the inference worker.
set -euo pipefail
task_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
task_checkout="$task_root/.local/llama.cpp"
task_manifest="$task_root/.local/llama-runtime.json"
task_python="$task_root/.venv/bin/python"
if [[ ! -x "$task_python" ]]; then
  echo 'Run bash scripts/setup-worker.sh first to create the Python environment.' >&2
  exit 1
fi
for task_command in git cmake nvcc; do
  command -v "$task_command" >/dev/null || { echo "Missing $task_command; install the CUDA toolkit and build tools first." >&2; exit 1; }
done
mkdir -p "$task_root/.local"
if [[ ! -d "$task_checkout" ]]; then
  git clone --depth 1 https://github.com/ggml-org/llama.cpp.git "$task_checkout"
fi
if [[ ! -d "$task_checkout/.git" ]] || [[ "$(git -C "$task_checkout" remote get-url origin)" != 'https://github.com/ggml-org/llama.cpp.git' ]]; then
  echo 'The runtime directory is not the managed llama.cpp checkout. Choose a separate directory.' >&2
  exit 1
fi
task_commit="$(git -C "$task_checkout" rev-parse HEAD)"
if [[ -f "$task_manifest" ]]; then
  task_pinned="$("$task_python" -c 'import json,sys; print(json.load(open(sys.argv[1]))["revision"])' "$task_manifest")"
  if [[ "$task_commit" != "$task_pinned" ]]; then
    echo 'The runtime checkout changed from the pinned revision. Restore that revision or use a separate setup.' >&2
    exit 1
  fi
fi
if [[ -n "$(git -C "$task_checkout" status --porcelain)" ]]; then
  echo 'The runtime checkout contains changes. Preserve them before reinstalling.' >&2
  exit 1
fi
cmake -S "$task_checkout" -B "$task_checkout/build" -DGGML_CUDA=ON -DLLAMA_CURL=OFF -DCMAKE_BUILD_TYPE=Release
cmake --build "$task_checkout/build" --config Release --target llama-server -j 4
task_binary="$task_checkout/build/bin/llama-server"
# Verify the API/flags used by DreamWhisper before publishing a successful setup.
task_help="$("$task_binary" --help)"
for task_flag in '--reasoning' '--jinja' '--api-key' '--no-context-shift'; do
  [[ "$task_help" == *"$task_flag"* ]] || { echo "Runtime lacks $task_flag. Use a compatible llama.cpp revision." >&2; exit 1; }
done
"$task_python" - "$task_manifest" "$task_commit" "$task_binary" <<'PY'
import hashlib, json, pathlib, subprocess, sys
manifest, revision, binary = sys.argv[1:]
with open(binary, 'rb') as stream:
    checksum = hashlib.file_digest(stream, 'sha256').hexdigest()
version = subprocess.run([binary, '--version'], text=True, capture_output=True, check=True)
data = {'source': 'https://github.com/ggml-org/llama.cpp', 'revision': revision,
        'binary_sha256': checksum, 'version': (version.stdout + version.stderr).strip()}
target = pathlib.Path(manifest)
temporary = target.with_suffix('.tmp')
temporary.write_text(json.dumps(data, indent=2) + '\n')
temporary.replace(target)
PY
"$task_python" "$task_root/worker/prepare_text_model.py" --output "$task_root/models/qwen3-8b" --runtime-manifest "$task_manifest"
echo "Setup ready. Select $task_binary and $task_root/models/qwen3-8b/Qwen3-8B-Q4_K_M.gguf under Settings."
