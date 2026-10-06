#!/usr/bin/env python3
"""Explicit download of the official GGUF; repeat runs retain the recorded revision."""
import argparse
import hashlib
import json
from pathlib import Path
from huggingface_hub import HfApi, hf_hub_download

MODEL = "Qwen/Qwen3-8B-GGUF"
FILE = "Qwen3-8B-Q4_K_M.gguf"


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--revision", help="Exact commit, branch, or tag; existing revision is retained by default")
    parser.add_argument("--runtime-manifest", type=Path)
    args = parser.parse_args()
    manifest = args.output / "dreamwhisper-text-model.json"
    existing = json.loads(manifest.read_text()) if manifest.exists() else None
    if existing and existing.get("source") != MODEL:
        parser.error("Choose a separate directory for a different model")
    revision = args.revision or (existing["revision"] if existing else "main")
    if existing and not args.revision:
        file = args.output / FILE
        if file.is_file() and digest(file) == existing["sha256"]:
            print(f"Verified existing local model: {file.resolve()}")
            return
    commit = HfApi().model_info(MODEL, revision=revision).sha
    if existing and existing["revision"] != commit:
        parser.error("Choose a new directory for another revision")
    args.output.mkdir(parents=True, exist_ok=True)
    file = Path(hf_hub_download(MODEL, filename=FILE, revision=commit, local_dir=args.output))
    checksum = digest(file)
    if existing and checksum != existing["sha256"]:
        raise RuntimeError("Model checksum does not match the recorded revision")
    metadata = {"source": MODEL, "revision": commit, "file": FILE, "sha256": checksum}
    if args.runtime_manifest:
        metadata["runtime"] = json.loads(args.runtime_manifest.read_text())
    temporary = manifest.with_suffix(".tmp")
    temporary.write_text(json.dumps(metadata, indent=2) + "\n")
    temporary.replace(manifest)
    print(f"Model ready: {file.resolve()}\nRevision: {commit}\nSHA-256: {checksum}")


if __name__ == "__main__":
    main()
