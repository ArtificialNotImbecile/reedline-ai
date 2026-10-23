#!/usr/bin/env python3
"""Bounded synthetic streaming diagnostic. Dry run by default; never prints model text."""
import argparse
import json
import os
from pathlib import Path
import re
import statistics
import sys
import time
from decimal import Decimal, InvalidOperation

ENDPOINT = "https://api.deepseek.com/chat/completions"
MODEL = "deepseek-flash"
KEY_ENV = "REEDLINE_AI_API_KEY"
MODEL_WINDOW = 1_048_576
OUTPUT_LIMIT = 128
ATTEMPT_RESERVE = Decimal("2.098176")
MAX_BYTES = 65536
ROOT = Path(__file__).resolve().parents[1]


def emit(value):
    print(json.dumps(value, ensure_ascii=True, allow_nan=False), flush=True)


def product_body():
    # Load only the public, fixed application prompt, never history or user files.
    source = (ROOT / "src/backend.rs").read_text()
    match = re.search(r'pub const SYSTEM_PROMPT: &str = ("(?:[^"\\]|\\.)*");', source)
    if not match:
        raise ValueError("product_prompt_not_found")
    return {"model": MODEL, "messages": [
        {"role": "system", "content": json.loads(match.group(1))},
        {"role": "user", "content": json.dumps({"prefix": "git ", "suffix": " --short"}, separators=(",", ":"))}],
        "max_tokens": OUTPUT_LIMIT, "temperature": 0, "stream": False}


def request_body(group):
    if group == "product":
        body = product_body()
    elif group == "minimal":
        body = {"model": MODEL, "messages": [{"role": "user", "content":
            'Return only the missing text between prefix "git " and suffix " --short".'}],
            "max_tokens": OUTPUT_LIMIT, "temperature": 0}
    else:
        raise ValueError("invalid_group")
    body.update(stream=True, stream_options={"include_usage": True}, thinking={"type": "disabled"})
    return json.dumps(body, separators=(",", ":"), ensure_ascii=False).encode()


def validate_usage(value):
    if not isinstance(value, dict):
        raise ValueError("missing_usage")
    result = {}
    for name, limit in (("prompt_tokens", MODEL_WINDOW), ("completion_tokens", OUTPUT_LIMIT),
                        ("total_tokens", MODEL_WINDOW + OUTPUT_LIMIT)):
        count = value.get(name)
        if type(count) is not int or not 0 <= count <= limit:
            raise ValueError("invalid_usage")
        result[name] = count
    if result["total_tokens"] != result["prompt_tokens"] + result["completion_tokens"]:
        raise ValueError("inconsistent_usage")
    for name in ("prompt_cache_hit_tokens", "prompt_cache_miss_tokens"):
        if name in value:
            count = value[name]
            if type(count) is not int or not 0 <= count <= result["prompt_tokens"]:
                raise ValueError("invalid_cache_usage")
            result[name] = count
    if all(k in result for k in ("prompt_cache_hit_tokens", "prompt_cache_miss_tokens")):
        if result["prompt_cache_hit_tokens"] + result["prompt_cache_miss_tokens"] != result["prompt_tokens"]:
            raise ValueError("inconsistent_cache_usage")
    cost = Decimal(result["prompt_tokens"] * 2 + result["completion_tokens"] * 8) / 1_000_000
    return result, cost


class SSEParser:
    """Incremental byte framing, including split UTF-8, CRLF and multiple data lines."""
    def __init__(self, started, clock=time.perf_counter):
        self.started, self.clock = started, clock
        self.buffer = b""
        self.lines = []
        self.received = 0
        self.first_content_ms = None
        self.done_ms = None
        self.usage = None
        self.content_bytes = 0
        self.content_chunks = 0
        self.error = None

    def feed(self, data):
        self.received += len(data)
        if self.received > MAX_BYTES:
            raise ValueError("response_size_limit")
        self.buffer += data
        while b"\n" in self.buffer:
            line, self.buffer = self.buffer.split(b"\n", 1)
            self.line(line.rstrip(b"\r"))

    def line(self, line):
        if not line:
            if self.lines:
                payload = b"\n".join(self.lines)
                self.lines.clear()
                self.event(payload)
        elif line.startswith(b"data:"):
            self.lines.append(line[5:].removeprefix(b" "))

    def event(self, payload):
        if self.done_ms is not None:
            raise ValueError("event_after_done")
        elapsed = (self.clock() - self.started) * 1000
        if payload.strip() == b"[DONE]":
            self.done_ms = elapsed
            return
        value = json.loads(payload)
        if not isinstance(value, dict) or "error" in value:
            raise ValueError("invalid_stream_event")
        if value.get("usage") is not None:
            self.usage = value["usage"]
        choices = value.get("choices", [])
        if not isinstance(choices, list):
            raise ValueError("invalid_choices")
        for choice in choices:
            if not isinstance(choice, dict):
                raise ValueError("invalid_choice")
            delta = choice.get("delta", {})
            if not isinstance(delta, dict):
                raise ValueError("invalid_delta")
            content = delta.get("content")
            if isinstance(content, str) and content:
                if self.first_content_ms is None:
                    self.first_content_ms = elapsed
                self.content_chunks += 1
                self.content_bytes += len(content.encode())
            elif content is not None and not isinstance(content, str):
                raise ValueError("invalid_content")

    def finish(self):
        if self.buffer:
            self.line(self.buffer.rstrip(b"\r"))
            self.buffer = b""
        self.line(b"")
        if self.done_ms is None or self.first_content_ms is None:
            raise ValueError("incomplete_or_empty_stream")
        return validate_usage(self.usage)


def local_preparation():
    records = []
    for group in ("minimal", "product"):
        samples = []
        for _ in range(100):
            started = time.perf_counter()
            body = request_body(group)
            samples.append((time.perf_counter() - started) * 1000)
        records.append({"body_group": group, "samples": len(samples), "body_bytes": len(body),
                        "prepare_p50_ms": round(statistics.median(samples), 6),
                        "prepare_max_ms": round(max(samples), 6)})
    return records


def make_client(pycurl, args):
    curl = pycurl.Curl()
    curl.setopt(pycurl.URL, ENDPOINT)
    curl.setopt(pycurl.FOLLOWLOCATION, False)
    curl.setopt(pycurl.MAXREDIRS, 0)
    curl.setopt(pycurl.TIMEOUT_MS, args.timeout_seconds * 1000)
    curl.setopt(pycurl.CONNECTTIMEOUT_MS, min(args.timeout_seconds, 15) * 1000)
    curl.setopt(pycurl.NOSIGNAL, 1)
    curl.setopt(pycurl.SSL_VERIFYPEER, 1)
    curl.setopt(pycurl.SSL_VERIFYHOST, 2)
    curl.setopt(pycurl.PROXY_SSL_VERIFYPEER, 1)
    curl.setopt(pycurl.PROXY_SSL_VERIFYHOST, 2)
    if not args.allow_env_proxy:
        curl.setopt(pycurl.PROXY, "")
    # libcurl always verifies the TLS chain. Explicit native-trust opt-in can
    # select the same platform CA bundle used by the Rust application.
    if args.trust_native_certs:
        cafile = os.environ.get("SSL_CERT_FILE") or os.environ.get("REQUESTS_CA_BUNDLE")
        if not cafile:
            import ssl
            cafile = ssl.get_default_verify_paths().cafile
        if cafile:
            curl.setopt(pycurl.CAINFO, cafile)
            curl.setopt(pycurl.PROXY_CAINFO, cafile)
    return curl


def perform(curl, pycurl, body, key):
    parser = SSEParser(0)
    def write(data):
        try:
            parser.feed(data)
            return len(data)
        except (ValueError, UnicodeError, TypeError):
            parser.error = "invalid_stream_or_size_limit"
            return 0
    curl.setopt(pycurl.POSTFIELDS, body)
    curl.setopt(pycurl.HTTPHEADER, ["Content-Type: application/json", "Accept: text/event-stream",
                                  "Authorization: Bearer " + key])
    curl.setopt(pycurl.WRITEFUNCTION, write)
    parser.started = time.perf_counter()
    error_code = None
    try:
        curl.perform()
    except pycurl.error as error:
        error_code = error.args[0] if type(error.args[0]) is int else -1
    ended = time.perf_counter()
    record = {"http_status": curl.getinfo(pycurl.RESPONSE_CODE),
              "request_to_first_content_ms": parser.first_content_ms,
              "request_to_done_ms": parser.done_ms,
              "request_to_end_ms": (ended - parser.started) * 1000,
              "new_connections": curl.getinfo(pycurl.NUM_CONNECTS),
              "proxy_connect_status": curl.getinfo(pycurl.HTTP_CONNECTCODE),
              "http_version_code": curl.getinfo(pycurl.INFO_HTTP_VERSION),
              "response_bytes": parser.received, "content_bytes": parser.content_bytes,
              "content_chunks": parser.content_chunks}
    for name in ("NAMELOOKUP_TIME", "CONNECT_TIME", "APPCONNECT_TIME", "PRETRANSFER_TIME", "STARTTRANSFER_TIME", "TOTAL_TIME"):
        record["curl_" + name.lower() + "_ms"] = curl.getinfo(getattr(pycurl, name)) * 1000
    if hasattr(pycurl, "CONN_ID"):
        record["connection_id"] = curl.getinfo(pycurl.CONN_ID)
    record["observed_connection_reuse"] = record["new_connections"] == 0 and record["http_status"] == 200
    for key_name, value in list(record.items()):
        if isinstance(value, float):
            record[key_name] = round(value, 3)
    if error_code is not None:
        record.update(error="transport_or_stream_error", curl_error_code=error_code)
        return record, None
    if record["http_status"] != 200:
        record["error"] = "provider_http_status"
        return record, None
    try:
        usage, cost = parser.finish()
    except (ValueError, UnicodeError, TypeError):
        record["error"] = "invalid_stream_or_usage"
        return record, None
    record.update(usage=usage, estimated_upper_cny=str(cost))
    return record, cost


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--allow-billable-api", action="store_true")
    ap.add_argument("--max-cost-cny", default="20")
    ap.add_argument("--prior-accounted-cny", default="0")
    ap.add_argument("--allow-env-proxy", action="store_true")
    ap.add_argument("--trust-native-certs", action="store_true")
    ap.add_argument("--timeout-seconds", type=int, choices=range(1, 61), default=30)
    args = ap.parse_args(argv)
    try:
        cap, accounted = Decimal(args.max_cost_cny), Decimal(args.prior_accounted_cny)
        if not all(n.is_finite() for n in (cap, accounted)) or not 0 <= accounted <= cap <= 20:
            raise ValueError()
    except (InvalidOperation, ValueError):
        emit({"error": "invalid_budget"})
        return 1
    emit({"schema": 1, "dry_run": not args.allow_billable_api, "max_requests": 6,
          "attempt_reserve_cny": str(ATTEMPT_RESERVE), "prior_accounted_cny": str(accounted),
          "synthetic_only": True, "application_default_debounce_ms_config_only": 180,
          "application_poll_interval_ms_config_only": 33, "local_python_preparation": local_preparation()})
    if not args.allow_billable_api:
        return 0
    try:
        import pycurl
    except ImportError:
        emit({"error": "pycurl_not_installed", "requests_started": 0})
        return 1
    key = os.environ.get(KEY_ENV)
    if not key or any(ord(c) < 33 or ord(c) > 126 for c in key):
        emit({"error": "missing_or_invalid_key", "requests_started": 0})
        return 1
    if accounted + ATTEMPT_RESERVE * 6 > cap:
        emit({"error": "six_attempt_reserve_exceeds_cap", "requests_started": 0})
        return 1
    attempts = 0
    for group in ("minimal", "product"):
        started = time.perf_counter()
        try:
            curl = make_client(pycurl, args)
        except Exception:
            emit({"error": "client_initialization_failed", "requests_started": attempts})
            return 1
        client_prepare_ms = (time.perf_counter() - started) * 1000
        try:
            for sample in range(1, 4):
                start = time.perf_counter()
                body = request_body(group)
                prepare_ms = (time.perf_counter() - start) * 1000
                accounted += ATTEMPT_RESERVE
                attempts += 1
                emit({"attempt_reserved": attempts, "accounted_cny": str(accounted), "body_group": group, "sample": sample})
                try:
                    record, cost = perform(curl, pycurl, body, key)
                except Exception:
                    record, cost = {"error": "diagnostic_failure_redacted"}, None
                if cost is not None:
                    accounted += cost - ATTEMPT_RESERVE
                record.update(body_group=group, sample=sample, client_sequence="fresh" if sample == 1 else "same_handle",
                              prepare_ms=round(prepare_ms, 6), client_prepare_ms=round(client_prepare_ms, 6) if sample == 1 else 0,
                              request_body_bytes=len(body), accounted_cny=str(accounted), attempts_started=attempts)
                emit(record)
                if cost is None:
                    return 1
        finally:
            curl.close()
    emit({"requests_completed": attempts, "accounted_cny": str(accounted), "stopped_on_failure": False})
    return 0


if __name__ == "__main__":
    sys.exit(main())
