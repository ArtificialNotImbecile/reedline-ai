# Opt-in native Nushell adapter. Source this file, then call reedline-ai-enable.
# Requires the commandline and experimental job mailbox APIs (tested on 0.116.1).
# No profile writes, command execution, history reads, or automatic requests.
# Python 3 provides bounded subprocess IO; request/result bytes stay in memory.
const REEDLINE_AI_DEFAULT_TRANSPORT = path self ../shared/bridge.py

def reedline-ai-grapheme-to-byte [text: string, cursor: int] {
    if $cursor < 0 or $cursor > ($text | str length --grapheme-clusters) {
        error make {msg: 'Invalid editor cursor'}
    }
    $text | split chars --grapheme-clusters | take $cursor | str join | str length --utf-8-bytes
}

def reedline-ai-byte-to-grapheme [text: string, cursor: int] {
    mut bytes = 0
    mut position = 0
    for cluster in ($text | split chars --grapheme-clusters) {
        if $bytes == $cursor { return $position }
        $bytes += ($cluster | str length --utf-8-bytes)
        $position += 1
    }
    if $bytes == $cursor { return $position }
    error make {msg: 'Response cursor is not on a grapheme boundary'}
}

def reedline-ai-safe-text [text: string] {
    not ($text =~ '[\x00-\x1f\x7f-\x9f\x{061c}\x{200b}-\x{200f}\x{2028}-\x{202e}\x{2060}-\x{206f}\x{feff}]')
}

# Fail closed even for a malformed or mismatched local backend response.
def reedline-ai-check-response [request: record, response: record] {
    try {
        if $response.protocol != 1 or $response.request_id != $request.request_id { return false }
        if $response.buffer != $request.buffer or $response.cursor != $request.cursor { return false }
        if ($response.insertion | describe) != 'string' or ($response.result | describe) != 'string' { return false }
        if ($response.additional_acceptance_required | describe) != 'bool' { return false }
        if ($response.risk_review.incomplete | describe) != 'bool' { return false }
        if ($response.risk_review.warnings | describe) !~ '^list<string>|^list<any>' { return false }
        if not (reedline-ai-safe-text $response.insertion) or not (reedline-ai-safe-text $response.result) { return false }
        if ($response.insertion | str length --utf-8-bytes) > 1024 or ($response.insertion | str contains '```') { return false }
        if ($response.result | str length --utf-8-bytes) > 9216 { return false }
        let prefix = if $request.cursor == 0 { '' } else { $request.buffer | str substring --utf-8-bytes 0..<($request.cursor) }
        let suffix = $request.buffer | str substring --utf-8-bytes ($request.cursor)..
        if $response.result != ($prefix + $response.insertion + $suffix) { return false }
        if $response.result_cursor != ($request.cursor + ($response.insertion | str length --utf-8-bytes)) { return false }
        reedline-ai-byte-to-grapheme $response.result $response.result_cursor | ignore
        true
    } catch { false }
}

# Cancellation discards the UI result, but never SIGKILLs a helper that owns
# children. Preserve the one-worker bound until its own deadline/cleanup finishes.
def --env reedline-ai-drain [] {
    let draining = $env.REEDLINE_AI_DRAINING? | default null
    if $draining == null { return false }
    if (job list | any {|item| $item.id == $draining.worker }) { return true }
    try { job flush --tag $draining.tag } catch { }
    $env.REEDLINE_AI_DRAINING = null
    false
}

def --env reedline-ai-cancel [--quiet] {
    let state = $env.REEDLINE_AI_STATE? | default null
    if $state != null {
        if $state.worker != null and (job list | any {|item| $item.id == $state.worker }) {
            $env.REEDLINE_AI_DRAINING = {worker: $state.worker, tag: $state.tag}
        } else {
            try { job flush --tag $state.tag } catch { }
        }
    }
    $env.REEDLINE_AI_STATE = null
    reedline-ai-drain | ignore
    if not $quiet { print 'reedline-ai: discarded. Any in-flight worker drains within its bounded timeout.' }
}

def --env reedline-ai-request [] {
    if not ($env.REEDLINE_AI_ENABLED? | default false) { return }
    reedline-ai-cancel --quiet
    if (reedline-ai-drain) {
        print 'reedline-ai: one bounded request is still draining; no new request was started.'
        return
    }
    let buffer = commandline
    if ($buffer | str trim | is-empty) { return }
    if ($buffer | str length --utf-8-bytes) > 8192 or not (reedline-ai-safe-text $buffer) {
        print 'reedline-ai: only single-line input of at most 8192 UTF-8 bytes is supported.'
        return
    }
    let cursor = reedline-ai-grapheme-to-byte $buffer (commandline get-cursor)
    let tag = random int 1000000000..2000000000
    let request = {
        protocol: 1
        request_id: (random uuid)
        shell: {kind: nushell, version: (version | get version)}
        cwd: $env.PWD
        buffer: $buffer
        cursor: $cursor
    }
    let executable = $env.REEDLINE_AI_EXECUTABLE
    let arguments = $env.REEDLINE_AI_ARGUMENTS? | default []
    let transport = $env.REEDLINE_AI_TRANSPORT
    let python = $env.REEDLINE_AI_PYTHON
    let envelope = {backend: $executable, arguments: $arguments, request: $request}
    let worker = job spawn {
        # Do not allow an arbitrary ENV_CONVERSIONS override to export state.
        hide-env --ignore-errors REEDLINE_AI_STATE
        let outcome = try {
            let output = $envelope | to json --raw | ^$python $transport --stdin | complete
            if $output.exit_code != 0 or ($output.stdout | str length --utf-8-bytes) > 65536 {
                {ok: false}
            } else {
                {ok: true, response: ($output.stdout | from json)}
            }
        } catch { {ok: false} }
        $outcome | job send 0 --tag $tag
    }
    $env.REEDLINE_AI_STATE = {request: $request, tag: $tag, worker: $worker, ready: null}
    print 'reedline-ai: requested. Keep editing; Ctrl+Alt+P previews when ready.'
}

def --env reedline-ai-preview [] {
    let state = $env.REEDLINE_AI_STATE? | default null
    if $state == null { return }
    if (commandline) != $state.request.buffer or (reedline-ai-grapheme-to-byte (commandline) (commandline get-cursor)) != $state.request.cursor or $env.PWD != $state.request.cwd {
        reedline-ai-cancel --quiet
        print 'reedline-ai: discarded a stale result; request again for the current line.'
        return
    }
    if $state.ready != null {
        print $'reedline-ai preview: ($state.ready.result | to json --raw)'
        return
    }
    let outcome = try { job recv --tag $state.tag --timeout 0sec } catch { null }
    if $outcome == null {
        print 'reedline-ai: still pending; editing remains available.'
        return
    }
    if not $outcome.ok or not (reedline-ai-check-response $state.request $outcome.response) {
        reedline-ai-cancel --quiet
        print 'reedline-ai: request failed, timed out, or returned an invalid response.'
        return
    }
    let response = $outcome.response
    if ($response.insertion | is-empty) {
        reedline-ai-cancel --quiet
        print 'reedline-ai: no suggestion.'
        return
    }
    print $'reedline-ai preview: ($response.result | to json --raw)'
    if $response.additional_acceptance_required or $response.risk_review.incomplete or not ($response.risk_review.warnings | is-empty) {
        reedline-ai-cancel --quiet
        print 'reedline-ai: acceptance blocked by risk review. No extra-confirmation override is implemented.'
        return
    }
    $env.REEDLINE_AI_STATE = ($state | upsert ready $response | upsert worker null)
    print 'Ctrl+Alt+A inserts this preview only. Enter remains the separate native execution key.'
}

def --env reedline-ai-accept [] {
    let state = $env.REEDLINE_AI_STATE? | default null
    if $state == null or $state.ready == null { return }
    let response = $state.ready
    if (commandline) != $state.request.buffer or (reedline-ai-grapheme-to-byte (commandline) (commandline get-cursor)) != $state.request.cursor or $env.PWD != $state.request.cwd {
        reedline-ai-cancel --quiet
        print 'reedline-ai: stale preview discarded.'
        return
    }
    if not (reedline-ai-check-response $state.request $response) or $response.additional_acceptance_required or $response.risk_review.incomplete or not ($response.risk_review.warnings | is-empty) {
        reedline-ai-cancel --quiet
        return
    }
    let cursor = reedline-ai-byte-to-grapheme $response.result $response.result_cursor
    reedline-ai-cancel --quiet
    # Never add --accept: that flag executes text rather than merely editing it.
    commandline edit --replace $response.result
    commandline set-cursor $cursor
}

def --env reedline-ai-enable [] {
    if ($env.REEDLINE_AI_ENABLED? | default false) { return }
    let executable = $env.REEDLINE_AI_EXECUTABLE? | default 'reedline-ai'
    let found = which $executable | where type == external
    if ($found | is-empty) { error make {msg: 'Set REEDLINE_AI_EXECUTABLE to the built reedline-ai binary first.'} }
    let python = $env.REEDLINE_AI_PYTHON? | default 'python3'
    let python_found = which $python | where type == external
    let transport = $env.REEDLINE_AI_TRANSPORT? | default $REEDLINE_AI_DEFAULT_TRANSPORT
    if ($python_found | is-empty) or not ($transport | path exists) {
        error make {msg: 'Python 3 and integrations/shared/bridge.py are required; set REEDLINE_AI_PYTHON and REEDLINE_AI_TRANSPORT if needed.'}
    }
    let bindings = [
        {name: reedline_ai_request, modifier: control_alt, keycode: char_g, mode: [emacs vi_insert vi_normal], event: {send: executehostcommand, cmd: 'reedline-ai-request; $env.REEDLINE_AI_HOST_EVENT = true'}}
        {name: reedline_ai_preview, modifier: control_alt, keycode: char_p, mode: [emacs vi_insert vi_normal], event: {send: executehostcommand, cmd: 'reedline-ai-preview; $env.REEDLINE_AI_HOST_EVENT = true'}}
        {name: reedline_ai_accept, modifier: control_alt, keycode: char_a, mode: [emacs vi_insert vi_normal], event: {send: executehostcommand, cmd: 'reedline-ai-accept; $env.REEDLINE_AI_HOST_EVENT = true'}}
        {name: reedline_ai_cancel, modifier: control_alt, keycode: char_x, mode: [emacs vi_insert vi_normal], event: {send: executehostcommand, cmd: 'reedline-ai-cancel; $env.REEDLINE_AI_HOST_EVENT = true'}}
    ]
    for binding in $bindings {
        if ($env.config.keybindings | any {|old| ($old.name == $binding.name) or ($old.modifier == $binding.modifier and $old.keycode == $binding.keycode) }) {
            error make {msg: 'A reedline-ai key or binding name is already configured; no bindings were changed.'}
        }
    }
    $env.REEDLINE_AI_EXECUTABLE = ($found | first | get path)
    $env.REEDLINE_AI_PYTHON = ($python_found | first | get path)
    $env.REEDLINE_AI_TRANSPORT = ($transport | path expand)
    $env.REEDLINE_AI_STATE = null
    $env.REEDLINE_AI_ENABLED = true
    $env.REEDLINE_AI_BINDINGS = $bindings
    $env.config.keybindings ++= $bindings
    # One inert-when-disabled hook; do not replace the user's existing hooks.
    if not ($env.REEDLINE_AI_HOOK_INSTALLED? | default false) {
        $env.config.hooks.pre_prompt ++= [{||
            # executehostcommand also starts a prompt. Only a completed adapter
            # key event may preserve its state for that one repaint.
            if ($env.REEDLINE_AI_HOST_EVENT? | default false) {
                $env.REEDLINE_AI_HOST_EVENT = false
            } else if ($env.REEDLINE_AI_ENABLED? | default false) {
                reedline-ai-cancel --quiet
            }
        }]
        $env.REEDLINE_AI_HOOK_INSTALLED = true
    }
}

def --env reedline-ai-disable [] {
    reedline-ai-cancel --quiet
    $env.REEDLINE_AI_ENABLED = false
    let owned = $env.REEDLINE_AI_BINDINGS? | default []
    let remaining = ($env.config.keybindings | where {|binding| not ($owned | any {|original| $binding == $original }) } | collect)
    # Nu merges a nonempty keybinding list by name; clear first to remove ours.
    $env.config.keybindings = []
    $env.config.keybindings = $remaining
    $env.REEDLINE_AI_BINDINGS = []
}
