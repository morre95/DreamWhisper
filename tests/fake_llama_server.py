#!/usr/bin/env python3
"""CPU-only llama-server stand-in for lifecycle and request-contract tests."""
import argparse
import json
import os
from pathlib import Path
import time
from http.server import BaseHTTPRequestHandler, HTTPServer

parser = argparse.ArgumentParser()
parser.add_argument("--port", type=int)
parser.add_argument("--model")
parser.add_argument("--api-key")
args, _ = parser.parse_known_args()
config = json.loads(Path(args.model).read_text())
Path(config["pid"]).write_text(str(os.getpid()))
if config.get("whisper_pid"):
    pid_file = Path(config["whisper_pid"])
    if pid_file.exists() and Path("/proc", pid_file.read_text().strip()).exists():
        raise RuntimeError("Speech process was not released before drafting")
if config.get("scenario") == "crash":
    raise SystemExit(1)


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def answer(self, value):
        body = json.dumps(value).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        assert self.path == "/health"
        self.answer({"status": "ok"})

    def do_POST(self):
        assert self.headers["Authorization"] == "Bearer " + args.api_key
        data = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        if self.path == "/apply-template":
            self.answer({"prompt": json.dumps(data["messages"], ensure_ascii=False)})
        elif self.path == "/tokenize":
            self.answer({"tokens": [1] * (12289 if config.get("scenario") == "too_long" else 200)})
        else:
            assert self.path == "/v1/chat/completions"
            assert data["response_format"]["schema"]["properties"]
            assert data["chat_template_kwargs"]["enable_thinking"] is False
            if config.get("scenario") == "wait":
                time.sleep(600)
            user = json.loads(data["messages"][1]["content"])
            assert "reflections_explicitly_included" in user
            scene = {"title": "Skogen" if user["language"] == "sv" else "Forest",
                     "summary": "Scen" if user["language"] == "sv" else "Scene",
                     "details": [{"text": "Skog", "quote": user.get("description", ""), "english": "A forest indoors and outdoors"}],
                     "additions": [{"text": "Mjukt ljus", "quote": None, "english": "Soft light"}], "questions": []}
            if user['task'] == 'creative_additions':
                output = {"additions": scene["additions"]}
            elif user['task'] == 'combine_reviewed_scenes':
                scene['details'] = []
                scene['additions'] = []
                output = {"scenes": [scene], "arrangement": "Scen ett till vänster, scen två till höger", "arrangement_english": "Place scene one on the left and scene two on the right"}
            else:
                scene['additions'] = []
                output = {"scenes": [scene], "arrangement": ""}
            content = "broken json" if config.get("scenario") == "invalid" else json.dumps(output)
            self.answer({"choices": [{"finish_reason": "stop", "message": {"content": content}}]})


HTTPServer(("127.0.0.1", args.port), Handler).serve_forever()
