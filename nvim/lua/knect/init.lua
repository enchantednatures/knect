-- Public entrypoint. Commands are registered by nvim/plugin/knect.lua and are
-- re-registered idempotently when users call setup() manually.

local config = require("knect.config")

local M = {}

local commands_registered = false

local function define(name, callback, opts)
  if vim.fn.exists(":" .. name) == 2 then
    pcall(vim.api.nvim_del_user_command, name)
  end
  vim.api.nvim_create_user_command(name, callback, opts)
end

--- Register the :Knect command family. Safe to call more than once.
function M.ensure_commands()
  if commands_registered then
    return
  end
  commands_registered = true

  define("Knect", function()
    require("knect.topology").open()
  end, { desc = "Open the knect topology explorer" })

  define("KnectServices", function()
    require("knect.pickers").open("services")
  end, { desc = "List Knative Services" })

  define("KnectBrokers", function()
    require("knect.pickers").open("brokers")
  end, { desc = "List Knative Brokers" })

  define("KnectViolations", function()
    require("knect.pickers").open("violations")
  end, { desc = "List topology warnings and policy violations" })

  define("KnectDiff", function()
    require("knect.diff").open()
  end, { desc = "Three-way diff for the current Kubernetes YAML buffer" })

  define("KnectTap", function(args)
    require("knect.tap").open_command(args.fargs)
  end, {
    nargs = "*",
    desc = "Stream CloudEvents from a Broker or Trigger",
    complete = function()
      return { "broker", "trigger" }
    end,
  })

  define("KnectTapStop", function()
    require("knect.tap").stop()
  end, { desc = "Stop the active knect tap" })
end

--- Configure the plugin.
---@param opts table|nil
---@return table
function M.setup(opts)
  config.setup(opts)
  M.ensure_commands()
  return M
end

--- Open the topology explorer (same as :Knect).
---@param opts table|nil
function M.open(opts)
  return require("knect.topology").open(opts)
end

--- Open one of the pickers (services, brokers, violations).
---@param kind string
---@param opts table|nil
function M.pick(kind, opts)
  return require("knect.pickers").open(kind, opts)
end

--- Open the three-way diff.
---@param opts table|nil
function M.diff(opts)
  return require("knect.diff").open(opts)
end

--- Open a live tap.
---@param target table|nil
function M.tap(target)
  return require("knect.tap").open(target)
end

return M
