# Optional Bash/Readline integration. Source explicitly; never changes profiles.
# Requires Bash >= 5.1 and Python >= 3.9. No generated text is evaluated.
[[ $- == *i* ]] || return 0
if (( BASH_VERSINFO[0] < 5 || (BASH_VERSINFO[0] == 5 && BASH_VERSINFO[1] < 1) )); then
    builtin printf '%s\n' 'reedline-ai: Bash 5.1 or newer is required.' >&2
    return 1
fi
if ! command -v python3 >/dev/null 2>&1; then
    builtin printf '%s\n' 'reedline-ai: Python 3 is required.' >&2
    return 1
fi
# Re-sourcing does not discard an open result pipe.
if [[ ${__rai_loaded:-0} == 1 ]]; then return 0; fi
__rai_bridge=$(builtin cd -- "$(command dirname -- "${BASH_SOURCE[0]}")" && builtin pwd)/bridge.py
if ! declare -p REEDLINE_AI_ARGS >/dev/null 2>&1; then
    declare -a REEDLINE_AI_ARGS=(--backend mock)
fi
if [[ ! $(declare -p REEDLINE_AI_ARGS) =~ ^declare\ -[^\ ]*a ]]; then
    builtin printf '%s\n' 'reedline-ai: REEDLINE_AI_ARGS must be a Bash indexed array.' >&2
    return 1
fi
# Native array elements preserve existing prompt code without reparsing it here.
# Refuse readonly, nameref, or associative prompt variables rather than altering
# their semantics or installing an adapter that cannot invalidate on Ctrl-C.
__rai_prompt_decl=$(declare -p PROMPT_COMMAND 2>/dev/null || :)
if [[ $__rai_prompt_decl =~ ^declare\ -[^\ ]*[rnA] ]]; then
    builtin printf '%s\n' 'reedline-ai: PROMPT_COMMAND must be writable and non-associative.' >&2
    return 1
fi
__rai_saved_prompt=("${PROMPT_COMMAND[@]}")
__rai_saved_prompt_decl=$__rai_prompt_decl
__rai_saved_prompt_array=0
__rai_saved_prompt_export=0
[[ $__rai_prompt_decl =~ ^declare\ -[^\ ]*a ]] && __rai_saved_prompt_array=1
[[ $__rai_prompt_decl =~ ^declare\ -[^\ ]*x ]] && __rai_saved_prompt_export=1
# Store only the three affected chords. bind parses its own documented data
# format when restoring; no original shell command is executed during restore.
__rai_saved_bindings=()
for __rai_map in emacs-standard vi-insert; do
    for __rai_type in -p -s -X; do
        while IFS= builtin read -r __rai_line; do
            case $__rai_line in
                '"\C-x\C-g":'*|'"\C-x\C-a":'*|'"\C-x\C-r":'*)
                    __rai_saved_bindings+=("$__rai_map" "$__rai_type" "$__rai_line") ;;
            esac
        done < <(builtin bind -m "$__rai_map" "$__rai_type")
    done
done
unset __rai_map __rai_type __rai_line
__rai_loaded=1
__rai_pending=0
__rai_previewed=0
__rai_confirmed=0
__rai_serial=0

# The first prompt command leaves $? intact for existing prompt functions.
__rai_prompt_reset() {
    local previous_status=$?
    __rai_clear
    return "$previous_status"
}

__rai_byte_count() { local LC_ALL=C; builtin printf '%s' "${#1}"; }
__rai_prefix_bytes() { local LC_ALL=C; builtin printf '%s' "${1:0:$2}"; }

__rai_note() { builtin printf '\n%s\n' "$1"; }

__rai_clear() {
    if [[ -n ${__rai_fd:-} ]]; then
        exec {__rai_fd}<&-
        unset __rai_fd
    fi
    __rai_pending=0
    __rai_previewed=0
    __rai_confirmed=0
    unset __rai_result __rai_result_cursor __rai_risk __rai_preview __rai_created __rai_preview_time
}

__rai_matches() {
    local now=$SECONDS
    [[ -n ${__rai_created:-} ]] &&
    (( now >= __rai_created && now - __rai_created < 30 )) &&
    { [[ $__rai_previewed != 1 ]] ||
      (( now >= __rai_preview_time && now - __rai_preview_time < 15 )); } &&
    [[ ${READLINE_LINE-} == "${__rai_buffer-}" &&
       ${READLINE_POINT-} == "${__rai_cursor-}" &&
       $PWD == "${__rai_cwd-}" &&
       ${HISTCMD-} == "${__rai_history-}" ]]
}

# Only drain an already-ready pipe. Inference never runs in a bind -x callback.
__rai_collect() {
    local -a fields=()
    if ! __rai_matches; then
        __rai_clear
        __rai_note 'reedline-ai: stale result discarded; request changed or expired.'
        return 1
    fi
    if ! IFS= builtin read -r -t 0 -u "$__rai_fd"; then
        __rai_note 'reedline-ai: pending; keep editing, then press Ctrl-x Ctrl-a to review.'
        return 1
    fi
    builtin mapfile -d '' -t -u "$__rai_fd" fields
    exec {__rai_fd}<&-
    unset __rai_fd
    __rai_pending=0
    if ! __rai_matches; then
        __rai_clear
        __rai_note 'reedline-ai: stale result discarded; buffer, cursor, directory or prompt changed.'
        return 1
    fi
    if (( ${#fields[@]} != 5 )) || [[ ${fields[0]-} != ok ]]; then
        local reason=${fields[0]-unavailable}
        # The bridge emits only these fixed error names, never provider stderr.
        case $reason in
            timeout|invalid-input|invalid-response|output-limit|backend-error|empty|unavailable) ;;
            *) reason=unavailable ;;
        esac
        __rai_clear
        __rai_note "reedline-ai: $reason; command line unchanged."
        return 1
    fi
    __rai_result=${fields[1]}
    __rai_result_cursor=${fields[2]}
    __rai_risk=${fields[3]}
    __rai_preview=${fields[4]}
    __rai_preview_time=$SECONDS
    __rai_previewed=1
    __rai_confirmed=0
    __rai_note "reedline-ai preview (JSON-escaped, not executed): $__rai_preview"
    if [[ $__rai_risk == 1 ]]; then
        __rai_note 'reedline-ai: risk or incomplete review; Ctrl-x Ctrl-r acknowledges this preview, then Ctrl-x Ctrl-a inserts.'
    else
        __rai_note 'reedline-ai: Ctrl-x Ctrl-a inserts this preview; Enter is still native Bash execution.'
    fi
    return 0
}

__rai_invoke() {
    # Whitespace-only input never starts Python or the backend.
    if [[ ! ${READLINE_LINE-} =~ [^[:space:]] ]]; then
        __rai_clear
        __rai_note 'reedline-ai: blank input; no request sent.'
        return 0
    fi
    if [[ $__rai_pending == 1 ]]; then
        __rai_collect || :
        return 0
    fi
    __rai_clear
    __rai_created=$SECONDS
    __rai_buffer=$READLINE_LINE
    __rai_cursor=$READLINE_POINT
    __rai_cwd=$PWD
    __rai_history=${HISTCMD-}
    __rai_serial=$((__rai_serial + 1))
    local request_id="${BASHPID}-${__rai_serial}"
    # bind -x uses locale characters; the protocol uses UTF-8 bytes.
    local prefix=${READLINE_LINE:0:READLINE_POINT}
    local byte_point
    byte_point=$(__rai_byte_count "$prefix")
    # Sensitive input goes through an anonymous pipe, not argv, files or logs.
    # The process substitution execs Python; closing its result pipe cancels it.
    exec {__rai_fd}< <(
        exec python3 "$__rai_bridge" "${REEDLINE_AI_TIMEOUT_SECONDS:-8}" -- \
            "${REEDLINE_AI_BIN:-reedline-ai}" "${REEDLINE_AI_ARGS[@]}" --adapter-request \
            < <(builtin printf '%s\0' "$request_id" "$BASH_VERSION" "$__rai_cwd" "$__rai_buffer" "$byte_point") \
            2>/dev/null
    )
    __rai_pending=1
    __rai_note 'reedline-ai: requested; Ctrl-x Ctrl-a reviews when ready, then Ctrl-x Ctrl-a inserts.'
    return 0
}

__rai_accept() {
    if [[ $__rai_pending == 1 ]]; then
        __rai_collect || :
        return 0 # A newly collected preview can never insert on the same key.
    fi
    if [[ $__rai_previewed != 1 ]]; then
        __rai_note 'reedline-ai: no preview; invoke with Ctrl-x Ctrl-g first.'
        return 0
    fi
    if ! __rai_matches; then
        __rai_clear
        __rai_note 'reedline-ai: stale preview discarded; invoke again.'
        return 0
    fi
    if [[ $__rai_risk == 1 && $__rai_confirmed != 1 ]]; then
        __rai_note 'reedline-ai: insertion blocked; acknowledge this preview with Ctrl-x Ctrl-r first.'
        return 0
    fi
    READLINE_LINE=$__rai_result
    local result_prefix
    result_prefix=$(__rai_prefix_bytes "$__rai_result" "$__rai_result_cursor")
    READLINE_POINT=${#result_prefix}
    __rai_clear
    __rai_note 'reedline-ai: inserted only, not executed. Review before pressing Enter.'
    return 0
}

__rai_confirm() {
    if [[ $__rai_previewed == 1 ]] && __rai_matches && [[ $__rai_risk == 1 ]]; then
        __rai_confirmed=1
        __rai_note 'reedline-ai: current preview acknowledged; Ctrl-x Ctrl-a inserts without executing.'
    else
        __rai_confirmed=0
        __rai_note 'reedline-ai: no matching risky preview to acknowledge.'
    fi
    return 0
}

# Only these chords are changed. Tab, Enter, history, arrows and editing stay native.
builtin bind -m emacs-standard -x '"\C-x\C-g":__rai_invoke'
builtin bind -m emacs-standard -x '"\C-x\C-a":__rai_accept'
builtin bind -m emacs-standard -x '"\C-x\C-r":__rai_confirm'
builtin bind -m vi-insert -x '"\C-x\C-g":__rai_invoke'
builtin bind -m vi-insert -x '"\C-x\C-a":__rai_accept'
builtin bind -m vi-insert -x '"\C-x\C-r":__rai_confirm'

# Reset first on every new primary prompt (Enter and Ctrl-C), then run existing
# prompt commands exactly once in their original order. No eval and no traps.
PROMPT_COMMAND=(__rai_prompt_reset "${PROMPT_COMMAND[@]}")
unset __rai_prompt_decl


reedline_ai_disable() {
    [[ ${__rai_loaded:-0} == 1 ]] || return 0
    __rai_clear
    local map chord index command_text callback expected current owned
    for map in emacs-standard vi-insert; do
        for chord in '\C-x\C-g' '\C-x\C-a' '\C-x\C-r'; do
            case $chord in
                '\C-x\C-g') callback=__rai_invoke ;;
                '\C-x\C-a') callback=__rai_accept ;;
                '\C-x\C-r') callback=__rai_confirm ;;
            esac
            expected="\"$chord\": \"$callback\""
            owned=0
            while IFS= builtin read -r current; do
                [[ $current == "$expected" ]] && owned=1
            done < <(builtin bind -m "$map" -X)
            # A user/plugin may have replaced or removed a chord since enable.
            # Restore load-time data only while our exact callback still owns it.
            [[ $owned == 1 ]] || continue
            builtin bind -m "$map" -r "$chord"
            for ((index=0; index<${#__rai_saved_bindings[@]}; index+=3)); do
                [[ ${__rai_saved_bindings[index]} == "$map" ]] || continue
                command_text=${__rai_saved_bindings[index+2]}
                [[ $command_text == "\"$chord\":"* ]] || continue
                if [[ ${__rai_saved_bindings[index+1]} == -X ]]; then
                    builtin bind -m "$map" -x "$command_text"
                else
                    builtin bind -m "$map" "$command_text"
                fi
            done
        done
    done
    local -a remaining=()
    local unchanged=1
    for command_text in "${PROMPT_COMMAND[@]}"; do
        [[ $command_text == __rai_prompt_reset ]] || remaining+=("$command_text")
    done
    if (( ${#remaining[@]} != ${#__rai_saved_prompt[@]} )); then
        unchanged=0
    else
        for ((index=0; index<${#remaining[@]}; index++)); do
            [[ ${remaining[index]} == "${__rai_saved_prompt[index]}" ]] || unchanged=0
        done
    fi
    if [[ $unchanged == 1 && -z $__rai_saved_prompt_decl ]]; then
        unset PROMPT_COMMAND
    elif [[ $unchanged == 1 && $__rai_saved_prompt_array == 0 ]]; then
        unset PROMPT_COMMAND
        PROMPT_COMMAND=${__rai_saved_prompt[0]-}
        [[ $__rai_saved_prompt_export == 1 ]] && export PROMPT_COMMAND
    else
        PROMPT_COMMAND=("${remaining[@]}")
    fi
    __rai_loaded=0
    __rai_note 'reedline-ai: disabled in this shell; adapter-owned bindings and prompt hook removed; later key changes preserved.'
}
