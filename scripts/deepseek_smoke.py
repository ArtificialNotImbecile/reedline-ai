#!/usr/bin/env python3
"""Explicit, billable, three-request evaluation through the Rust HTTP adapter.

Never called by CI. No model output is executed. See README before opting in.
Uses only fixed synthetic buffers, an environment key, and the official endpoint.
"""
import argparse
from decimal import Decimal, InvalidOperation
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

ENDPOINT = "https://api.deepseek.com/chat/completions"
MODEL = "deepseek-flash"
KEY_ENV = "REEDLINE_AI_API_KEY"
MAX_CONTEXT_TOKENS = 1_048_576
MAX_OUTPUT_TOKENS = 128
# Official peak prices checked 2026-10-23; never assume cached/off-peak discounts.
# https://api-docs.deepseek.com/zh-cn/quick_start/pricing/
INPUT_CNY_PER_MILLION = Decimal("2")
OUTPUT_CNY_PER_MILLION = Decimal("8")
PER_ATTEMPT_RESERVE = (
    MAX_CONTEXT_TOKENS * INPUT_CNY_PER_MILLION
    + MAX_OUTPUT_TOKENS * OUTPUT_CNY_PER_MILLION
) / 1_000_000
CASES = (
    ("suffix", "git st", 6, "atus"),
    ("fim", "git  --short", 4, "status"),
    ("unicode", "echo 世", 8, "界"),
)

# Exact known application messages only. Never print arbitrary captured stderr.
SAFE_ERRORS = {
    b"reedline-ai: request failed or timed out (details redacted)": "transport_or_timeout",
    b"reedline-ai: provider returned an invalid chat completion": "invalid_completion_response",
    b"reedline-ai: provider response exceeded the 64 KiB limit": "response_size_limit",
    b"reedline-ai: API key cannot be used as an HTTP header": "invalid_auth_header",
    b"reedline-ai: could not initialize the HTTP client": "http_client_initialization",
}


def emit(value):
    # Escape Unicode/terminal controls; output is JSON, never a command to run.
    print(json.dumps(value, ensure_ascii=True), flush=True)


def validate_result(value, buffer, cursor, key):
    if not isinstance(value, dict):
        raise ValueError("invalid result")
    insertion = value.get("insertion")
    result = value.get("result")
    usage = value.get("usage")
    if not isinstance(insertion, str) or not isinstance(result, str):
        raise ValueError("invalid completion")
    if key in insertion or key in result:
        raise ValueError("credential-like output rejected")
    if len(insertion.encode("utf-8")) > 1024 or any(
        ord(char) < 32 or 127 <= ord(char) <= 159 for char in insertion
    ):
        raise ValueError("unsafe completion")
    raw = buffer.encode("utf-8")
    expected = raw[:cursor].decode() + insertion + raw[cursor:].decode()
    if value.get("cursor") != cursor or result != expected:
        raise ValueError("insertion did not preserve context")
    if not isinstance(usage, dict):
        raise ValueError("missing token usage")
    prompt = usage.get("prompt_tokens")
    completion = usage.get("completion_tokens")
    total = usage.get("total_tokens")
    if not all(type(count) is int for count in (prompt, completion, total)):
        raise ValueError("invalid token usage")
    if not (0 <= prompt <= MAX_CONTEXT_TOKENS and 0 <= completion <= MAX_OUTPUT_TOKENS):
        raise ValueError("token usage exceeded configured bounds")
    if total != prompt + completion:
        raise ValueError("inconsistent token usage")
    cost = (prompt * INPUT_CNY_PER_MILLION + completion * OUTPUT_CNY_PER_MILLION) / 1_000_000
    # Only allowlisted numeric usage fields are reported.
    safe_usage = {"prompt_tokens": prompt, "completion_tokens": completion, "total_tokens": total}
    for field in ("prompt_cache_hit_tokens", "prompt_cache_miss_tokens"):
        count = usage.get(field)
        if type(count) is int and 0 <= count <= prompt:
            safe_usage[field] = count
    return insertion, result, safe_usage, cost


def run_cases(binary, key, budget, runner=subprocess.run, allow_env_proxy=False, trust_native_certs=False):
    reserve = PER_ATTEMPT_RESERVE * len(CASES)
    if not budget.is_finite() or budget < reserve or budget > Decimal("20"):
        emit({"error": "budget must cover the conservative reserve and be at most CNY 20", "reserve_cny": str(reserve)})
        return 1
    emit({"model": MODEL, "requests_max": len(CASES), "output_tokens_max_per_request": MAX_OUTPUT_TOKENS,
          "reserve_upper_cny_at_2026_10_23_prices": str(reserve), "price_assumption": "peak, all prompt tokens uncached"})
    cost_total = Decimal("0")
    for name, buffer, cursor, sample in CASES:
        command = [str(binary), "--backend", "http", "--provider", "deepseek",
                   "--endpoint", ENDPOINT, "--model", MODEL, "--api-key-env", KEY_ENV,
                   "--timeout-seconds", "15", "--report-usage", "--complete", buffer,
                   "--cursor", str(cursor)]
        if allow_env_proxy:
            command.append("--allow-env-proxy")
        if trust_native_certs:
            command.append("--trust-native-certs")
        start = time.monotonic()
        try:
            completed = runner(command, capture_output=True, timeout=20, check=False)
        except (OSError, subprocess.TimeoutExpired):
            emit({"case": name, "error": "process failed or timed out; stopped without retry"})
            return 1
        if completed.returncode != 0:
            # Includes HTTP/auth/payment errors. Never echo captured stderr or bodies.
            error = {"case": name, "error": "completion failed; stopped without retry; details redacted", "exit_code": completed.returncode}
            status = re.fullmatch(rb"reedline-ai: provider returned HTTP ([0-9]{3}) \(body redacted\)\r?\n", completed.stderr)
            if status:
                error["http_status"] = int(status.group(1))
                error["error_kind"] = "provider_http_status"
            else:
                error["error_kind"] = SAFE_ERRORS.get(completed.stderr.strip(),
                    "invalid_cli_arguments" if completed.returncode == 2 else "unclassified_redacted")
            emit(error)
            return 1
        try:
            if len(completed.stdout) > 16_384:
                raise ValueError("oversized CLI result")
            value = json.loads(completed.stdout)
            insertion, result, usage, cost = validate_result(value, buffer, cursor, key)
        except (ValueError, UnicodeError, TypeError):
            emit({"case": name, "error": "invalid or unsafe result/usage; stopped without retry"})
            return 1
        cost_total += cost
        emit({"case": name, "latency_ms": round((time.monotonic() - start) * 1000),
              "insertion": insertion, "result": result, "context_preserved": True,
              "nonempty": bool(insertion), "example_match": insertion == sample,
              "usage": usage, "estimated_upper_cny": str(cost)})
    emit({"requests_completed": len(CASES), "estimated_upper_cny": str(cost_total),
          "note": "Usage-based estimate, not a provider billing guarantee. Example matches are quality observations, not deterministic assertions."})
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--allow-billable-api", action="store_true", help="explicitly permit this one-time remote test")
    parser.add_argument("--max-cost-cny", required=True, help="approved cap, at most 20; dated-price reserve must fit")
    parser.add_argument("--allow-env-proxy", action="store_true", help="opt in to configured environment proxies; never printed")
    parser.add_argument("--trust-native-certs", action="store_true", help="opt in to platform/SSL_CERT_FILE certificate trust; TLS verification stays on")
    default_binary = Path(__file__).resolve().parents[1] / "target" / "release" / ("reedline-ai.exe" if os.name == "nt" else "reedline-ai")
    parser.add_argument("--binary", type=Path, default=default_binary)
    args = parser.parse_args(argv)
    if not args.allow_billable_api:
        parser.error("--allow-billable-api is required; no request made")
    try:
        budget = Decimal(args.max_cost_cny)
    except InvalidOperation:
        parser.error("--max-cost-cny must be a decimal number")
    if not args.binary.is_file():
        parser.error("binary not found; run cargo build --locked --release first")
    key = os.environ.get(KEY_ENV)
    if not key:
        parser.error("REEDLINE_AI_API_KEY must be set in the process environment; never pass the key as an argument")
    return run_cases(args.binary, key, budget, allow_env_proxy=args.allow_env_proxy, trust_native_certs=args.trust_native_certs)


if __name__ == "__main__":
    sys.exit(main())
