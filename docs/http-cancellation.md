# Local HTTP cancellation

The built-in OpenAI-compatible HTTP backend supports cooperative cancellation.
When the standalone editor changes the buffer/cursor, prefers a native result,
clears/submits/cancels, or exits, its old request future is dropped. A replacement
request does not have to wait for the previous HTTP timeout. This covers waiting
for response headers and reading an incomplete response body. Generation and
source-context checks still reject stale completions; cancelled output is never
cached or offered for acceptance.

This is **local transport cancellation**, not a promise that the provider stops
processing, stops billing, or forgets data already transmitted. Cancellation itself does not add automatic native-shell predictors or command
execution. [Standalone streaming preview](streaming.md) is a separate layer and
retains these cancellation boundaries.

Native shell adapters retain their process-based cancellation and existing wire
protocol. The public synchronous `Backend::complete` method remains available.
Third-party/custom backends remain source-compatible through a default
`complete_cancellable` implementation, but must override that method to interrupt
a running operation. Therefore the prompt cancellation/exit guarantee applies
to this project's built-in HTTP backend, not arbitrary blocking extensions.

An operating-system DNS lookup may continue outside the cancelled request.
Shutdown waits at most 100 ms for the HTTP runtime's blocking pool rather than
waiting indefinitely on that OS operation. HTTP timeouts, response/body bounds,
redirect refusal, explicit proxy/certificate options, request privacy limits and
output validation are unchanged.

## Synthetic verification

`cargo test --locked --test http_cancellation --test http_backend` uses only
loopback HTTP servers and synthetic strings, no provider credentials or paid API:

- A server deliberately never sends headers; changing the context closes its
  socket and a second request completes before the old ten-second timeout.
- Clear, blank input, invalid context and native-history preference all cancel
  the previous transport and cannot publish a stale result.
- A server sends headers and a partial body, then stalls; cancellation closes it.
- Dropping the last editor engine cancels the HTTP call and joins promptly.
- A token cancelled before a call causes no connection.

This does not establish Windows native-console, actual-phone, Docker SSH or
provider-side cancellation behavior; those have their own validation records.
