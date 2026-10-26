-- Mocked Clink API contract tests, not a Windows/Clink terminal pass.
local script = arg[1] or 'integrations/cmd/reedline-ai.lua'
local callbacks, bindings, filters = {}, {}, {}
local real_time, now = os.time, 1000
os.time = function() return now end
local files, pending_async, process_calls = {}, nil, {}
local native = {['emacs:"\\C-r"'] = 'reverse-search-history', ['emacs:"\\C-i"'] = 'complete', ['emacs:"\\C-xg"'] = 'glob-list-expansions'}
for k,v in pairs(native) do bindings[k] = v end
local async, response_blocked, response_invalid = true, false, false
local version = {version_encoded=1009034, version_major=1, version_minor=9, version_patch=34}
clink = {
 version_encoded = 10090034, version_major=1, version_minor=9, version_patch=34,
 promptfilter = function() local f={}; filters[#filters+1]=f; return f end,
 onbeginedit = function(f) callbacks.begin=f end,
 onendedit = function(f) callbacks.finish=f end,
 promptcoroutine = function(f) pending_async=f end,
 runcoroutineuntilcomplete = function() end,
 refilterprompt = function() for _,f in ipairs(filters) do f:filter() end end,
}
rl = {
 getbinding = function(k,m) return bindings[m..':'..k] end,
 setbinding = function(k,v,m)
  assert(v ~= nil, 'Clink 1.9.35 rejects nil')
  if v == '' then bindings[m..':'..k]=nil; return false end
  bindings[m..':'..k]=v; return true
 end,
}
settings = {get=function() return async end}
os.getpid = function() return 123 end
os.getcwd = function() return 'C:\\synthetic' end
os.rmdir = function() return true end
local old_remove = os.remove
os.remove = function(path) files[path]=nil; return true end
local old_open, old_popen = io.open, io.popen
io.open = function(path, mode)
 if mode == 'wxb' then
  assert(files[path] == nil)
  files[path]=''
  return {write=function(_,text) files[path]=files[path]..text; return true end, close=function() end}
 end
 assert(files[path], path)
 return {read=function() return files[path] end, close=function() end}
end
local function hex(s) return (s:gsub('.', function(c) return string.format('%02x', c:byte()) end)) end
local line, cursor = 'git  --short', 5
local folder='C:\\private\\reedline-ai-clink-0123456789abcdef0123456789abcdef'
local last_id, replacement
io.popenyield = function(command)
 assert(command:sub(1,2) == '""' and command:sub(-1) == '"', 'CMD outer quoting')
 process_calls[#process_calls+1]=command
 local text
 if command:find('%-%-prepare') then text=hex(folder)..'\n'
 else
  assert(command:find('%-%-run'))
  local envelope=files[folder..'\\request.json']; assert(envelope)
  local id=envelope:match('"request_id":"([^"]+)"'); last_id=id
  local buffer=envelope:match('"buffer":"([^"]+)"')
  local byte_cursor=tonumber(envelope:match('"cursor":(%d+)'))
  local insertion='status'
  local result=buffer:sub(1,byte_cursor)..insertion..buffer:sub(byte_cursor+1)
  if replacement then insertion=replacement; result=replacement end
  files[folder..'\\response.frame']=table.concat({replacement and 'RLA2R' or 'RLA1',hex(response_invalid and 'wrong' or id),hex(buffer),tostring(byte_cursor),hex(insertion),hex(result),tostring(replacement and #result or byte_cursor+#insertion),response_blocked and '1' or '0'},'\n')..'\n'
  text='READY\n'
 end
 return {read=function() return text end, close=function() end}, function() return true end
end
local output={}
local real_print=print
print=function(text) output[#output+1]=text end
local buffer={anchor=nil}
function buffer:getbuffer() return line end
function buffer:getcursor() return cursor end
function buffer:getanchor() return self.anchor end
function buffer:beginoutput() end
function buffer:refreshline() end
function buffer:beginundogroup() end
function buffer:endundogroup() end
function buffer:insert(text) line=line:sub(1,cursor-1)..text..line:sub(cursor); cursor=cursor+#text end
function buffer:remove(first, last) line=line:sub(1,first-1)..line:sub(last); cursor=first end
function buffer:setcursor(value) cursor=value end
local function complete_async()
 local f=pending_async; pending_async=nil; assert(f)
 local co=coroutine.create(function() f(true) end)
 local ok,err=coroutine.resume(co); assert(ok,err)
end
local function reset(text, point)
 callbacks.finish(); callbacks.begin()
 line=text; cursor=point or #text+1
end

assert(loadfile(script))()
assert(#process_calls==0)
local config={python='C:\\Python\\python.exe',backend='C:\\Tools\\reedline-ai.exe',helper='C:\\Tools\\bridge.py'}
bindings['emacs:"\\C-xq"']='custom-user-binding'
assert(not reedline_ai_enable(config)); assert(bindings['emacs:"\\C-xq"']=='custom-user-binding')
bindings['emacs:"\\C-xq"']=nil
assert(reedline_ai_enable(config))
for k,v in pairs(native) do assert(bindings[k]==v) end

reset('   '); reedline_ai_request(buffer); assert(#process_calls==0)
reset('\194\160\227\128\128'); reedline_ai_request(buffer); assert(#process_calls==0 and pending_async==nil)
reset('git  --short',5); reedline_ai_request(buffer); assert(#process_calls==0)
reedline_ai_accept(buffer); assert(line=='git  --short')
complete_async(); reedline_ai_accept(buffer); assert(line=='git  --short')
reedline_ai_preview(buffer); reedline_ai_accept(buffer); assert(line=='git status --short' and cursor==11)
reedline_ai_accept(buffer); assert(line=='git status --short')
assert(last_id=='clink-123-1')
reset('echo ',6); reedline_ai_request(buffer); complete_async(); reedline_ai_preview(buffer)
now=now+15; reedline_ai_accept(buffer); assert(line=='echo ', 'expired preview must not insert')
reset('echo ',6); reedline_ai_request(buffer); complete_async()
now=now+30; reedline_ai_preview(buffer); reedline_ai_accept(buffer); assert(line=='echo ', 'expired request must not preview')
reset('echo ',6); reedline_ai_request(buffer); complete_async(); reedline_ai_preview(buffer)
now=now-1; reedline_ai_accept(buffer); assert(line=='echo ', 'clock rollback must invalidate')

reset('git ',5); reedline_ai_request(buffer); line='git edited'; cursor=#line+1
complete_async(); reedline_ai_preview(buffer); reedline_ai_accept(buffer); assert(line=='git edited')
reset('git ',5); reedline_ai_request(buffer); complete_async(); reedline_ai_preview(buffer)
cursor=4; reedline_ai_accept(buffer); assert(line=='git ')

reset('git ',5); reedline_ai_request(buffer); complete_async(); reedline_ai_preview(buffer)
reset('git ',5); reedline_ai_accept(buffer); assert(line=='git ')
reset('git ',5); reedline_ai_request(buffer); reset('git ',5); complete_async(); reedline_ai_preview(buffer); reedline_ai_accept(buffer); assert(line=='git ')

for _,kind in ipairs({'risk','malformed'}) do
 reset('git ',5); response_blocked=kind=='risk'; response_invalid=kind=='malformed'
 reedline_ai_request(buffer); complete_async(); reedline_ai_preview(buffer); reedline_ai_accept(buffer); assert(line=='git ')
end
response_blocked,response_invalid=false,false
replacement='echo 世界'
reset('启动监控', 7); reedline_ai_request(buffer); complete_async(); reedline_ai_accept(buffer); assert(line=='启动监控')
reedline_ai_preview(buffer); reedline_ai_accept(buffer); assert(line==replacement and cursor==#replacement+1)
for _,kind in ipairs({'edit','selection','risk','expiry','cancel'}) do
 reset('启动监控',7); response_blocked=kind=='risk'
 reedline_ai_request(buffer); complete_async(); reedline_ai_preview(buffer)
 if kind=='edit' then line='edited'; cursor=1 end
 if kind=='selection' then buffer.anchor=1 end
 if kind=='expiry' then now=now+15 end
 if kind=='cancel' then reedline_ai_cancel(buffer) end
 reedline_ai_accept(buffer); assert(line~=replacement,kind)
 buffer.anchor=nil
end
response_blocked=false; replacement=nil
local count=#process_calls
reset('git ',5); async=false; reedline_ai_request(buffer); assert(#process_calls==count and pending_async==nil)
async=true; buffer.anchor=1; reedline_ai_request(buffer); assert(#process_calls==count and pending_async==nil); buffer.anchor=nil
reedline_ai_disable()
for k,v in pairs(native) do assert(bindings[k]==v) end
for k,_ in pairs(bindings) do assert(native[k], 'adapter binding was not restored: '..k) end
assert(reedline_ai_enable(config))
local old_getenv = os.getenv
os.getenv = function(name) if name == 'REEDLINE_AI_ENABLE' then return '0' end; return old_getenv(name) end
callbacks.begin()
assert(bindings['emacs:"\\C-xq"']==nil)
os.getenv = old_getenv
print=real_print; io.open=old_open; io.popen=old_popen; os.remove=old_remove; os.time=real_time
print('PASS: mocked Clink API lifecycle, async handoff, explicit preview/accept, risk/stale/selection gates and bindings')
