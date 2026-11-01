-- Configuration defaults and validation for knect.nvim.
--
-- The plugin has no Lua dependencies. Every option can be set through
-- `require("knect").setup({ ... })`; environment variables are used as the
-- initial defaults so tests and wrappers can point at a disposable daemon.

local M = {}

local defaults = {
  -- Unix domain socket used by knectd for newline-delimited JSON-RPC.
  socket_path = "/tmp/knectd.sock",
  -- Optional HTTP endpoint, for example "http://127.0.0.1:7420". When set it
  -- is used only if the socket cannot be reached (unless transport = "http").
  http_addr = nil,
  -- "auto" prefers UDS, "uds" forbids HTTP, "http" skips the socket.
  transport = "auto",
  -- Connect on the first command instead of at startup. The plugin never
  -- blocks Vim while connecting.
  auto_connect = true,
  -- Per-request deadline in milliseconds.
  timeout_ms = 5000,
  -- Input debounce used by interactive pickers/refreshes.
  debounce_ms = 100,
  -- Optional integrations. They are always probed with pcall; these flags let
  -- a user (or a test) force the dependency-free fallback.
  nui = true,
  telescope = true,
  -- Reconnect policy for dropped event streams.
  reconnect = {
    initial_ms = 250,
    max_ms = 5000,
    multiplier = 2,
  },
  -- Tap defaults passed to tap.start.
  tap = {
    max_events = 500,
    ttl_secs = 300,
  },
  -- HTTP fallback executable. curl is the only supported transport because it
  -- is present on every supported platform.
  curl = "curl",
}

local function env_value(name)
  local value = vim.env[name]
  if value == nil or value == "" then
    return nil
  end
  return value
end

local function env_defaults()
  local overrides = {}
  local socket = env_value("KNECT_SOCKET_PATH")
  if socket then
    overrides.socket_path = socket
  end
  local http_addr = env_value("KNECT_HTTP_ADDR")
  if http_addr then
    overrides.http_addr = http_addr
  end
  local transport = env_value("KNECT_TRANSPORT")
  if transport then
    overrides.transport = transport
  end
  return overrides
end

local current = vim.tbl_deep_extend("force", vim.deepcopy(defaults), env_defaults())

local function validate(config)
  if type(config.socket_path) ~= "string" or config.socket_path == "" then
    error("knect: socket_path must be a non-empty string")
  end
  if config.http_addr ~= nil and type(config.http_addr) ~= "string" then
    error("knect: http_addr must be a string or nil")
  end
  if not vim.tbl_contains({ "auto", "uds", "http" }, config.transport) then
    error("knect: transport must be one of auto, uds, http")
  end
  if type(config.timeout_ms) ~= "number" or config.timeout_ms <= 0 then
    error("knect: timeout_ms must be a positive number")
  end
  if type(config.debounce_ms) ~= "number" or config.debounce_ms < 0 then
    error("knect: debounce_ms must be a non-negative number")
  end
  if config.reconnect.initial_ms <= 0 or config.reconnect.max_ms < config.reconnect.initial_ms then
    error("knect: reconnect.initial_ms must be positive and <= reconnect.max_ms")
  end
  if config.reconnect.multiplier < 1 then
    error("knect: reconnect.multiplier must be >= 1")
  end
end

--- Return a fresh copy of the built-in defaults.
---@return table
function M.defaults()
  return vim.deepcopy(defaults)
end

--- Return the active configuration table.
---@return table
function M.get()
  return current
end

--- Merge options over the defaults and validate them.
---@param opts table|nil
---@return table
function M.setup(opts)
  current = vim.tbl_deep_extend(
    "force",
    vim.deepcopy(defaults),
    env_defaults(),
    opts or {}
  )
  validate(current)
  return current
end

--- Restore defaults (used by the headless test suite).
function M.reset()
  current = vim.tbl_deep_extend("force", vim.deepcopy(defaults), env_defaults())
end

--- Return the configured HTTP base URL without a trailing slash, or nil.
---@return string|nil
function M.http_url()
  local addr = current.http_addr
  if addr == nil or addr == "" then
    return nil
  end
  if not addr:match("^https?://") then
    addr = "http://" .. addr
  end
  return (addr:gsub("/+$", ""))
end

return M
