#!/usr/bin/env python3
"""One persistent offline GPU worker. stdout is exclusively newline-delimited JSON."""
from __future__ import annotations

import argparse
import ctypes
import importlib.metadata
import json
import os
from pathlib import Path
import sys
import time
import traceback


def emit(event: str, **fields) -> None:
    print(json.dumps({"type": event, **fields}, ensure_ascii=False), flush=True)


def cuda_libraries() -> None:
    # Make pip-installed NVIDIA libraries discoverable before loading CTranslate2.
    import site
    directories = [str(Path(base) / "nvidia" / package / "lib")
                   for base in site.getsitepackages() for package in ("cublas", "cudnn")
                   if (Path(base) / "nvidia" / package / "lib").is_dir()]
    # CUDA's dynamically loaded sublibraries need the search path at process startup.
    if directories and os.environ.get("DREAMWHISPER_CUDA_PATH_READY") != "1":
        os.environ["LD_LIBRARY_PATH"] = ":".join(directories + [os.environ.get("LD_LIBRARY_PATH", "")])
        os.environ["DREAMWHISPER_CUDA_PATH_READY"] = "1"
        os.execv(sys.executable, [sys.executable, *sys.argv])
    for base in site.getsitepackages():
        nvidia = Path(base) / "nvidia"
        for package, name in (("cublas", "libcublasLt.so.12"), ("cublas", "libcublas.so.12"), ("cudnn", "libcudnn.so.9")):
            lib = nvidia / package / "lib" / name
            if lib.exists():
                ctypes.CDLL(str(lib), mode=ctypes.RTLD_GLOBAL)


def validated_request(value: dict) -> tuple[str, str, str]:
    if not isinstance(value, dict) or value.get("type") != "transcribe":
        raise ValueError("Expected a transcribe request")
    job_id, path = value.get("job_id"), value.get("path")
    if not isinstance(job_id, str) or not job_id:
        raise ValueError("Missing job_id")
    if not isinstance(path, str) or not Path(path).is_file():
        raise ValueError("Audio file does not exist")
    language = value.get("language", "sv")
    if language not in ("sv", "en"):
        raise ValueError("Expected language sv or en")
    return job_id, path, language


def transcribe(pipeline, path: str, batch_size: int, job_id: str, language: str = "sv") -> tuple[list, float]:
    segments, info = pipeline.transcribe(
        path, language=language, task="transcribe", batch_size=batch_size,
        word_timestamps=True, vad_filter=True, condition_on_previous_text=False,
    )
    result = []
    for segment in segments:
        result.append({
            "start": segment.start, "end": segment.end, "text": segment.text,
            "words": [{"start": w.start, "end": w.end, "word": w.word, "probability": w.probability}
                      for w in (segment.words or [])],
        })
        emit("progress", job_id=job_id, seconds=segment.end, duration=info.duration)
    return result, info.duration


def is_oom(exc: BaseException) -> bool:
    text = str(exc).lower()
    return "out of memory" in text or "cuda_error_out_of_memory" in text


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", required=True)
    parser.add_argument("--batch-size", type=int, default=8, choices=[1, 2, 4, 8, 16])
    args = parser.parse_args()
    model_path = Path(args.model).resolve()
    try:
        if not (model_path / "model.bin").is_file():
            raise RuntimeError("Modellen saknas. Kör worker/prepare_model.py först och välj den skapade modellmappen.")
        os.environ["HF_HUB_OFFLINE"] = "1"
        os.environ["TRANSFORMERS_OFFLINE"] = "1"
        cuda_libraries()
        from faster_whisper import BatchedInferencePipeline, WhisperModel
        model = WhisperModel(str(model_path), device="cuda", compute_type="float16", local_files_only=True)
        provenance_path = model_path / "dreamwhisper-model.json"
        provenance = json.loads(provenance_path.read_text()) if provenance_path.exists() else {"source": str(model_path), "revision": "unknown"}
        emit("ready")
    except Exception as exc:
        traceback.print_exc(file=sys.stderr)
        emit("fatal", error=str(exc))
        return 1

    for line in sys.stdin:
        request = {}
        try:
            request = json.loads(line)
            if request.get("type") == "shutdown":
                return 0
            job_id, path, language = validated_request(request)
            started = time.monotonic()
            batch_size = args.batch_size
            while True:
                try:
                    # Fresh pipeline for each attempt resets VAD/timestamp state.
                    result, duration = transcribe(BatchedInferencePipeline(model), path, batch_size, job_id, language)
                    break
                except RuntimeError as exc:
                    if not is_oom(exc) or batch_size == 1:
                        raise
                    batch_size //= 2
                    emit("progress", job_id=job_id, seconds=0, duration=0, message=f"GPU-minnet tog slut; försöker med batch {batch_size}")
            emit("completed", job_id=job_id, segments=result, metadata={
                "model": provenance, "compute_type": "float16", "device": "cuda", "language": language,
                "batch_size": batch_size, "duration": duration, "elapsed_seconds": time.monotonic() - started,
                "faster_whisper_version": importlib.metadata.version("faster-whisper"),
                "ctranslate2_version": importlib.metadata.version("ctranslate2"),
                "vad_filter": True, "word_timestamps": True,
            })
        except Exception as exc:
            traceback.print_exc(file=sys.stderr)
            emit("error", job_id=request.get("job_id") if isinstance(request, dict) else None, error=str(exc))
            # A broken CUDA context should be replaced rather than reused.
            if isinstance(exc, RuntimeError):
                return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
