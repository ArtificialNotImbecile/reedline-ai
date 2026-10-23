#!/usr/bin/env python3
"""Regenerate the synthetic benchmark manifest offline with DeepSeek's tokenizer.

Requires the optional `tokenizers` Python package and a local tokenizer.json.
No network, API credentials, user files, or command history are accessed.
The tokenizer is data, not imported Python code. API usage remains authoritative.
Source: https://cdn.deepseek.com/api-docs/deepseek_v4_tokenizer.zip
"""
import argparse
import hashlib
import json
from pathlib import Path

TARGETS = (0, 2000, 4000, 8000, 16000)
SOURCE = "https://cdn.deepseek.com/api-docs/deepseek_v4_tokenizer.zip"


def synthetic_history(full_lines, tail_terms):
    history = [
        f"git log --oneline --max-count=8 -- synthetic/fixture_{index:04}.txt"
        for index in range(full_lines)
    ]
    if tail_terms:
        history.append("echo" + " synthetic" * tail_terms)
    return history


def encoded_history(history):
    return json.dumps(history, ensure_ascii=False, separators=(",", ":"))


def generate(tokenizer):
    targets = []
    for target in TARGETS:
        if target == 0:
            targets.append(dict(nominal_history_tokens=0, full_lines=0, tail_terms=0,
                                local_history_json_tokens=0))
            continue
        def count(lines, tail):
            return len(tokenizer.encode(encoded_history(synthetic_history(lines, tail)),
                                        add_special_tokens=False).ids)
        lines = 0
        while count(lines + 1, 0) < target - 8:
            lines += 1
            if lines > 1000:
                raise ValueError("tokenizer does not support bounded synthetic history")
        tail = 1
        while count(lines, tail) < target:
            tail += 1
            if tail > 128:
                raise ValueError("tokenizer does not support the bounded fixture construction")
        actual = count(lines, tail)
        if actual != target:
            raise ValueError("cannot construct exact local history-token target")
        targets.append(dict(nominal_history_tokens=target, full_lines=lines,
                            tail_terms=tail, local_history_json_tokens=actual))
    return targets


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tokenizer", type=Path)
    parser.add_argument("--output", type=Path, default=Path("examples/latency_fixture_manifest.json"))
    args = parser.parse_args()
    from tokenizers import Tokenizer
    manifest = dict(schema=1, tokenizer_source=SOURCE,
                    tokenizer_sha256=hashlib.sha256(args.tokenizer.read_bytes()).hexdigest(),
                    token_count_note="Exact local tokenizer counts of compact synthetic history JSON only; API prompt_tokens is the authoritative full-request count.",
                    targets=generate(Tokenizer.from_file(str(args.tokenizer))))
    args.output.write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps({"manifest": str(args.output), "targets": manifest["targets"]}))


if __name__ == "__main__":
    main()
