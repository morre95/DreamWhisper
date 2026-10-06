#!/usr/bin/env python3
"""Force a small inference batch to test CUDA kernels even without a speech recording."""
import argparse
import json
from pathlib import Path
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from worker.worker import cuda_libraries


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--model", type=Path, default=ROOT / "models/kb-whisper-large")
    args = p.parse_args()
    cuda_libraries()
    import numpy as np
    from faster_whisper import WhisperModel, BatchedInferencePipeline
    started = time.monotonic()
    model = WhisperModel(str(args.model.resolve()), device="cuda", compute_type="float16", local_files_only=True)
    # Explicit clip bypasses speech detection to exercise encoder, decoder and timestamp alignment.
    segments, _ = BatchedInferencePipeline(model).transcribe(
        np.zeros(16000 * 3, dtype=np.float32), language="sv", task="transcribe",
        batch_size=8, word_timestamps=True, vad_filter=False,
        clip_timestamps=[{"start": 0.0, "end": 3.0}],
    )
    result = list(segments)
    print(json.dumps({"device": "cuda", "compute_type": "float16", "batch_size": 8,
                      "elapsed_seconds": round(time.monotonic() - started, 2),
                      "segments": len(result), "note": "Synthetic silence; this verifies GPU inference, not speech quality."}))


if __name__ == "__main__":
    main()
