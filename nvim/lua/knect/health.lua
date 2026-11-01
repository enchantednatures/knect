-- `:checkhealth knect` provider.

local config = require("knect.config")

local M = {}

local function start(name)
  if type(vim.health.start) == "function" then
    vim.health.start(name)
  else
    vim.health.report_start(name)
  end
end

local function ok(message)
  if type(vim.health.ok) == "function" then
    vim.health.ok(message)
  else
    vim.health.report_ok(message)
  end
end

local function warn(message)
  if type(vim.health.warn) == "function" then
    vim.health.warn(message)
  else
    vim.health.report_warn(message)
  end
end

local function error_message(message)
  if type(vim.health.error) == "function" then
    vim.health.error(message)
  else
    vim.health.report_error(message)
  end
end

local function info(message)
  if type(vim.health.info) == "function" then
    vim.health.info(message)
  else
    vim.health.report_info(message)
  end
end

local function plugin_available(module)
  local ok_load = pcall(require, module)
  return ok_load
end

function M.check()
  start("knect.nvim")

  if vim.fn.has("nvim-0.10") == 1 then
    ok("Neovim >= 0.10 (" .. tostring(vim.version().major) .. "." .. tostring(vim.version().minor) .. ")")
  else
    error_message("knect.nvim requires Neovim 0.10 or newer")
  end

  if vim.uv ~= nil or vim.loop ~= nil then
    ok("libuv bindings available (vim.uv)")
  else
    error_message("vim.uv is missing; knect cannot open the daemon socket")
  end

  if vim.json ~= nil then
    ok("vim.json available")
  else
    error_message("vim.json is missing; knect cannot encode JSON-RPC")
  end

  if type(vim.system) == "function" then
    ok("vim.system available for the HTTP fallback")
  else
    warn("vim.system is missing; only the UDS transport is available")
  end

  local cfg = config.get()
  info("socket_path: " .. tostring(cfg.socket_path))
  info("transport: " .. tostring(cfg.transport))
  info("timeout_ms: " .. tostring(cfg.timeout_ms))
  if cfg.http_addr ~= nil and cfg.http_addr ~= "" then
    info("http_addr: " .. tostring(cfg.http_addr))
  else
    info("http_addr: (not configured)")
  end

  local uv = vim.uv or vim.loop
  local stat = uv and uv.fs_stat(cfg.socket_path)
  if stat ~= nil then
    ok("daemon socket exists: " .. tostring(cfg.socket_path))
  else
    warn("daemon socket not found: " .. tostring(cfg.socket_path))
  end

  if cfg.http_addr ~= nil and cfg.http_addr ~= "" then
    if vim.fn.executable(cfg.curl) == 1 then
      ok("curl executable found for HTTP fallback: " .. tostring(cfg.curl))
    else
      warn("curl not found; HTTP fallback is disabled")
    end
  end

  if plugin_available("nui.split") then
    ok("Nui.nvim detected (split explorer)")
  else
    info("Nui.nvim not installed; using floating windows")
  end

  if plugin_available("telescope.pickers") then
    ok("Telescope.nvim detected (picker UI)")
  else
    info("Telescope.nvim not installed; using vim.ui.select")
  end

  if vim.fn.exists(":Knect") == 2 then
    ok(":Knect commands registered")
  else
    error_message(":Knect commands are not registered; add nvim/ to runtimepath")
  end
end

return M
