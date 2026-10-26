# Incremental completion preview

The standalone desktop editor requests OpenAI-compatible **Chat Completions SSE**
(`stream: true`). Base URL, API key and model are still the only normal provider
settings; no new protocol/provider switch is required.

While receiving, an end-of-line hint shows cumulative text followed by
`[receiving; not ready]`. This is **display only**: Ctrl-F/right-arrow/word acceptance
cannot insert partial AI text, and the completion menu stays pending. Mid-line
requests show receiving status, not an inline insertion preview. Only a complete,
validated response becomes an ordinary acceptable suggestion. Existing risk
confirmation applies to the final suggestion; neither preview nor acceptance
executes a command. Local history acceptance remains separate.

Editing, clearing, or superseding the request cancels the client request and removes
its preview. Stale callbacks cannot overwrite the new context or populate its
cache. Client cancellation does not guarantee provider billing/work has stopped.

## Compatibility and limits

- Native shell bridge and direct/headless completion retain their existing
  non-streaming, final-only response. The mobile UI remains final-only (its engine
  can receive SSE internally); it does not render partial insertions.
- SSE requires `Content-Type: text/event-stream`, LF or CRLF framing, exactly one
  completion choice, textual deltas, `finish_reason: "stop"`, and `[DONE]`.
  Role-only chunks, SSE comments/metadata and usage-only chunks are allowed.
- Truncated, malformed, refusal/tool-call, non-stop, unsafe/control-character, and
  oversized responses are rejected; partial text is never cached/accepted.
- Limits: 64 KiB HTTP body, 16 KiB SSE line/event, 4096 data events, and the existing
  1024-byte safe insertion limit. Timeout applies to the whole response.
- A server ignoring `stream: true` and returning ordinary completion JSON is handled
  in that **same response**. A server rejecting streaming returns an error. There
  is no automatic non-streaming retry or duplicate paid request.
- This is bounded Chat Completions streaming, not a new native FIM protocol,
  tool-execution stream, multi-choice merger, or reconnect/resume implementation.

## Local verification (no paid API)

`cargo test --test http_streaming` uses synthetic loopback HTTP responses for
incremental preview, UTF-8/framing boundaries, malformed streams, limits,
cancellation/socket closure and JSON compatibility. `python3 scripts/pty_streaming.py`
uses a synthetic server plus the actual standalone binary to verify visible partial
repaint, blocked early Ctrl-F, and final acceptance without execution.
