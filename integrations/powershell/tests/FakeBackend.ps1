# Offline fixture. Nothing read from stdin is evaluated.
param([string]$Mode = 'ok', [string]$CountFile = '', [string]$Literal = '')
[Console]::InputEncoding = [Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$request = [Console]::In.ReadToEnd() | ConvertFrom-Json
if ($CountFile) { [IO.File]::AppendAllText($CountFile, "request`n") }
if ($Mode -eq 'slow') { Start-Sleep -Seconds 30 }
if ($Mode -eq 'failure') { [Console]::Error.WriteLine('PRIVATE STDERR MUST NOT BE DISPLAYED'); exit 7 }
if ($Mode -eq 'invalid') { [Console]::WriteLine('not json'); exit 0 }
if ($Mode -eq 'large') { [Console]::WriteLine(('x' * 70000)); exit 0 }
$utf8 = [Text.UTF8Encoding]::new($false, $true)
$bytes = $utf8.GetBytes($request.buffer)
$prefix = $utf8.GetString($bytes, 0, $request.cursor)
$suffix = $utf8.GetString($bytes, $request.cursor, $bytes.Length - $request.cursor)
$insertion = '世界'
if ($Literal) { $insertion = $Literal }
$reply = @{
    protocol = 1; request_id = $request.request_id; buffer = $request.buffer; cursor = $request.cursor
    insertion = $insertion; result = $prefix + $insertion + $suffix
    result_cursor = $request.cursor + $utf8.GetByteCount($insertion)
    risk_review = @{ warnings = @(); incomplete = $false }; additional_acceptance_required = $false
}
if ($Mode -eq 'risk') { $reply.risk_review.warnings = @('synthetic warning') }
if ($Mode -eq 'incomplete') { $reply.risk_review.incomplete = $true }
if ($Mode -eq 'additional') { $reply.additional_acceptance_required = $true }
if ($Mode -eq 'stale') { $reply.request_id = 'wrong-request' }
[Console]::WriteLine(($reply | ConvertTo-Json -Depth 8 -Compress))
