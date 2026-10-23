import json
import unittest
from decimal import Decimal
from unittest.mock import patch
import api_latency_segments as bench


def event(value):
    return b"data: " + json.dumps(value, ensure_ascii=False).encode() + b"\r\n\r\n"


class SegmentedTests(unittest.TestCase):
    def test_fragmented_utf8_role_reasoning_and_usage(self):
        parser = bench.SSEParser(0, clock=lambda: 1)
        data = (b": heartbeat\n\n" + event({"choices": [{"delta": {"role": "assistant", "content": ""}}]})
                + event({"choices": [{"delta": {"reasoning_content": "not answer"}}]})
                + event({"choices": [{"delta": {"content": "世"}}]})
                + event({"choices": [], "usage": {"prompt_tokens": 10, "completion_tokens": 1, "total_tokens": 11}})
                + b"data: [DONE]\r\n\r\n")
        for byte in data:
            parser.feed(bytes([byte]))
        usage, cost = parser.finish()
        self.assertEqual(parser.first_content_ms, 1000)
        self.assertEqual(parser.content_chunks, 1)
        self.assertEqual(parser.content_bytes, 3)
        self.assertEqual(usage["total_tokens"], 11)
        self.assertEqual(cost, Decimal("0.000028"))

    def test_role_and_reasoning_do_not_set_ttft(self):
        p = bench.SSEParser(0, clock=lambda: 1)
        p.feed(event({"choices": [{"delta": {"role": "assistant", "reasoning_content": "abc", "content": ""}}]}))
        self.assertIsNone(p.first_content_ms)

    def test_multiline_sse_and_eof(self):
        p = bench.SSEParser(0, clock=lambda: 1)
        p.feed(b'data: {"choices":\ndata: [{"delta":{"content":"x"}}]}\n\n')
        p.feed(event({"usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}}))
        p.feed(b'data: [DONE]')
        p.finish()
        self.assertEqual(p.content_bytes, 1)

    def test_missing_usage_or_done_fails(self):
        p = bench.SSEParser(0, clock=lambda: 1)
        p.feed(event({"choices": [{"delta": {"content": "x"}}]}))
        with self.assertRaises(ValueError):
            p.finish()
        p.feed(b'data: [DONE]\n\n')
        with self.assertRaises(ValueError):
            p.finish()

    def test_invalid_usage(self):
        for value in (None, {}, {"prompt_tokens": True, "completion_tokens": 1, "total_tokens": 2},
                      {"prompt_tokens": 1, "completion_tokens": 129, "total_tokens": 130},
                      {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 3},
                      {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2, "prompt_cache_hit_tokens": 2}):
            with self.assertRaises(ValueError):
                bench.validate_usage(value)

    def test_size_limit(self):
        with self.assertRaises(ValueError):
            bench.SSEParser(0).feed(b'x' * (bench.MAX_BYTES + 1))

    def test_both_requests_fixed_bounded_streaming(self):
        for group in ("minimal", "product"):
            body = json.loads(bench.request_body(group))
            self.assertEqual(body["max_tokens"], 128)
            self.assertEqual(body["thinking"], {"type": "disabled"})
            self.assertTrue(body["stream"])
            self.assertEqual(body["stream_options"], {"include_usage": True})
            self.assertNotIn("history", body["messages"][-1]["content"])
        body = json.loads(bench.request_body("product"))
        context = json.loads(body["messages"][1]["content"])
        self.assertEqual(context, {"prefix": "git ", "suffix": " --short"})

    def test_dry_run_does_not_read_key_or_network(self):
        getenv = bench.os.environ.get
        def guarded_get(name, *args):
            if name == bench.KEY_ENV:
                raise AssertionError('no key read')
            return getenv(name, *args)
        with patch.object(bench.os.environ, 'get', side_effect=guarded_get):
            with patch.object(bench, "emit") as emit:
                self.assertEqual(bench.main([]), 0)
                self.assertTrue(emit.call_args.args[0]["dry_run"])


if __name__ == '__main__':
    unittest.main()
