-- Zero-dependency JSON-RPC client for knectd.
--
-- UDS framing is newline-delimited JSON over a libuv pipe. The module keeps a
-- single pipe, correlates responses by request ID, dispatches SseEvent lines to
-- event subscribers, and reconnects with exponential backoff when a stream is
-- dropped. If the socket is unreachable and an HTTP address is configured, the
-- same API falls back to POST /rpc through `vim.system` + curl.

local config = require("knect.config")
local http = require("knect.http")
local jsonl = require("knect.jsonl")
local util = require("knect.util")

local uv = vim.uv or vim.loop

local M = {}

local state

local function rpc_error(code, message, retryable)
  return {
    code = code,
    message = message,
    data = { retryable = retryable == true },
  }
end

local function cancelled_error(message)
  return rpc_error(-32012, message or "request cancelled", false)
end

local function normalize_error(err)
  if type(err) == "table" then
    return err
  end
  return rpc_error(-32603, tostring(err), false)
end

local function normalize_params(params)
  if params == nil then
    return vim.empty_dict()
  end
  if type(params) == "table" and next(params) == nil then
    return vim.empty_dict()
  end
  return params
end

local function dispatch(message)
  if type(message) ~= "table" then
    return
  end
  local id = message.id
  if id ~= nil and id ~= vim.NIL then
    local entry = state.pending[id]
    if entry ~= nil then
      if message.error ~= nil and message.error ~= vim.NIL then
        entry:finish(normalize_error(message.error), nil)
      else
        local result = message.result
        if result == vim.NIL then
          result = nil
        end
        entry:finish(nil, result)
      end
    end
    return
  end
  if message.kind ~= nil then
    for _, subscriber in ipairs(state.subscribers) do
      local callback = subscriber.callback
      util.schedule(function()
        local ok, err = pcall(callback, message)
        if not ok then
          util.notify("event callback failed: " .. tostring(err), vim.log.levels.DEBUG)
        end
      end)
    end
  end
end

local function new_decoder()
  return jsonl.decoder(dispatch, function(line, err)
    util.notify(
      "discarding malformed JSON line: " .. tostring(err) .. " [" .. line:sub(1, 120) .. "]",
      vim.log.levels.WARN
    )
  end)
end

state = {
  closed = false,
  status = "idle",
  transport = nil,
  pipe = nil,
  connect_token = 0,
  connect_waiters = {},
  pending = {},
  next_id = 0,
  subscribers = {},
  reconnect_timer = nil,
  reconnect_attempt = 0,
  subscription = {
    active = false,
    inflight = false,
    id = nil,
    retry_timer = nil,
    last_error = nil,
  },
  decoder = new_decoder(),
}

local augroup = vim.api.nvim_create_augroup("KnectRpc", { clear = true })
vim.api.nvim_create_autocmd("VimLeavePre", {
  group = augroup,
  callback = function()
    M.close()
  end,
})

local function close_pipe()
  local pipe = state.pipe
  state.pipe = nil
  if pipe == nil then
    return
  end
  pcall(function()
    pipe:read_stop()
  end)
  if not pipe:is_closing() then
    pcall(function()
      pipe:close()
    end)
  end
end

local function cancel_timer(timer)
  if timer == nil then
    return
  end
  pcall(function()
    timer:stop()
  end)
  if not timer:is_closing() then
    pcall(function()
      timer:close()
    end)
  end
end

local function reject_pending(err)
  local ids = {}
  for id in pairs(state.pending) do
    ids[#ids + 1] = id
  end
  for _, id in ipairs(ids) do
    local entry = state.pending[id]
    if entry ~= nil then
      entry:finish(err, nil)
    end
  end
end

local function notify_subscribers_error(err)
  for _, subscriber in ipairs(state.subscribers) do
    if subscriber.on_error then
      local on_error = subscriber.on_error
      util.schedule(function()
        pcall(on_error, err)
      end)
    end
  end
end

local function flush_connect_waiters(err)
  local waiters = state.connect_waiters
  state.connect_waiters = {}
  for _, waiter in ipairs(waiters) do
    util.schedule(function()
      waiter(err)
    end)
  end
end

local schedule_reconnect
local ensure_subscription

local function retry_if_subscribers()
  if state.closed or #state.subscribers == 0 then
    return
  end
  state.status = "reconnecting"
  schedule_reconnect()
end

local function handle_disconnect(reason)
  if state.closed then
    return
  end
  close_pipe()
  state.subscription.active = false
  state.subscription.id = nil
  local message = "daemon connection closed"
  if type(reason) == "string" and reason ~= "" then
    message = message .. ": " .. reason
  end
  reject_pending(rpc_error(-32012, message, true))
  if #state.subscribers > 0 then
    state.status = "reconnecting"
    schedule_reconnect()
  else
    state.status = "idle"
  end
end

local function send_line(line, pipe)
  local target = pipe or state.pipe
  if target == nil or state.status ~= "connected" then
    return false
  end
  local ok = pcall(function()
    target:write(line, function(write_err)
      if write_err ~= nil and state.pipe == target then
        handle_disconnect(write_err)
      end
    end)
  end)
  return ok
end

local function http_send(entry)
  local cfg = config.get()
  local handle = http.request({
    url = cfg.http_addr,
    id = entry.id,
    method = entry.method,
    params = entry.params,
    timeout_ms = entry.timeout_ms,
    curl = cfg.curl,
  }, function(err, result)
    entry:finish(err, result)
  end)
  entry.http = handle
end

local ensure_connected

local function on_connected(token, pipe)
  if token ~= state.connect_token or state.closed then
    return
  end
  state.status = "connected"
  state.transport = "uds"
  state.reconnect_attempt = 0
  pipe:read_start(function(read_err, data)
    if state.closed or token ~= state.connect_token then
      return
    end
    if read_err ~= nil then
      handle_disconnect(tostring(read_err))
      return
    end
    if data == nil then
      handle_disconnect("eof")
      return
    end
    local ok, err = pcall(state.decoder.feed, state.decoder, data)
    if not ok then
      handle_disconnect(tostring(err))
    end
  end)
  flush_connect_waiters(nil)
  ensure_subscription()
end

schedule_reconnect = function()
  if state.closed or state.reconnect_timer ~= nil then
    return
  end
  local policy = config.get().reconnect
  local attempt = state.reconnect_attempt
  local delay = policy.initial_ms * (policy.multiplier ^ attempt)
  if delay > policy.max_ms then
    delay = policy.max_ms
  end
  state.reconnect_attempt = attempt + 1
  local owner = state
  local timer = uv.new_timer()
  state.reconnect_timer = timer
  timer:start(
    math.floor(delay),
    0,
    vim.schedule_wrap(function()
      if state.reconnect_timer == timer then
        state.reconnect_timer = nil
      end
      cancel_timer(timer)
      if state ~= owner or state.closed then
        return
      end
      state.status = "idle"
      ensure_connected(function(err)
        if err ~= nil then
          retry_if_subscribers()
        end
      end)
    end)
  )
end

ensure_connected = function(callback, force)
  if state.closed then
    callback(cancelled_error("client is closed"))
    return
  end
  if state.status == "connected" then
    callback(nil)
    return
  end
  if state.status == "connecting" or state.status == "reconnecting" then
    state.connect_waiters[#state.connect_waiters + 1] = callback
    return
  end

  local cfg = config.get()
  if state.transport == "http" then
    state.status = "connected"
    callback(nil)
    return
  end
  if cfg.transport == "http" or (not force and not cfg.auto_connect) then
    if cfg.transport == "http" then
      state.transport = "http"
      state.status = "connected"
      callback(nil)
    else
      callback(rpc_error(-32012, "knect is not connected; call rpc.connect() first", false))
    end
    return
  end

  state.status = "connecting"
  state.transport = "uds"
  state.connect_token = state.connect_token + 1
  state.connect_waiters[#state.connect_waiters + 1] = callback
  local token = state.connect_token
  local pipe = uv.new_pipe(false)
  state.pipe = pipe

  local function fail(err)
    if token ~= state.connect_token then
      return
    end
    close_pipe()
    state.status = "idle"
    if cfg.http_addr ~= nil and cfg.http_addr ~= "" and cfg.transport ~= "uds" then
      state.transport = "http"
      state.status = "connected"
      flush_connect_waiters(nil)
      ensure_subscription()
      return
    end
    state.transport = nil
    local message = "cannot connect to " .. tostring(cfg.socket_path)
    if type(err) == "string" and err ~= "" then
      message = message .. ": " .. err
    end
    flush_connect_waiters(rpc_error(-32603, message, true))
  end

  local ok, err = pcall(function()
    pipe:connect(cfg.socket_path, function(connect_err)
      if connect_err ~= nil then
        fail(tostring(connect_err))
      else
        on_connected(token, pipe)
      end
    end)
  end)
  if not ok then
    fail(tostring(err))
  end
end

--- Open the connection eagerly. Requests connect lazily when
--- `auto_connect = true` (the default).
---@param callback fun(err:table|nil)|nil
function M.connect(callback)
  ensure_connected(callback or function() end, true)
end

local Entry = {}
Entry.__index = Entry

function Entry:finish(err, result)
  if self.owner ~= state then
    return
  end
  local current = state.pending[self.id]
  if current ~= self then
    return
  end
  state.pending[self.id] = nil
  cancel_timer(self.timer)
  self.timer = nil
  if self.http ~= nil and self.http.cancel ~= nil then
    pcall(self.http.cancel)
    self.http = nil
  end
  local callback = self.callback
  if callback ~= nil then
    util.schedule(function()
      callback(err, result)
    end)
  end
end

--- Send a JSON-RPC request.
---
--- The callback receives `(err, result)`; exactly one is non-nil. The returned
--- request ID may be passed to `M.cancel` before the deadline.
---@param method string
---@param params table|nil
---@param callback fun(err:table|nil, result:any)|nil
---@param opts table|nil { timeout_ms?: number }
---@return integer|nil
function M.request(method, params, callback, opts)
  opts = opts or {}
  params = normalize_params(params)
  if state.closed then
    local cb = callback or function() end
    util.schedule(function()
      cb(cancelled_error("client is closed"), nil)
    end)
    return nil
  end
  state.next_id = state.next_id + 1
  local id = state.next_id
  local timeout_ms = opts.timeout_ms or config.get().timeout_ms
  local entry = setmetatable({
    id = id,
    method = method,
    params = params,
    callback = callback,
    timeout_ms = timeout_ms,
    owner = state,
  }, Entry)
  state.pending[id] = entry

  if timeout_ms ~= nil and timeout_ms > 0 then
    local timer = uv.new_timer()
    entry.timer = timer
    timer:start(
      math.floor(timeout_ms),
      0,
      function()
        cancel_timer(timer)
        if state.pending[id] == entry then
          entry:finish(
            rpc_error(-32011, string.format("request '%s' timed out", method), true),
            nil
          )
        end
      end
    )
  end

  ensure_connected(function(err)
    if err ~= nil then
      entry:finish(err, nil)
      return
    end
    if state.pending[id] ~= entry then
      return
    end
    if state.transport == "http" then
      http_send(entry)
      return
    end
    local request = {
      jsonrpc = "2.0",
      id = id,
      method = method,
      params = params or vim.empty_dict(),
    }
    local encoded, encode_err = jsonl.encode(request)
    if encoded == nil then
      entry:finish(rpc_error(-32600, "cannot encode request: " .. tostring(encode_err), false), nil)
      return
    end
    if not send_line(encoded .. "\n") then
      entry:finish(rpc_error(-32603, "connection is not writable", true), nil)
    end
  end, false)

  return id
end

--- Cancel an in-flight request. The callback receives a cancellation error.
---@param id integer|nil
function M.cancel(id)
  if id == nil then
    return
  end
  local entry = state.pending[id]
  if entry ~= nil then
    entry:finish(cancelled_error(), nil)
  end
end

--- Send a JSON-RPC notification (no ID, no response).
---@param method string
---@param params table|nil
function M.notify(method, params)
  params = normalize_params(params)
  ensure_connected(function(err)
    if err ~= nil or state.transport ~= "uds" then
      return
    end
    local encoded = jsonl.encode({
      jsonrpc = "2.0",
      method = method,
      params = params or vim.empty_dict(),
    })
    if encoded ~= nil then
      send_line(encoded .. "\n")
    end
  end, false)
end

local function schedule_subscription_retry()
  if state.closed or #state.subscribers == 0 then
    return
  end
  if state.subscription.retry_timer ~= nil then
    return
  end
  local owner = state
  local timer = uv.new_timer()
  state.subscription.retry_timer = timer
  timer:start(
    1000,
    0,
    vim.schedule_wrap(function()
      if state.subscription.retry_timer == timer then
        state.subscription.retry_timer = nil
      end
      cancel_timer(timer)
      if state ~= owner or state.closed then
        return
      end
      ensure_subscription()
    end)
  )
end

ensure_subscription = function()
  if state.closed or #state.subscribers == 0 then
    return
  end
  if state.subscription.active or state.subscription.inflight then
    return
  end
  if state.transport ~= "uds" then
    local err = rpc_error(
      -32603,
      "live events require the UDS JSONL transport; configure socket_path or use the HTTP UI",
      false
    )
    if state.subscription.last_error ~= err.message then
      state.subscription.last_error = err.message
      notify_subscribers_error(err)
    end
    return
  end
  local first = state.subscribers[1]
  local params = {}
  if first.topics ~= nil then
    params.topics = first.topics
  end
  if first.last_event_id ~= nil then
    params.last_event_id = first.last_event_id
  end
  state.subscription.inflight = true
  M.request("events.subscribe", params, function(err, result)
    state.subscription.inflight = false
    if state.closed then
      return
    end
    if err ~= nil then
      local message = util.format_error(err)
      if state.subscription.last_error ~= message then
        state.subscription.last_error = message
        notify_subscribers_error(err)
      end
      schedule_subscription_retry()
      return
    end
    state.subscription.last_error = nil
    state.subscription.active = true
    state.subscription.id = result and result.subscription_id
  end)
end

--- Subscribe to SseEvent lines on the UDS JSONL stream.
---
--- Returns a handle whose `stop()` removes the local callback. The daemon has
--- no unsubscribe method in schema v1; the socket is closed by `M.close()`.
---@param callback fun(event:table)
---@param opts table|nil { topics?: string[], last_event_id?: number, on_error?: fun(err:table) }
---@return table handle
function M.subscribe_events(callback, opts)
  opts = opts or {}
  callback = callback or function() end
  if state.closed then
    if opts.on_error ~= nil then
      util.schedule(function()
        pcall(opts.on_error, cancelled_error("client is closed"))
      end)
    end
    return {
      stop = function() end,
      id = function()
        return nil
      end,
    }
  end
  local subscriber = {
    callback = callback,
    on_error = opts.on_error,
    topics = opts.topics,
    last_event_id = opts.last_event_id,
  }
  state.subscribers[#state.subscribers + 1] = subscriber

  ensure_connected(function(err)
    if err ~= nil then
      notify_subscribers_error(err)
      retry_if_subscribers()
      return
    end
    ensure_subscription()
  end, false)

  return {
    stop = function()
      for index, current in ipairs(state.subscribers) do
        if current == subscriber then
          table.remove(state.subscribers, index)
          break
        end
      end
    end,
    id = function()
      return state.subscription.id
    end,
  }
end

--- Close the client and reject in-flight requests.
---@param opts table|nil { silent?: boolean }
function M.close(opts)
  opts = opts or {}
  if state == nil or state.closed then
    return
  end
  state.closed = true
  state.connect_token = state.connect_token + 1
  cancel_timer(state.reconnect_timer)
  state.reconnect_timer = nil
  cancel_timer(state.subscription.retry_timer)
  state.subscription.retry_timer = nil
  close_pipe()
  reject_pending(cancelled_error("client closed"))
  if not opts.silent then
    notify_subscribers_error(cancelled_error("client closed"))
  end
  state.subscribers = {}
  state.pending = {}
  state.status = "closed"
end

--- Reset module state. Intended for the headless test suite.
function M._reset()
  M.close({ silent = true })
  state = {
    closed = false,
    status = "idle",
    transport = nil,
    pipe = nil,
    connect_token = 0,
    connect_waiters = {},
    pending = {},
    next_id = 0,
    subscribers = {},
    reconnect_timer = nil,
    reconnect_attempt = 0,
    subscription = {
      active = false,
      inflight = false,
      id = nil,
      retry_timer = nil,
      last_error = nil,
    },
    decoder = new_decoder(),
  }
end

--- Introspection used by the test suite.
---@return table
function M._stats()
  local pending = 0
  for _ in pairs(state.pending) do
    pending = pending + 1
  end
  return {
    status = state.status,
    transport = state.transport,
    pending = pending,
    subscribers = #state.subscribers,
    reconnect_attempt = state.reconnect_attempt,
    subscription_active = state.subscription.active,
  }
end

return M
