-- HTTP/JSON-RPC fallback implemented with `vim.system` plus curl.
--
-- The UDS path is preferred. HTTP is only used when configured and the socket
-- is unreachable, or when the user forces `transport = "http"`. Every call is
-- non-blocking: `vim.system` owns the child process and invokes the callback
-- from the event loop.

local config = require("knect.config")
local jsonl = require("knect.jsonl")

local M = {}

local function normalize_url(addr)
  if not addr or addr == "" then
    return nil
  end
  if not addr:match("^https?://") then
    addr = "http://" .. addr
  end
  return (addr:gsub("/+$", ""))
end

local function rpc_error(message, code, retryable)
  return {
    code = code,
    message = message,
    data = { retryable = retryable == true },
  }
end

local function finish(callback, err, result)
  vim.schedule(function()
    callback(err, result)
  end)
end

--- Return true when the fallback can run on this Neovim.
---@return boolean
function M.available()
  return type(vim.system) == "function" and vim.fn.executable(config.get().curl) == 1
end

--- Perform one JSON-RPC POST.
---
--- @param opts table { url, id, method, params, timeout_ms, curl? }
--- @param callback fun(err:table|nil, result:any)
--- @return table|nil handle with a `cancel()` method
function M.request(opts, callback)
  local cfg = config.get()
  local url = normalize_url(opts.url or cfg.http_addr)
  if not url then
    finish(callback, rpc_error("no HTTP address configured", -32003, false), nil)
    return nil
  end
  if type(vim.system) ~= "function" then
    finish(callback, rpc_error("vim.system is required for the HTTP fallback", -32603, false), nil)
    return nil
  end
  local curl = opts.curl or cfg.curl or "curl"
  if vim.fn.executable(curl) ~= 1 then
    finish(
      callback,
      rpc_error("curl is required for the HTTP fallback; install curl or use the UDS socket", -32603, false),
      nil
    )
    return nil
  end

  local body, encode_error = jsonl.encode({
    jsonrpc = "2.0",
    id = opts.id,
    method = opts.method,
    params = (opts.params == nil or (type(opts.params) == "table" and next(opts.params) == nil))
        and vim.empty_dict()
      or opts.params,
  })
  if body == nil then
    finish(callback, rpc_error("cannot encode request: " .. tostring(encode_error), -32600, false), nil)
    return nil
  end

  local timeout_ms = opts.timeout_ms or cfg.timeout_ms
  local max_time = math.max(1, math.ceil(timeout_ms / 1000))
  local args = {
    curl,
    "-sS",
    "--max-time",
    tostring(max_time),
    "-X",
    "POST",
    "-H",
    "Content-Type: application/json",
    "--data-binary",
    body,
    url .. "/rpc",
  }

  local done = false
  local job
  local ok, error_or_job = pcall(vim.system, args, { text = true }, function(obj)
    if done then
      return
    end
    done = true
    if obj.code ~= 0 then
      local stderr = obj.stderr or ""
      local code = obj.code == 28 and -32011 or -32603
      local message = obj.code == 28
          and "HTTP request timed out"
        or ("curl failed (exit " .. tostring(obj.code) .. "): " .. vim.trim(stderr))
      finish(callback, rpc_error(message, code, code == -32011), nil)
      return
    end
    local response, decode_error = jsonl.decode(obj.stdout or "")
    if response == nil then
      finish(callback, rpc_error("invalid JSON response: " .. tostring(decode_error), -32700, false), nil)
      return
    end
    if response.error ~= nil and response.error ~= vim.NIL then
      local err = response.error
      if type(err) ~= "table" then
        err = rpc_error(tostring(err), -32603, false)
      end
      finish(callback, err, nil)
      return
    end
    local result = response.result
    if result == vim.NIL then
      result = nil
    end
    finish(callback, nil, result)
  end)
  if not ok then
    finish(callback, rpc_error("cannot start curl: " .. tostring(error_or_job), -32603, false), nil)
    return nil
  end
  job = error_or_job

  return {
    job = job,
    cancel = function()
      if done then
        return
      end
      done = true
      pcall(function()
        job:kill(15)
      end)
    end,
  }
end

--- Perform a health probe against /healthz (used by tests and diagnostics).
---@param url string
---@param callback fun(err:table|nil, body:string|nil)
function M.healthz(url, callback)
  local base = normalize_url(url)
  if not base then
    finish(callback, rpc_error("no HTTP address configured", -32003, false), nil)
    return
  end
  local args = { config.get().curl, "-sS", "--max-time", "2", base .. "/healthz" }
  local ok, error_or_job = pcall(vim.system, args, { text = true }, function(obj)
    if obj.code ~= 0 then
      finish(callback, rpc_error("healthz request failed", -32603, true), nil)
      return
    end
    finish(callback, nil, obj.stdout)
  end)
  if not ok then
    finish(callback, rpc_error("cannot start curl: " .. tostring(error_or_job), -32603, false), nil)
  end
end

return M
