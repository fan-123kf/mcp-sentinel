"""Measure serialized tool-list token counts using an explicitly named tokenizer.

Requires tiktoken. Counts tool definitions only; excludes model-specific API
wrappers, search responses, subsequent invocations, and conversation content.
"""
import argparse
import json
import hashlib
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("backend", type=Path)
parser.add_argument("meta", type=Path)
parser.add_argument("--encoding", default="cl100k_base")
parser.add_argument("--output", type=Path, required=True)
args = parser.parse_args()
try:
    import tiktoken
except ImportError:
    parser.error("Install tiktoken to measure tokens; byte counts are not a substitute")
encoding = tiktoken.get_encoding(args.encoding)

def count(path):
    definitions = json.loads(path.read_text(encoding="utf-8"))
    serialized = json.dumps(definitions, ensure_ascii=False, separators=(",", ":"))
    return len(encoding.encode(serialized, disallowed_special=()))

backend, meta = count(args.backend), count(args.meta)
report = {"tiktoken_version": tiktoken.__version__,
          "backend_sha256": hashlib.sha256(args.backend.read_bytes()).hexdigest(),
          "meta_sha256": hashlib.sha256(args.meta.read_bytes()).hexdigest(),
          "encoding": args.encoding, "backend_tokens": backend, "meta_tokens": meta,
          "reduction": 1 - meta / backend if backend else None,
          "scope": "serialized initial tool-list definitions only"}
args.output.write_text(json.dumps(report, indent=2), encoding="utf-8")
print(json.dumps(report, indent=2))
