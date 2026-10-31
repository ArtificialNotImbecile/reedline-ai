#requires -Version 5.1
# No Pester, network, provider keys, profile edits, or actual command execution.
# Uses real ProcessStartInfo transport but a stub editor, not an interactive PTY.
[CmdletBinding()]
param([string]$Binary = '', [string]$ModulePath = '')
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $ModulePath) { $ModulePath = Join-Path $PSScriptRoot '../ReedlineAI.psm1' }
$module = Import-Module $modulePath -PassThru -Force
$pwsh = (Get-Process -Id $PID).Path
$fixture = Join-Path $PSScriptRoot 'FakeBackend.ps1'
$temporary = Join-Path ([IO.Path]::GetTempPath()) ('reedline-ai-test-' + [guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $temporary
$countFile = Join-Path $temporary 'calls.txt'
$script:Assertions = 0
function Assert([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw "FAIL: $Message" }
    $script:Assertions++
}
function Assert-Throws([scriptblock]$Action, [string]$Message) {
    $threw = $false
    try { & $Action | Out-Null } catch { $threw = $true }
    Assert $threw $Message
}
function Get-TestKeyHandler([string[]]$Chord) {
    foreach ($key in $Chord) { & $module { param($key) Get-ReedlineAiKeyHandler $key } $key }
}
function Wait-JobReply($Job) {
    Assert ($Job.Completion.Wait(6000)) 'worker completes within bound'
    return $Job.Completion.GetAwaiter().GetResult()
}
function New-FixtureJob([string]$Mode = 'ok', [int]$Timeout = 4000, [string]$Literal = '') {
    $arguments = [string[]]@('-NoLogo', '-NoProfile', '-NonInteractive', '-File', $fixture,
        '-Mode', $Mode, '-CountFile', $countFile, '-Literal', $Literal)
    return [ReedlineAi.PowerShellAdapterV1.Job]::Start($pwsh, $arguments, $temporary, $requestJson, $Timeout)
}
try {
    Assert ([ReedlineAi.PowerShellAdapterV1.Job]::QuoteArgument('') -ceq '""') 'empty Windows argument'
    Assert ([ReedlineAi.PowerShellAdapterV1.Job]::QuoteArgument('a b\') -ceq '"a b\\"') 'trailing Windows backslash'
    Assert ([ReedlineAi.PowerShellAdapterV1.Job]::QuoteArgument('a"b') -ceq '"a\"b"') 'embedded Windows quote'
    Assert (-not (Test-Path $countFile)) 'import does not start a request'
    Assert ([ReedlineAi.PowerShellAdapterV1.Text]::ByteCursor('echo 世🦀後', 8) -eq 12) 'UTF-16 to UTF-8 cursor'
    Assert-Throws { [ReedlineAi.PowerShellAdapterV1.Text]::ByteCursor('echo 世🦀後', 7) } 'split surrogate rejected'
    Assert-Throws { [ReedlineAi.PowerShellAdapterV1.Text]::ByteCursor('abc', 4) } 'out-of-range cursor rejected'
    Assert (-not [ReedlineAi.PowerShellAdapterV1.Text]::Valid("echo`nhi", 8192)) 'multiline rejected'
    Assert (-not [ReedlineAi.PowerShellAdapterV1.Text]::Valid(([string][char]0x202E), 8192)) 'bidi rejected'
    Assert (-not [ReedlineAi.PowerShellAdapterV1.Text]::Valid(('x' * 8193), 8192)) 'oversized input rejected'
    $snapshot = [pscustomobject]@{
        Buffer = 'echo 🦀後'; Cursor = 7; ByteCursor = 9; RequestId = 'test-1'; Cwd = $temporary
    }
    $requestJson = @{
        protocol = 1; request_id = $snapshot.RequestId; shell = @{ kind = 'powershell'; version = '7.2' }
        cwd = $temporary; buffer = $snapshot.Buffer; cursor = $snapshot.ByteCursor
    } | ConvertTo-Json -Depth 4 -Compress
    $reply = Wait-JobReply (New-FixtureJob)
    Assert ($reply.Status -eq 'ok') 'real child process returns JSON'
    $parsed = & $module { param($json, $snapshot) ConvertFrom-ReedlineAiReply $json $snapshot } $reply.Output $snapshot
    Assert ($parsed.result -ceq 'echo 🦀世界後') 'Unicode FIM preserves suffix'
    Assert ($parsed.result_cursor -eq 15) 'UTF-8 result cursor checked'
    foreach ($mode in @('risk', 'incomplete', 'additional', 'stale', 'invalid')) {
        $bad = Wait-JobReply (New-FixtureJob $mode)
        Assert-Throws { & $module { param($json, $snapshot) ConvertFrom-ReedlineAiReply $json $snapshot } $bad.Output $snapshot } "reject $mode"
    }
    $bad = @{}
    ($reply.Output | ConvertFrom-Json).PSObject.Properties | ForEach-Object { $bad[$_.Name] = $_.Value }
    $bad.result = 'replacement'; $badJson = $bad | ConvertTo-Json -Depth 8 -Compress
    Assert-Throws { & $module { param($json, $snapshot) ConvertFrom-ReedlineAiReply $json $snapshot } $badJson $snapshot } 'replacement result rejected'
    $bad = @{}
    ($reply.Output | ConvertFrom-Json).PSObject.Properties | ForEach-Object { $bad[$_.Name] = $_.Value }
    $bad.Remove('risk_review'); $badJson = $bad | ConvertTo-Json -Depth 8 -Compress
    Assert-Throws { & $module { param($json, $snapshot) ConvertFrom-ReedlineAiReply $json $snapshot } $badJson $snapshot } 'missing risk review rejected'
    $snapshotV2 = [pscustomobject]@{
        Protocol = 2; Buffer = 'run CCTV'; Cursor = 8; ByteCursor = 8; RequestId = 'workflow'; Cwd = $temporary
    }
    $replacement = 'cd "世界🦀"'
    $v2 = @{
        protocol = 2; request_id = 'workflow'; buffer = 'run CCTV'; cursor = 8; insertion = ''
        replacement = $replacement; result = $replacement
        result_cursor = [Text.Encoding]::UTF8.GetByteCount($replacement)
        risk_review = @{ warnings = @(); incomplete = $false }; additional_acceptance_required = $false
    }
    $v2Json = $v2 | ConvertTo-Json -Depth 8 -Compress
    $parsedV2 = & $module { param($json, $snapshot) ConvertFrom-ReedlineAiReply $json $snapshot } $v2Json $snapshotV2
    Assert ($parsedV2.result -ceq $replacement) 'v2 Unicode replacement validates'
    foreach ($field in @('protocol', 'insertion', 'replacement', 'result', 'result_cursor')) {
        $bad = $v2.Clone()
        switch ($field) {
            protocol { $bad.protocol = 1 }
            insertion { $bad.insertion = 'x' }
            replacement { $bad.replacement = $null }
            result { $bad.result = 'other' }
            result_cursor { $bad.result_cursor = 1 }
        }
        $badJson = $bad | ConvertTo-Json -Depth 8 -Compress
        Assert-Throws { & $module { param($json, $snapshot) ConvertFrom-ReedlineAiReply $json $snapshot } $badJson $snapshotV2 } "v2 rejects $field"
    }
    foreach ($text in @(('x' * 8193), 'a```b', "a`nb")) {
        $bad = $v2.Clone(); $bad.replacement = $text; $bad.result = $text
        $bad.result_cursor = [Text.Encoding]::UTF8.GetByteCount($text)
        $badJson = $bad | ConvertTo-Json -Depth 8 -Compress
        Assert-Throws { & $module { param($json, $snapshot) ConvertFrom-ReedlineAiReply $json $snapshot } $badJson $snapshotV2 } 'unsafe replacement rejected'
    }
    $bad = $v2.Clone(); $bad.protocol = 1
    $badJson = $bad | ConvertTo-Json -Depth 8 -Compress
    $snapshotV1 = $snapshotV2.PSObject.Copy(); $snapshotV1.Protocol = 1
    Assert-Throws { & $module { param($json, $snapshot) ConvertFrom-ReedlineAiReply $json $snapshot } $badJson $snapshotV1 } 'v1 replacement rejected'
    $literal = 'quote'' " $() ; & | ` literal'
    $literalReply = Wait-JobReply (New-FixtureJob 'ok' 4000 $literal)
    Assert (($literalReply.Output | ConvertFrom-Json).insertion -ceq $literal) 'argument boundaries, no shell interpolation'
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $slow = New-FixtureJob 'slow' 250
    Assert ($watch.ElapsedMilliseconds -lt 1000) 'request returns without waiting for child'
    $timed = Wait-JobReply $slow
    Assert ($timed.Status -eq 'timeout') 'deadline terminates slow child without another keypress'
    $cancelled = New-FixtureJob 'slow'
    Assert ((Wait-JobReply (New-FixtureJob)).Status -eq 'busy') 'one worker even across callers, no process queue'
    $cancelled.Cancel()
    Assert ((Wait-JobReply $cancelled).Status -eq 'cancelled') 'explicit cancellation completes'
    Assert ((Wait-JobReply (New-FixtureJob 'failure')).Status -eq 'failed') 'nonzero exit fails closed'
    $large = Wait-JobReply (New-FixtureJob 'large')
    Assert ($large.Status -ne 'ok') 'oversized output fails closed'
    Assert ([string]::IsNullOrEmpty($large.Output)) 'failure never exposes child output'

    # Real PSReadLine registration lifecycle, without running the editor loop.
    Import-Module PSReadLine -MinimumVersion 2.0
    $nativeBefore = Get-PSReadLineKeyHandler -Bound | Sort-Object Key | Select-Object Key, Function | ConvertTo-Json -Compress
    Enable-ReedlineAI -Binary $pwsh
    Assert (@(Get-TestKeyHandler -Chord 'Ctrl+Alt+a', 'Ctrl+Alt+g', 'Ctrl+Alt+x').Count -eq 3) 'register three native handlers'
    Disable-ReedlineAI
    $nativeAfter = Get-PSReadLineKeyHandler -Bound | Sort-Object Key | Select-Object Key, Function | ConvertTo-Json -Compress
    Assert ($nativeBefore -ceq $nativeAfter) 'disable restores all original native bindings'
    Set-PSReadLineKeyHandler -Chord 'Ctrl+Alt+a' -Function BeginningOfLine
    Assert-Throws { Enable-ReedlineAI -Binary $pwsh } 'refuse existing binding collision'
    Assert ((Get-TestKeyHandler -Chord 'Ctrl+Alt+a').Function -eq 'BeginningOfLine') 'collision leaves existing binding intact'
    Remove-PSReadLineKeyHandler -Chord 'Ctrl+Alt+a'

    Enable-ReedlineAI -Binary $pwsh
    Set-PSReadLineKeyHandler -Chord 'Ctrl+Alt+a' -BriefDescription ReedlineAiRequest -ScriptBlock { }
    Disable-ReedlineAI
    Assert (@(Get-TestKeyHandler -Chord 'Ctrl+Alt+a').Count -eq 1) 'disable preserves replaced handler even with same display name'
    Assert (@(Get-TestKeyHandler -Chord 'Ctrl+Alt+g', 'Ctrl+Alt+x').Count -eq 0) 'disable removes owned sibling bindings'
    Remove-PSReadLineKeyHandler -Chord 'Ctrl+Alt+a'

    # Replace only the module's editor boundary. No real PSReadLine edit is used.
    & $module {
        param($binary, $fixture, $countFile, $cwd)
        $script:TestState = [pscustomobject]@{
            Buffer = ' '; Cursor = 1; SelectionLength = 0; Cwd = $cwd; FileSystem = $true
        }
        $script:TestInserted = ''; $script:TestStatus = ''
        function script:Get-ReedlineAiEditorState { return $script:TestState }
        function script:Write-ReedlineAiStatus([string]$Message) { $script:TestStatus = $Message }
        function script:Add-ReedlineAiText([string]$Text) { $script:TestInserted += $Text }
        function script:Set-ReedlineAiText([int]$Length, [string]$Text) {
            $script:TestReplaced = $Text; $script:TestReplaceLength = $Length
        }
        $script:Configuration = [pscustomobject]@{
            UseHistory = $false
            Binary = $binary
            Arguments = [string[]]@('-NoLogo', '-NoProfile', '-NonInteractive', '-File', $fixture, '-CountFile', $countFile)
            TestCwd = $cwd; TimeoutMilliseconds = 4000; RequestChord = 'Ctrl+Alt+a'; AcceptChord = 'Ctrl+Alt+g'
        }
    } $pwsh $fixture $countFile $temporary
    $beforeBlank = (Get-Content $countFile).Count
    & $module { Invoke-ReedlineAiRequest }
    Assert ((Get-Content $countFile).Count -eq $beforeBlank) 'blank input starts zero processes'
    Assert (& $module { $null -eq $script:Pending }) 'blank leaves no pending worker'
    $pending = & $module {
        $script:TestState.Buffer = 'echo 🦀後'; $script:TestState.Cursor = 7
        Invoke-ReedlineAiRequest
        Accept-ReedlineAiSuggestion
        return $script:Pending.Job
    }
    Assert (& $module { $script:TestInserted -eq '' }) 'request and premature accept do not insert'
    $null = Wait-JobReply $pending
    & $module { Invoke-ReedlineAiRequest }
    Assert (& $module { $null -ne $script:Preview -and $script:TestInserted -eq '' }) 'second request previews without insert'
    & $module { Invoke-ReedlineAiRequest; Accept-ReedlineAiSuggestion }
    Assert (& $module { $script:TestInserted -ceq '世界' }) 'third request key inserts once only'
    foreach ($change in @('Buffer', 'Cursor', 'Cwd', 'Selection')) {
        $pending = & $module { Invoke-ReedlineAiRequest; return $script:Pending.Job }
        $null = Wait-JobReply $pending
        & $module { Invoke-ReedlineAiRequest }
        & $module {
            param($change)
            switch ($change) {
                Buffer { $script:TestState.Buffer += 'x' }
                Cursor { $script:TestState.Cursor-- }
                Cwd { $script:TestState.Cwd += 'different' }
                Selection { $script:TestState.SelectionLength = 1 }
            }
            Accept-ReedlineAiSuggestion
            $script:TestState.Buffer = 'echo 🦀後'; $script:TestState.Cursor = 7
            $script:TestState.Cwd = $script:Configuration.TestCwd
            $script:TestState.SelectionLength = 0
        } $change
        Assert (& $module { $script:TestInserted -ceq '世界' -and $null -eq $script:Preview }) "reject stale $change"
    }
    $pending = & $module { Invoke-ReedlineAiRequest; return $script:Pending.Job }
    $null = Wait-JobReply $pending
    & $module { Invoke-ReedlineAiRequest; $script:Preview.ExpiresAt = 0; Accept-ReedlineAiSuggestion }
    Assert (& $module { $script:TestInserted -ceq '世界' -and $null -eq $script:Preview }) 'expired preview rejected'
    $pending = & $module { Invoke-ReedlineAiRequest; return $script:Pending.Job }
    $null = Wait-JobReply $pending
    & $module { Invoke-ReedlineAiRequest; Cancel-ReedlineAiRequest; Accept-ReedlineAiSuggestion }
    Assert (& $module { $script:TestInserted -ceq '世界' -and $null -eq $script:Preview }) 'dedicated cancel clears preview'
    $pending = & $module { Invoke-ReedlineAiRequest; return $script:Pending.Job }
    $null = Wait-JobReply $pending
    & $module { Cancel-ReedlineAiRequest; Invoke-ReedlineAiRequest }
    Assert (& $module { $null -eq $script:Preview -and $null -eq $script:Pending }) 'cancelled completed request cannot become preview'
    & $module {
        $script:TestState.Buffer = 'run CCTV'; $script:TestState.Cursor = 8
        $script:TestReplaced = ''
        $script:Preview = [pscustomobject]@{
            Snapshot = $script:TestState.PSObject.Copy(); IsReplacement = $true
            Insertion = ''; Result = 'cd "世界🦀"'; ExpiresAt = [ReedlineAi.PowerShellAdapterV1.Text]::NowMilliseconds + 15000
        }
        Accept-ReedlineAiSuggestion; Accept-ReedlineAiSuggestion
    }
    Assert (& $module { $script:TestReplaced -ceq 'cd "世界🦀"' -and $script:TestReplaceLength -eq 8 -and $null -eq $script:Preview }) 'replacement accept edits whole line once without execution'
    if ($Binary) {
        $resolved = (Get-Command $Binary -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
        $native = [ReedlineAi.PowerShellAdapterV1.Job]::Start($resolved,
            [string[]]@('--adapter-request', '--backend', 'mock'), $temporary, $requestJson, 4000)
        $nativeReply = Wait-JobReply $native
        Assert ($nativeReply.Status -eq 'ok') 'real reedline-ai mock bridge responds'
        $null = & $module { param($json, $snapshot) ConvertFrom-ReedlineAiReply $json $snapshot } $nativeReply.Output $snapshot
        Assert $true 'real bridge response validates'
    }
    Write-Host "PASS: $script:Assertions assertions (transport and stub-editor tests, not terminal coverage)."
} finally {
    # The test has not installed key bindings, so avoid loading PSReadLine at exit.
    & $module { $script:Configuration = $null; if ($null -ne $script:Pending) { $script:Pending.Job.Cancel() } }
    Remove-Module $module -Force
    Remove-Item -LiteralPath $temporary -Recurse -Force
}
