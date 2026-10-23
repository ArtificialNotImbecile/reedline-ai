-- Opt-in CMD integration via Clink, not a replacement shell.
-- Requires Clink >= 1.7.12 and Python 3.10+. No requests occur merely on load.
-- Set REEDLINE_AI_ENABLE=1, REEDLINE_AI_PYTHON and REEDLINE_AI_EXECUTABLE
-- explicitly before loading this script. See docs/cmd-adapter.md.

local enabled, attempted = false, false
local generation, serial = 0, 0
local pending, busy = nil, nil
local source = debug.getinfo(1, 'S').source
local directory = source:sub(1, 1) == '@' and source:sub(2):match('^(.*)[/\\]') or nil
local config
local keys = {
    {key = [["\C-xg"]], name = 'reedline_ai_request'},
    {key = [["\C-xp"]], name = 'reedline_ai_preview'},
    {key = [["\C-xa"]], name = 'reedline_ai_accept'},
    {key = [["\C-xx"]], name = 'reedline_ai_cancel'},
}
local maps = {'emacs', 'vi-insert'}

local function blank(line)
    line = line:gsub('%s', '')
    -- Unicode White_Space bytes, without depending on a particular Lua utf8 API.
    for _, space in ipairs({'\194\133', '\194\160', '\225\154\128',
        '\226\128\128', '\226\128\129', '\226\128\130', '\226\128\131',
        '\226\128\132', '\226\128\133', '\226\128\134', '\226\128\135',
        '\226\128\136', '\226\128\137', '\226\128\138', '\226\128\168',
        '\226\128\169', '\226\128\175', '\226\129\159', '\227\128\128'}) do
        line = line:gsub(space, '')
    end
    return line == ''
end

local function json_string(value)
    assert(type(value) == 'string')
    return '"' .. value:gsub('[%z\1-\31\\"]', function(c)
        if c == '"' then return '\\"' end
        if c == '\\' then return '\\\\' end
        return string.format('\\u%04x', string.byte(c))
    end) .. '"'
end

local function unhex(value)
    if not value or #value % 2 ~= 0 or value:find('[^0-9a-f]') then return nil end
    return (value:gsub('..', function(pair) return string.char(tonumber(pair, 16)) end))
end

-- Only configured executable/helper paths and an opaque temporary path reach
-- CMD's process launcher. Reject expansion/metacharacters, including % and !.
local function quote_path(value)
    if type(value) ~= 'string' or not value:match('^%a:[/\\]')
        or value:find('[%z\1-\31\127"%%!%^&|<>]') then
        error('Use absolute local paths without CMD expansion characters.')
    end
    return '"' .. value .. '"'
end

local function message(buffer, text)
    buffer:beginoutput()
    print('reedline-ai: ' .. text)
    buffer:refreshline()
end

local function unchanged(buffer, request)
    return enabled and buffer:getanchor() == nil and request.generation == generation
        and buffer:getbuffer() == request.buffer
        and buffer:getcursor() - 1 == request.cursor
        and os.getcwd() == request.cwd
end

local function parse_frame(text, request)
    if type(text) ~= 'string' or #text > 65536 then return nil end
    local fields = {}
    for value in text:gmatch('([^\n]*)\n') do fields[#fields + 1] = value:gsub('\r$', '') end
    if #fields ~= 8 or fields[1] ~= 'RLA1' or (fields[8] ~= '0' and fields[8] ~= '1') then return nil end
    local id, original, insertion, result = unhex(fields[2]), unhex(fields[3]), unhex(fields[5]), unhex(fields[6])
    local cursor, result_cursor = tonumber(fields[4]), tonumber(fields[7])
    if id ~= request.id or original ~= request.buffer or cursor ~= request.cursor
        or not insertion or not result or #insertion > 1024 or #result > 9216 or insertion:find('```', 1, true)
        or result_cursor ~= cursor + #insertion
        or result ~= original:sub(1, cursor) .. insertion .. original:sub(cursor + 1)
        or result:find('[%z\1-\31\127]') then return nil end
    return {insertion = insertion, result = result, cursor = result_cursor, blocked = fields[8] == '1'}
end

local function envelope(request)
    local args = {}
    for _, argument in ipairs(request.config.arguments) do args[#args + 1] = json_string(argument) end
    local version = 'Clink ' .. tostring(clink.version_major) .. '.' .. tostring(clink.version_minor) .. '.' .. tostring(clink.version_patch)
    return '{"backend":' .. json_string(request.config.backend) .. ',"arguments":[' .. table.concat(args, ',')
        .. '],"request":{"protocol":1,"request_id":' .. json_string(request.id)
        .. ',"shell":{"kind":"cmd","version":' .. json_string(version)
        .. '},"cwd":' .. json_string(request.cwd) .. ',"buffer":' .. json_string(request.buffer)
        .. ',"cursor":' .. tostring(request.cursor) .. '}}'
end

local function read_process(command)
    -- Only call inside a live asynchronous prompt coroutine. A read in a key
    -- callback or a synchronous prompt filter would block the editor.
    local pipe, close = io.popenyield(command, 'rb')
    if not pipe then error('helper unavailable') end
    local text = pipe:read(65537) or ''
    if type(close) == 'function' then close() else pipe:close() end
    if #text > 65536 then error('oversized helper output') end
    return text
end

local function run(request, async)
    if not async then request.error = true; busy = nil; return end
    clink.runcoroutineuntilcomplete(coroutine.running())
    local folder, path
    local ok, result = pcall(function()
        local command = quote_path(request.config.python) .. ' ' .. quote_path(request.config.helper)
        folder = unhex(read_process(command .. ' --prepare'):gsub('[\r\n]+$', ''))
        if not folder then error('private temporary directory unavailable') end
        path = folder .. '\\request.json'
        quote_path(path) -- Validate before writing private data to the new directory.
        -- Cancellation/new prompt during preparation must start no backend.
        if pending ~= request or request.generation ~= generation then return nil end
        local file = assert(io.open(path, 'wxb'))
        local wrote, failure = file:write(envelope(request))
        file:close()
        if not wrote then error(failure) end
        if read_process(command .. ' --run ' .. quote_path(path)):gsub('[\r\n]+$', '') ~= 'READY' then error('backend failed') end
        local response_file = assert(io.open(folder .. '\\response.frame', 'rb'))
        local frame = response_file:read(65537) or ''
        response_file:close()
        return parse_frame(frame, request)
    end)
    if path then os.remove(path) end
    if folder then os.remove(folder .. '\\response.frame'); os.rmdir(folder) end
    busy = nil
    if pending == request and request.generation == generation then
        if ok and result then request.response = result else request.error = true end
    end
end

local filter = clink.promptfilter(99)
function filter:filter()
    local request = busy
    if request and not request.launched then
        request.launched = true
        clink.promptcoroutine(function(async) run(request, async) end, request.id)
    end
    return nil
end

function reedline_ai_request(buffer)
    if not enabled then return end
    pending = nil
    if busy then
        message(buffer, 'one bounded request is still running; wait for it to finish.')
        return
    end
    if buffer:getanchor() ~= nil then message(buffer, 'clear the active selection before requesting.'); return end
    local line = buffer:getbuffer()
    if blank(line) then return end
    if #line > 8192 or line:find('[%z\1-\31\127]') then
        message(buffer, 'only single-line input of at most 8192 UTF-8 bytes is supported.')
        return
    end
    if not settings.get('prompt.async') then
        message(buffer, 'async prompts are disabled; no request was started.')
        return
    end
    serial = serial + 1
    local request = {id = 'clink-' .. tostring(os.getpid()) .. '-' .. tostring(serial),
        buffer = line, cursor = buffer:getcursor() - 1, cwd = os.getcwd(), generation = generation, config = config}
    if #envelope(request) > 24576 then
        message(buffer, 'request exceeds the transport budget.')
        return
    end
    pending, busy = request, request
    clink.refilterprompt()
    message(buffer, 'requested. Keep editing; Ctrl-X then P previews when ready.')
end

function reedline_ai_preview(buffer)
    local request = pending
    if not request then return end
    if not unchanged(buffer, request) then
        pending = nil
        message(buffer, 'stale result discarded; request again for the current line.')
    elseif request.error then
        pending = nil
        message(buffer, 'request failed, timed out, or returned an invalid response.')
    elseif not request.response then
        message(buffer, 'still pending; editing remains available.')
    elseif #request.response.insertion == 0 then
        pending = nil
        message(buffer, 'no suggestion.')
    else
        message(buffer, 'preview: ' .. json_string(request.response.result))
        if request.response.blocked then
            pending = nil
            message(buffer, 'acceptance blocked by risk review. No override is implemented.')
        else
            request.previewed = true
            message(buffer, 'Ctrl-X then A inserts only. Enter remains the separate native execution key.')
        end
    end
end

function reedline_ai_accept(buffer)
    local request = pending
    if not request or not request.previewed or not request.response then return end
    pending = nil
    if request.response.blocked or not unchanged(buffer, request) then
        message(buffer, 'stale or blocked preview discarded.')
        return
    end
    -- Clink indices are one-based UTF-8 byte positions. Insert in one undo group.
    -- No Readline accept-line command, CMD evaluation, or synthetic Enter is used.
    buffer:beginundogroup()
    buffer:insert(request.response.insertion)
    buffer:setcursor(request.response.cursor + 1)
    buffer:endundogroup()
    buffer:refreshline()
end

function reedline_ai_cancel(buffer)
    pending = nil
    if buffer then message(buffer, 'discarded; any running worker remains bounded by its 15-second timeout.') end
end

function reedline_ai_enable(options)
    if enabled then return true end
    if not clink.version_encoded or clink.version_encoded < 10070012 then return false end
    options = options or {}
    local candidate = {
        python = options.python or os.getenv('REEDLINE_AI_PYTHON'),
        backend = options.backend or os.getenv('REEDLINE_AI_EXECUTABLE'),
        helper = options.helper or (directory and directory .. '\\..\\shared\\bridge.py'),
        arguments = options.arguments or {},
    }
    local ok = pcall(function()
        quote_path(candidate.python); quote_path(candidate.backend); quote_path(candidate.helper)
        assert(candidate.python:lower():match('%.exe$') and candidate.backend:lower():match('%.exe$'))
        assert(type(candidate.arguments) == 'table' and #candidate.arguments <= 32)
        for _, argument in ipairs(candidate.arguments) do assert(type(argument) == 'string') end
    end)
    if not ok then return false end
    for _, map in ipairs(maps) do
        for _, binding in ipairs(keys) do
            -- Do not overwrite a user's existing custom or native bindings.
            if rl.getbinding(binding.key, map) ~= nil then return false end
        end
    end
    config = candidate
    local installed = {}
    for _, map in ipairs(maps) do
        for _, binding in ipairs(keys) do
            if not rl.setbinding(binding.key, '"luafunc:' .. binding.name .. '"', map) then
                for _, old in ipairs(installed) do rl.setbinding(old.key, nil, old.map) end
                return false
            end
            installed[#installed + 1] = {key = binding.key, map = map}
        end
    end
    enabled = true
    return true
end

function reedline_ai_disable()
    enabled, pending = false, nil
    for _, map in ipairs(maps) do
        for _, binding in ipairs(keys) do
            if rl.getbinding(binding.key, map) == '"luafunc:' .. binding.name .. '"' then
                rl.setbinding(binding.key, nil, map)
            end
        end
    end
end

local function reset_prompt()
    generation = generation + 1
    pending = nil
end
clink.onendedit(reset_prompt)
clink.onbeginedit(function()
    reset_prompt()
    local requested = os.getenv('REEDLINE_AI_ENABLE')
    if requested == '0' then
        if enabled then reedline_ai_disable() end
        attempted = false
    elseif not attempted and requested == '1' then
        attempted = true
        if not reedline_ai_enable() then
            print('reedline-ai: not enabled. Check Clink version, explicit absolute paths, and key conflicts.')
        end
    end
end)
