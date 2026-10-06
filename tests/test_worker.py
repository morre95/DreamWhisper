"""Protocol tests with a fake inference backend; no model download or GPU required."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("worker", ROOT / "worker" / "worker.py")
worker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(worker)


class WorkerTests(unittest.TestCase):
    def test_json_protocol_preserves_swedish(self):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            worker.emit("error", error="Återförsök önskas")
        self.assertEqual(json.loads(output.getvalue())["error"], "Återförsök önskas")

    def test_bad_request_rejected(self):
        for value in ([], {}, {"type": "transcribe", "job_id": "x", "path": "/missing"}):
            with self.assertRaises(ValueError):
                worker.validated_request(value)

    def test_language_validation_and_legacy_default(self):
        with tempfile.NamedTemporaryFile() as audio:
            request = {"type": "transcribe", "job_id": "job", "path": audio.name}
            self.assertEqual(worker.validated_request(request)[2], "sv")
            self.assertEqual(worker.validated_request({**request, "language": "en"})[2], "en")
            with self.assertRaises(ValueError):
                worker.validated_request({**request, "language": "de"})

    def test_oom_detection(self):
        self.assertTrue(worker.is_oom(RuntimeError("CUDA failed with out of memory")))
        self.assertFalse(worker.is_oom(RuntimeError("file unreadable")))

    def test_missing_model_fails_before_claiming_work(self):
        with tempfile.TemporaryDirectory() as directory:
            result = subprocess.run([sys.executable, str(ROOT / "worker" / "worker.py"), "--model", directory],
                                    input="", text=True, capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 1)
        self.assertEqual(json.loads(result.stdout)["type"], "fatal")

    def test_persistent_worker_batch_fallback_and_job_ids(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            model = base / "model"
            model.mkdir()
            (model / "model.bin").write_bytes(b"fake")
            (model / "dreamwhisper-model.json").write_text(json.dumps({"source": "test", "revision": "abc"}))
            audio = base / "note.wav"
            audio.write_bytes(b"fake audio")
            (base / "faster_whisper.py").write_text('''
from types import SimpleNamespace as N
class WhisperModel:
    def __init__(self, *args, **kwargs):
        assert kwargs['device'] == 'cuda'
        assert kwargs['compute_type'] == 'float16'
        assert kwargs['local_files_only'] is True
class BatchedInferencePipeline:
    def __init__(self, model): pass
    def transcribe(self, path, **kwargs):
        assert kwargs['language'] in ('sv', 'en')
        assert kwargs['word_timestamps'] is True
        if kwargs['batch_size'] > 2: raise RuntimeError('CUDA out of memory')
        return iter([N(start=0.0, end=1.0, text='Hej världen' if kwargs['language'] == 'sv' else 'Hello world', words=[N(start=0.0, end=1.0, word='Hej', probability=.9)])]), N(duration=1.0)
''')
            for package in ("faster_whisper", "ctranslate2"):
                dist = base / f"{package}-1.0.dist-info"
                dist.mkdir()
                (dist / "METADATA").write_text(f"Name: {package.replace('_', '-')}\nVersion: 1.0\n")
            requests = [{"type": "transcribe", "job_id": f"job-{i}", "path": str(audio), "language": "sv" if i == 1 else "en"} for i in (1, 2)]
            requests.append({"type": "shutdown"})
            env = {**os.environ, "PYTHONPATH": str(base)}
            result = subprocess.run([sys.executable, str(ROOT / "worker" / "worker.py"), "--model", str(model)],
                                    input="\n".join(json.dumps(r) for r in requests) + "\n", env=env,
                                    text=True, capture_output=True, timeout=20)
            self.assertEqual(result.returncode, 0, result.stderr)
            messages = [json.loads(line) for line in result.stdout.splitlines()]
            self.assertEqual(messages[0]["type"], "ready")
            completed = [m for m in messages if m["type"] == "completed"]
            self.assertEqual([m["job_id"] for m in completed], ["job-1", "job-2"])
            self.assertEqual(completed[0]["metadata"]["batch_size"], 2)
            self.assertEqual(completed[0]["segments"][0]["text"], "Hej världen")
            self.assertEqual(completed[0]["metadata"]["model"]["revision"], "abc")
            self.assertEqual(completed[1]["metadata"]["language"], "en")
            self.assertEqual(completed[1]["segments"][0]["text"], "Hello world")


if __name__ == "__main__":
    unittest.main()
