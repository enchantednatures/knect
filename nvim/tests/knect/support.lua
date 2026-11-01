-- Shared helpers for the headless Plenary suite: contract fixtures plus small
-- in-process UDS and HTTP mock servers built on vim.uv.

local jsonl = require("knect.jsonl")

local M = {}

local uv = vim.uv or vim.loop

function M.repo_root()
  if vim.env.KNECT_REPO_ROOT ~= nil and vim.env.KNECT_REPO_ROOT ~= "" then
    return vim.env.KNECT_REPO_ROOT
  end
  local source = debug.getinfo(1, "S").source:sub(2)
  return vim.fn.fnamemodify(source, ":p:h:h:h:h")
end

function M.contract(name)
  local path = M.repo_root() .. "/testdata/contracts/" .. name
  local handle = assert(io.open(path, "r"))
  local text = handle:read("*a")
  handle:close()
  return vim.json.decode(text)
end

function M.temp_socket_path()
  local path = vim.fn.tempname() .. ".sock"
  vim.fn.mkdir(vim.fn.fnamemodify(path, ":h"), "p")
  os.remove(path)
  return path
end

function M.wait_for(predicate, timeout, label)
  local ok = vim.wait(timeout or 3000, predicate, 10)
  assert(ok, "timed out waiting for " .. (label or "condition"))
end

function M.tap_handle()
  return {
    id = "tap-1",
    target = { kind = "broker", namespace = "default", name = "orders" },
    state = "active",
    created = "2026-10-31T12:05:00Z",
    expires = "2026-10-31T12:10:00Z",
    max_events = 500,
    callback_url = "http://127.0.0.1:7421/tap/tap-1",
    filter = vim.NIL,
  }
end

function M.tap_event(overrides)
  local data = {
    id = "evt-1",
    ce_id = "ce-abc-123",
    ce_source = "/apis/v1/namespaces/default/pingsources/orders",
    ce_type = "dev.knative.sources.ping",
    ce_subject = vim.NIL,
    ce_time = "2026-10-31T12:05:30Z",
    headers = {},
    payload = { order_id = "order-42", total = 17.5 },
    received_at = "2026-10-31T12:05:31Z",
  }
  for key, value in pairs(overrides or {}) do
    data[key] = value
  end
  return {
    seq = 7,
    ts = "2026-10-31T12:05:31Z",
    kind = "tap.event",
    data = data,
  }
end

local function encode_response(id, result, err)
  return vim.json.encode({
    jsonrpc = "2.0",
    id = id,
    result = err ~= nil and vim.NIL or result,
    error = err ~= nil and err or vim.NIL,
  })
end

--- Start a newline-delimited JSON-RPC server on a Unix socket.
---
--- `responses` maps a method name to either a value or a function
--- `(params, ctx)`. `ctx` provides `respond(result)`, `respond_error(err)`,
--- `notify(event)`, and `later(ms, fn)`.
function M.start_mock(path, responses)
  local server = {
    path = path,
    responses = responses or {},
    requests = {},
    clients = {},
    closed = false,
  }

  os.remove(path)
  local listener = uv.new_pipe(false)
  server.listener = listener
  local ok, bind_err = listener:bind(path)
  assert(ok, "cannot bind mock socket " .. path .. ": " .. tostring(bind_err))

  local function remove_client(client)
    for index, candidate in ipairs(server.clients) do
      if candidate == client then
        table.remove(server.clients, index)
        break
      end
    end
    if not client:is_closing() then
      client:close()
    end
  end

  function server:send(client, value)
    if server.closed or client == nil or client:is_closing() then
      return
    end
    local encoded, err = jsonl.encode(value)
    assert(encoded, err)
    client:write(encoded .. "\n")
  end

  function server:emit(event, only_client)
    if only_client ~= nil then
      self:send(only_client, event)
      return
    end
    for _, client in ipairs(self.clients) do
      self:send(client, event)
    end
  end

  function server:request_count(method)
    return self.requests[method] or 0
  end

  function server:close()
    if self.closed then
      return
    end
    self.closed = true
    for index = #self.clients, 1, -1 do
      local client = self.clients[index]
      if not client:is_closing() then
        client:close()
      end
    end
    self.clients = {}
    if not listener:is_closing() then
      listener:close()
    end
    os.remove(path)
  end

  local function dispatch(client, message)
    if type(message) ~= "table" or message.method == nil then
      return
    end
    local method = message.method
    server.requests[method] = (server.requests[method] or 0) + 1
    local handler = server.responses[method]
    local function respond(result)
      server:send(client, {
        jsonrpc = "2.0",
        id = message.id,
        result = result,
        error = vim.NIL,
      })
    end
    local function respond_error(err)
      server:send(client, {
        jsonrpc = "2.0",
        id = message.id,
        result = vim.NIL,
        error = err,
      })
    end
    local ctx = {
      respond = respond,
      respond_error = respond_error,
      notify = function(event)
        server:emit(event, client)
      end,
      later = function(ms, fn)
        local timer = uv.new_timer()
        timer:start(
          ms,
          0,
          vim.schedule_wrap(function()
            timer:stop()
            timer:close()
            if not server.closed then
              fn()
            end
          end)
        )
      end,
      client = client,
    }
    if type(handler) == "function" then
      handler(message.params or vim.empty_dict(), ctx)
    elseif handler ~= nil then
      respond(handler)
    else
      respond_error({
        code = -32601,
        message = "method not found: " .. tostring(method),
        data = { retryable = false },
      })
    end
  end

  local function accept_client(client)
    server.clients[#server.clients + 1] = client
    local decoder = jsonl.decoder(function(message)
      dispatch(client, message)
    end, function()
      return
    end)
    client:read_start(function(err, data)
      if err ~= nil or data == nil then
        remove_client(client)
        return
      end
      decoder:feed(data)
    end)
  end

  listener:listen(64, function(listen_err)
    if listen_err ~= nil or server.closed then
      return
    end
    local client = uv.new_pipe(false)
    listener:accept(client)
    accept_client(client)
  end)

  return server
end

--- Start a minimal HTTP/1.1 JSON-RPC server on loopback.
function M.start_http_mock(responses)
  local server = {
    responses = responses or {},
    requests = {},
    clients = {},
    closed = false,
  }
  local tcp = uv.new_tcp()
  server.tcp = tcp
  assert(tcp:bind("127.0.0.1", 0), "cannot bind HTTP mock")
  local address = tcp:getsockname()
  server.port = address.port
  server.url = "http://127.0.0.1:" .. tostring(address.port)

  function server:request_count(method)
    return self.requests[method] or 0
  end

  function server:close()
    if self.closed then
      return
    end
    self.closed = true
    for index = #self.clients, 1, -1 do
      local client = self.clients[index]
      if not client:is_closing() then
        client:close()
      end
    end
    self.clients = {}
    if not tcp:is_closing() then
      tcp:close()
    end
  end

  local function handle_body(client, body)
    local ok, message = pcall(vim.json.decode, body)
    if not ok then
      return
    end
    server.requests[message.method] = (server.requests[message.method] or 0) + 1
    local handler = server.responses[message.method]
    local result
    local err
    if type(handler) == "function" then
      local ctx = {
        respond = function(value)
          result = value
        end,
        respond_error = function(value)
          err = value
        end,
      }
      handler(message.params or vim.empty_dict(), ctx)
    elseif handler ~= nil then
      result = handler
    else
      err = {
        code = -32601,
        message = "method not found: " .. tostring(message.method),
        data = { retryable = false },
      }
    end
    local encoded = encode_response(message.id, result, err)
    local response = table.concat({
      "HTTP/1.1 200 OK",
      "Content-Type: application/json",
      "Content-Length: " .. #encoded,
      "Connection: close",
      "",
      encoded,
    }, "\r\n")
    client:write(response, function()
      if not client:is_closing() then
        client:close()
      end
    end)
  end

  tcp:listen(16, function(listen_err)
    if listen_err ~= nil or server.closed then
      return
    end
    local client = uv.new_tcp()
    tcp:accept(client)
    server.clients[#server.clients + 1] = client
    local buffer = ""
    client:read_start(function(read_err, data)
      if read_err ~= nil or data == nil then
        if not client:is_closing() then
          client:close()
        end
        return
      end
      buffer = buffer .. data
      local header_end = buffer:find("\r\n\r\n", 1, true)
      if header_end == nil then
        return
      end
      local headers = buffer:sub(1, header_end - 1)
      local length = tonumber(headers:match("[Cc]ontent%-[Ll]ength:%s*(%d+)")) or 0
      local body = buffer:sub(header_end + 4)
      if #body < length then
        return
      end
      handle_body(client, body:sub(1, length))
    end)
  end)

  return server
end

return M
