#requires -Version 5.1
# Import alone does not bind keys, launch a process, or modify a profile.
Set-StrictMode -Version Latest
if (-not ('ReedlineAi.PowerShellAdapterV1.Job' -as [type])) {
    Add-Type -Path (Join-Path $PSScriptRoot 'AdapterProcess.cs') -ErrorAction Stop
}
$script:Configuration = $null
$script:Pending = $null
$script:Preview = $null

function Get-ReedlineAiEditorState {
    $line = ''; $cursor = 0; $selectionStart = -1; $selectionLength = 0
    [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line, [ref]$cursor)
    [Microsoft.PowerShell.PSConsoleReadLine]::GetSelectionState([ref]$selectionStart, [ref]$selectionLength)
    $location = Get-Location
    [pscustomobject]@{
        Buffer = $line; Cursor = $cursor; SelectionLength = $selectionLength
        Cwd = $location.ProviderPath; FileSystem = ($location.Provider.Name -eq 'FileSystem')
    }
}

function Write-ReedlineAiStatus([string]$Message) {
    Write-Host "`n[reedline-ai] $Message"
    [Microsoft.PowerShell.PSConsoleReadLine]::InvokePrompt()
}

function Test-ReedlineAiIdentity($State, $Snapshot) {
    return $State.FileSystem -and $State.SelectionLength -le 0 -and
        [string]::Equals($State.Buffer, $Snapshot.Buffer, [StringComparison]::Ordinal) -and
        $State.Cursor -eq $Snapshot.Cursor -and
        [string]::Equals($State.Cwd, $Snapshot.Cwd, [StringComparison]::Ordinal)
}

function ConvertFrom-ReedlineAiReply([string]$Json, $Snapshot) {
    # ConvertFrom-Json -AsHashtable/-Depth are not available in Windows PS 5.1.
    $object = ConvertFrom-Json -InputObject $Json -ErrorAction Stop
    if ($object -isnot [pscustomobject]) { throw 'Invalid reply.' }
    $reply = @{}
    foreach ($property in $object.PSObject.Properties) { $reply[$property.Name] = $property.Value }
    if ($reply.risk_review -is [pscustomobject]) {
        $review = @{}
        foreach ($property in $reply.risk_review.PSObject.Properties) { $review[$property.Name] = $property.Value }
        $reply.risk_review = $review
    }
    if ($reply -isnot [System.Collections.IDictionary]) { throw 'Invalid reply.' }
    foreach ($name in @('protocol', 'request_id', 'buffer', 'cursor', 'insertion', 'result',
                       'result_cursor', 'risk_review', 'additional_acceptance_required')) {
        if (-not $reply.Contains($name)) { throw 'Missing reply field.' }
    }
    foreach ($name in @('protocol', 'cursor', 'result_cursor')) {
        if ($reply[$name] -isnot [long] -and $reply[$name] -isnot [int]) { throw 'Invalid integer.' }
    }
    foreach ($name in @('request_id', 'buffer', 'insertion', 'result')) {
        if ($reply[$name] -isnot [string]) { throw 'Invalid text.' }
    }
    if ($reply.protocol -ne 1 -or
        -not [string]::Equals($reply.request_id, $Snapshot.RequestId, [StringComparison]::Ordinal) -or
        -not [string]::Equals($reply.buffer, $Snapshot.Buffer, [StringComparison]::Ordinal) -or
        $reply.cursor -ne $Snapshot.ByteCursor) { throw 'Stale reply.' }
    if (-not [ReedlineAi.PowerShellAdapterV1.Text]::Valid($reply.insertion, 1024) -or
        $reply.insertion.Contains('```') -or
        -not [ReedlineAi.PowerShellAdapterV1.Text]::Valid($reply.result, 9216)) { throw 'Unsafe reply.' }
    $expected = $Snapshot.Buffer.Insert($Snapshot.Cursor, $reply.insertion)
    $expectedCursor = [ReedlineAi.PowerShellAdapterV1.Text]::ByteCursor(
        $expected, $Snapshot.Cursor + $reply.insertion.Length)
    if (-not [string]::Equals($reply.result, $expected, [StringComparison]::Ordinal) -or
        $reply.result_cursor -ne $expectedCursor) { throw 'Non-insertion reply.' }
    if ($reply.additional_acceptance_required -isnot [bool] -or
        $reply.risk_review -isnot [System.Collections.IDictionary] -or
        -not $reply.risk_review.Contains('incomplete') -or
        -not $reply.risk_review.Contains('warnings') -or
        $reply.risk_review.incomplete -isnot [bool] -or
        $reply.risk_review.warnings -isnot [array]) { throw 'Missing risk review.' }
    foreach ($warning in $reply.risk_review.warnings) {
        if ($warning -isnot [string]) { throw 'Invalid risk review.' }
    }
    # This first adapter BLOCKS risky or incompletely reviewed candidates. A
    # backend --risk-policy=off flag cannot bypass the native adapter gate.
    if ($reply.additional_acceptance_required -or $reply.risk_review.incomplete -or
        $reply.risk_review.warnings.Count -gt 0) { throw 'Candidate requires additional review and is blocked.' }
    return $reply
}

function Get-ReedlineAiHistoryCandidate($State) {
    if (-not $script:Configuration.UseHistory) { return $null }
    $prefix = $State.Buffer.Substring(0, $State.Cursor)
    $suffix = $State.Buffer.Substring($State.Cursor)
    if ([Microsoft.PowerShell.PSConsoleReadLine].GetMethods().Name -contains 'GetHistoryItems') {
        $items = [Microsoft.PowerShell.PSConsoleReadLine]::GetHistoryItems()
    } else {
        $items = @(Get-History -Count 200)
    }
    for ($index = $items.Count - 1; $index -ge [Math]::Max(0, $items.Count - 200); $index--) {
        $line = $items[$index].CommandLine
        if ($line -and -not [char]::IsWhiteSpace($line[0]) -and $line -cne $State.Buffer -and
            $line.StartsWith($prefix, [StringComparison]::Ordinal) -and
            $line.EndsWith($suffix, [StringComparison]::Ordinal) -and
            $line.Length -gt $State.Buffer.Length -and
            [ReedlineAi.PowerShellAdapterV1.Text]::Valid($line, 8192)) { return $line }
    }
    return $null
}

function Invoke-ReedlineAiRequest {
    if ($null -eq $script:Configuration) { return }
    try {
        $state = Get-ReedlineAiEditorState
        if ([string]::IsNullOrWhiteSpace($state.Buffer)) {
            $script:Preview = $null
            if ($null -ne $script:Pending) { $script:Pending.Cancelled = $true; $script:Pending.Job.Cancel() }
            return # A blank buffer never starts a process or a model request.
        }
        if ($null -ne $script:Preview) {
            Accept-ReedlineAiSuggestion
            return
        }
        if ($null -ne $script:Pending) {
            if ($script:Pending.Cancelled -or
                [ReedlineAi.PowerShellAdapterV1.Text]::NowMilliseconds -gt $script:Pending.ExpiresAt -or
                -not (Test-ReedlineAiIdentity $state $script:Pending)) {
                $script:Pending.Cancelled = $true
                $script:Pending.Job.Cancel()
                if ($script:Pending.Job.Completion.IsCompleted) { $script:Pending = $null }
                $script:Preview = $null
                Write-ReedlineAiStatus 'Input changed; discarded request. Press the request key again for the new input.'
                return
            }
            if (-not $script:Pending.Job.Completion.IsCompleted) {
                Write-ReedlineAiStatus 'Still pending. Keep editing; press the request key again to check.'
                return
            }
            $pending = $script:Pending
            $script:Pending = $null
            $response = $pending.Job.Completion.GetAwaiter().GetResult()
            if ($response.Status -ne 'ok') {
                $script:Preview = $null
                Write-ReedlineAiStatus 'Request failed, timed out, or was cancelled. Input is unchanged.'
                return
            }
            $reply = ConvertFrom-ReedlineAiReply $response.Output $pending
            if ([string]::IsNullOrEmpty($reply.insertion)) {
                Write-ReedlineAiStatus 'No suggestion. Input is unchanged.'
                return
            }
            $script:Preview = [pscustomobject]@{
                Snapshot = $pending; Insertion = $reply.insertion; Result = $reply.result
                ExpiresAt = [ReedlineAi.PowerShellAdapterV1.Text]::NowMilliseconds + 15000
            }
            Write-ReedlineAiStatus ("Preview: " + $reply.result + "`nInsert: " + $script:Configuration.RequestChord + " (or " + $script:Configuration.AcceptChord + ")")
            return
        }
        $script:Preview = $null
        if (-not $state.FileSystem -or $state.SelectionLength -gt 0 -or
            -not [ReedlineAi.PowerShellAdapterV1.Text]::Valid($state.Buffer, 8192)) {
            Write-ReedlineAiStatus 'Only single-line, unselected input in a filesystem directory is supported.'
            return
        }
        $byteCursor = [ReedlineAi.PowerShellAdapterV1.Text]::ByteCursor($state.Buffer, $state.Cursor)
        $requestId = [guid]::NewGuid().ToString('N')
        $request = @{
            protocol = 1; request_id = $requestId
            shell = @{ kind = 'powershell'; version = $PSVersionTable.PSVersion.ToString() }
            cwd = $state.Cwd; buffer = $state.Buffer; cursor = $byteCursor
        }
        $candidate = Get-ReedlineAiHistoryCandidate $state
        if ($candidate) { $request.history_candidate = $candidate }
        $request = $request | ConvertTo-Json -Depth 4 -Compress
        $job = [ReedlineAi.PowerShellAdapterV1.Job]::Start(
            $script:Configuration.Binary, $script:Configuration.Arguments,
            $state.Cwd, $request, $script:Configuration.TimeoutMilliseconds)
        $script:Pending = [pscustomobject]@{
            Buffer = $state.Buffer; Cursor = $state.Cursor; ByteCursor = $byteCursor
            Cwd = $state.Cwd; RequestId = $requestId; Job = $job; Cancelled = $false
            ExpiresAt = [ReedlineAi.PowerShellAdapterV1.Text]::NowMilliseconds + $script:Configuration.TimeoutMilliseconds + 15000
        }
        Write-ReedlineAiStatus 'Requested. Keep editing; press the request key again to preview when ready.'
    } catch {
        $script:Preview = $null
        Write-ReedlineAiStatus 'Unavailable, invalid, or review-required suggestion. Nothing was inserted.'
    }
}

function Add-ReedlineAiText([string]$Text) {
    [Microsoft.PowerShell.PSConsoleReadLine]::Insert($Text)
}

function Accept-ReedlineAiSuggestion {
    if ($null -eq $script:Preview) { return }
    $preview = $script:Preview
    $script:Preview = $null # Consume once, including failed/stale acceptance.
    try {
        $state = Get-ReedlineAiEditorState
        if ([ReedlineAi.PowerShellAdapterV1.Text]::NowMilliseconds -gt $preview.ExpiresAt -or
            -not (Test-ReedlineAiIdentity $state $preview.Snapshot)) {
            Write-ReedlineAiStatus 'Input or cursor changed; preview discarded. Request a new suggestion.'
            return
        }
        # Insert supports PSReadLine undo/redo and preserves the suffix. There is
        # deliberately no AcceptLine, evaluation, prompt submission, or history API.
        Add-ReedlineAiText $preview.Insertion
    } catch {
        Write-ReedlineAiStatus 'Suggestion could not be inserted.'
    }
}

function Cancel-ReedlineAiRequest {
    $script:Preview = $null
    if ($null -ne $script:Pending) {
        $script:Pending.Cancelled = $true
        $script:Pending.Job.Cancel()
    }
    Write-ReedlineAiStatus 'Request and preview discarded.'
}

function Get-ReedlineAiKeyHandler([string]$Chord) {
    $command = Get-Command Get-PSReadLineKeyHandler -ErrorAction Stop
    if ($command.Parameters.ContainsKey('Chord')) {
        Get-PSReadLineKeyHandler -Chord $Chord -ErrorAction Stop
    } else {
        # PSReadLine 2.0/2.1 have the same editing APIs but no targeted lookup.
        Get-PSReadLineKeyHandler -Bound | Where-Object { $_.Key -eq $Chord }
    }
}

function Enable-ReedlineAI {
    [CmdletBinding()]
    param(
        [string]$Binary = 'reedline-ai',
        [switch]$NoHistory,
        [string[]]$BackendArguments = @('--backend', 'mock'),
        [ValidateRange(100, 60000)][int]$TimeoutMilliseconds = 10000,
        [ValidatePattern('^(Ctrl\+Alt\+[a-z]|F([5-9]|1[0-2]))$')][string]$RequestChord = 'Ctrl+Alt+a',
        [ValidatePattern('^(Ctrl\+Alt\+[a-z]|F([5-9]|1[0-2]))$')][string]$AcceptChord = 'Ctrl+Alt+g',
        [ValidatePattern('^(Ctrl\+Alt\+[a-z]|F([5-9]|1[0-2]))$')][string]$CancelChord = 'Ctrl+Alt+x'
    )
    if ($null -ne $script:Configuration) { throw 'Already enabled. Disable-ReedlineAI first.' }
    if ($null -ne $script:Pending -and -not $script:Pending.Job.Completion.IsCompleted) {
        throw 'Previous worker is stopping. Try again after the configured timeout.'
    }
    if (@(@($RequestChord, $AcceptChord, $CancelChord) | Select-Object -Unique).Count -ne 3) {
        throw 'Request, acceptance, and cancel must use different keys.'
    }
    Import-Module PSReadLine -MinimumVersion 2.0 -ErrorAction Stop
    if ((Get-PSReadLineOption).EditMode -eq 'Vi') {
        throw 'This initial adapter supports Emacs/Windows edit modes, not Vi mode.'
    }
    foreach ($method in @('GetBufferState', 'GetSelectionState', 'Insert', 'InvokePrompt')) {
        if (-not ([Microsoft.PowerShell.PSConsoleReadLine].GetMethods().Name -contains $method)) {
            throw "PSReadLine is missing $method. Update PSReadLine before enabling the adapter."
        }
    }
    foreach ($chord in @($RequestChord, $AcceptChord, $CancelChord)) {
        if (@(Get-ReedlineAiKeyHandler $chord).Count -ne 0) {
            throw "Key $chord is already bound. Choose an unused Ctrl+Alt+letter or F5-F12 chord."
        }
    }
    $executable = Get-Command -Name $Binary -CommandType Application -ErrorAction Stop | Select-Object -First 1
    $script:Configuration = [pscustomobject]@{
        UseHistory = -not $NoHistory
        Binary = $executable.Source; Arguments = [string[]](@('--adapter-request') + $BackendArguments)
        TimeoutMilliseconds = $TimeoutMilliseconds; RequestChord = $RequestChord; AcceptChord = $AcceptChord
        CancelChord = $CancelChord; Owner = 'reedline-ai binding ' + [guid]::NewGuid().ToString('N')
    }
    $script:Pending = $null; $script:Preview = $null
    try {
        Set-PSReadLineKeyHandler -Chord $RequestChord -BriefDescription ReedlineAiRequest -Description $script:Configuration.Owner -ScriptBlock {
            param($key, $arg)
            Invoke-ReedlineAiRequest
        } -ErrorAction Stop
        Set-PSReadLineKeyHandler -Chord $AcceptChord -BriefDescription ReedlineAiAccept -Description $script:Configuration.Owner -ScriptBlock {
            param($key, $arg)
            Accept-ReedlineAiSuggestion
        } -ErrorAction Stop
        Set-PSReadLineKeyHandler -Chord $CancelChord -BriefDescription ReedlineAiCancel -Description $script:Configuration.Owner -ScriptBlock {
            param($key, $arg)
            Cancel-ReedlineAiRequest
        } -ErrorAction Stop
    } catch {
        Disable-ReedlineAI
        throw
    }
}

function Disable-ReedlineAI {
    [CmdletBinding()]
    param()
    $script:Preview = $null
    if ($null -ne $script:Pending) { $script:Pending.Cancelled = $true; $script:Pending.Job.Cancel() }
    if ($null -ne $script:Configuration) {
        foreach ($binding in @(
            @($script:Configuration.RequestChord, 'ReedlineAiRequest'),
            @($script:Configuration.AcceptChord, 'ReedlineAiAccept'),
            @($script:Configuration.CancelChord, 'ReedlineAiCancel'))) {
            $current = @(Get-ReedlineAiKeyHandler $binding[0])
            if ($current.Count -eq 1 -and $current[0].Function -eq $binding[1] -and
                $current[0].Description -eq $script:Configuration.Owner) {
                Remove-PSReadLineKeyHandler -Chord $binding[0] -ErrorAction SilentlyContinue
            }
        }
    }
    $script:Configuration = $null
}

$ExecutionContext.SessionState.Module.OnRemove = { Disable-ReedlineAI }
Export-ModuleMember -Function Enable-ReedlineAI, Disable-ReedlineAI
