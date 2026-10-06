#!/usr/bin/env python3
"""Explicit, one-time download. Pins the resolved model commit in the local archive."""
import argparse
import json
from pathlib import Path
from huggingface_hub import HfApi, snapshot_download


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--revision", default="main", help="A branch, tag, or exact model commit")
    args = p.parse_args()
    model_id = "KBLab/kb-whisper-large"
    commit = HfApi().model_info(model_id, revision=args.revision).sha
    args.output.mkdir(parents=True, exist_ok=True)
    existing = args.output / "dreamwhisper-model.json"
    if existing.exists() and json.loads(existing.read_text())["revision"] != commit:
        p.error("Mappen innehåller en annan modellrevision. Välj en ny tom mapp för denna revision.")
    snapshot_download(model_id, revision=commit, local_dir=args.output,
                      allow_patterns=["model.bin", "config.json", "tokenizer.json", "preprocessor_config.json", "vocabulary.*"])
    if not (args.output / "model.bin").is_file():
        raise RuntimeError("Den valda revisionen innehåller ingen CTranslate2-modell")
    existing.write_text(json.dumps({"source": model_id, "revision": commit}, indent=2) + "\n")
    print(f"Modellen är klar: {args.output.resolve()}\nRevision: {commit}")


if __name__ == "__main__":
    main()
