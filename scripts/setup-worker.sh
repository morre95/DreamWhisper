#!/usr/bin/env bash
set -euo pipefail
project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
python_program="${DREAMWHISPER_PYTHON:-python3.12}"
"$python_program" -m venv "$project_dir/.venv"
"$project_dir/.venv/bin/python" -m pip install -r "$project_dir/worker/requirements-linux-py312.lock"
printf '\nPythonmiljö klar: %s/.venv/bin/python\n' "$project_dir"
printf 'Hämta sedan KB-Whisper enligt README. Modellen laddas inte ner av detta skript.\n'
