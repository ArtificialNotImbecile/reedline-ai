"""Offline tests: never read keys, launch binaries, or contact model APIs."""
import contextlib
from decimal import Decimal
import io
import json
import subprocess
import unittest
from unittest.mock import Mock, patch

import deepseek_smoke as smoke


class SmokeTests(unittest.TestCase):
    def invoke(self, runner, budget=Decimal("20")):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            code = smoke.run_cases("unused-binary", "TEST_ONLY_FAKE_KEY", budget, runner)
        return code, output.getvalue()

    @staticmethod
    def success(command, **kwargs):
        buffer = command[command.index("--complete") + 1]
        cursor = int(command[command.index("--cursor") + 1])
        insertion = next(case[3] for case in smoke.CASES if case[1] == buffer)
        raw = buffer.encode()
        result = raw[:cursor].decode() + insertion + raw[cursor:].decode()
        body = {"cursor": cursor, "insertion": insertion, "result": result,
                "usage": {"prompt_tokens": 100, "completion_tokens": 5, "total_tokens": 105}}
        return subprocess.CompletedProcess(command, 0, json.dumps(body).encode(), b"")

    def test_three_fixed_requests_with_limits_and_usage(self):
        runner = Mock(side_effect=self.success)
        code, output = self.invoke(runner)
        self.assertEqual(code, 0)
        self.assertEqual(runner.call_count, 3)
        self.assertNotIn("TEST_ONLY_FAKE_KEY", output)
        self.assertIn('"estimated_upper_cny": "0.00072"', output)
        for call in runner.call_args_list:
            command = call.args[0]
            self.assertIn(smoke.ENDPOINT, command)
            self.assertIn("--report-usage", command)
            self.assertNotIn("TEST_ONLY_FAKE_KEY", command)
            self.assertEqual(call.kwargs["timeout"], 20)

    def test_failure_stops_without_retry_or_raw_output(self):
        runner = Mock(return_value=subprocess.CompletedProcess([], 1, b"SECRET_BODY", b"SECRET_ERROR"))
        code, output = self.invoke(runner)
        self.assertEqual(code, 1)
        self.assertEqual(runner.call_count, 1)
        self.assertNotIn("SECRET", output)

    def test_budget_rejected_before_any_request(self):
        for budget in (Decimal("0"), Decimal("NaN"), Decimal("21")):
            runner = Mock()
            code, _ = self.invoke(runner, budget)
            self.assertEqual(code, 1)
            runner.assert_not_called()

    def test_only_allowlisted_http_status_is_reported(self):
        runner = Mock(return_value=subprocess.CompletedProcess([], 1, b"", b"reedline-ai: provider returned HTTP 402 (body redacted)\n"))
        code, output = self.invoke(runner)
        self.assertEqual(code, 1)
        self.assertIn('"http_status": 402', output)
        self.assertEqual(runner.call_count, 1)

    def test_known_error_classes_without_raw_error_echo(self):
        for stderr, exit_code, kind in (
            (b"reedline-ai: request failed or timed out (details redacted)\n", 1, "transport_or_timeout"),
            (b"reedline-ai: provider returned an invalid chat completion\n", 1, "invalid_completion_response"),
            (b"PRIVATE_ARGUMENT_OR_KEY", 2, "invalid_cli_arguments"),
            (b"PRIVATE_PROVIDER_OR_KEY", 1, "unclassified_redacted"),
        ):
            runner = Mock(return_value=subprocess.CompletedProcess([], exit_code, b"", stderr))
            code, output = self.invoke(runner)
            self.assertEqual(code, 1)
            self.assertIn(kind, output)
            self.assertNotIn("PRIVATE", output)
            self.assertEqual(runner.call_count, 1)

    def test_credential_or_invalid_usage_is_never_printed(self):
        for insertion, tokens in (("TEST_ONLY_FAKE_KEY", 5), ("atus", 129)):
            body = {"cursor": 6, "insertion": insertion, "result": "git st" + insertion,
                    "usage": {"prompt_tokens": 100, "completion_tokens": tokens, "total_tokens": 100 + tokens}}
            runner = Mock(return_value=subprocess.CompletedProcess([], 0, json.dumps(body).encode(), b""))
            code, output = self.invoke(runner)
            self.assertEqual(code, 1)
            self.assertEqual(runner.call_count, 1)
            self.assertNotIn("TEST_ONLY_FAKE_KEY", output)

    def test_missing_opt_in_does_not_read_key_or_run(self):
        with patch.object(smoke.os.environ, "get", return_value=None) as env_get, contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit):
                smoke.main(["--max-cost-cny", "20"])
            self.assertNotIn(smoke.KEY_ENV, [call.args[0] for call in env_get.call_args_list])

    def test_missing_or_malformed_usage_stops(self):
        for usage in (None, {}, {"prompt_tokens": "SECRET", "completion_tokens": 1, "total_tokens": 1}):
            body = {"cursor": 6, "insertion": "atus", "result": "git status", "usage": usage}
            runner = Mock(return_value=subprocess.CompletedProcess([], 0, json.dumps(body).encode(), b""))
            code, output = self.invoke(runner)
            self.assertEqual(code, 1)
            self.assertEqual(runner.call_count, 1)
            self.assertNotIn("SECRET", output)

    def test_network_options_are_forwarded_only_when_opted_in(self):
        for enabled in (False, True):
            runner = Mock(side_effect=self.success)
            with contextlib.redirect_stdout(io.StringIO()):
                code = smoke.run_cases("unused-binary", "TEST_ONLY_FAKE_KEY", Decimal("20"),
                                       runner, allow_env_proxy=enabled, trust_native_certs=enabled)
            self.assertEqual(code, 0)
            for call in runner.call_args_list:
                self.assertEqual("--allow-env-proxy" in call.args[0], enabled)
                self.assertEqual("--trust-native-certs" in call.args[0], enabled)


if __name__ == "__main__":
    unittest.main()
