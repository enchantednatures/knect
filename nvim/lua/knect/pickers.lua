-- Telescope pickers with a vim.ui.select fallback.

local config = require("knect.config")
local rpc = require("knect.rpc")
local topology = require("knect.topology")
local util = require("knect.util")
local telescope = require("knect.ui.telescope")

local M = {}

local SEVERITY_RANK = {
  critical = 1,
  high = 2,
  medium = 3,
  low = 4,
  info = 5,
  unknown = 6,
}

local function severity_rank(value)
  return SEVERITY_RANK[tostring(value or "unknown")] or 6
end

local function node_label(node)
  return string.format(
    "[%s] %s <%s> - %s",
    tostring(node.health or "unknown"),
    util.node_label(node),
    tostring(node.id and node.id.kind or node.category or "?"),
    tostring(node.summary or "")
  )
end

local function open_details(lines, title)
  local bufnr = util.scratch_buffer("knect://details/" .. tostring(title), {
    filetype = "knect-details",
    bufhidden = "wipe",
  })
  util.set_buffer_lines(bufnr, lines)
  local columns = math.max(20, vim.o.columns)
  local screen_lines = math.max(8, vim.o.lines - vim.o.cmdheight)
  local width = math.max(30, math.min(columns - 4, math.floor(columns * 0.72)))
  local height = math.max(8, math.min(screen_lines - 4, math.floor(screen_lines * 0.7)))
  local winid = vim.api.nvim_open_win(bufnr, true, {
    relative = "editor",
    row = math.max(1, math.floor((screen_lines - height) / 2) - 1),
    col = math.max(1, math.floor((columns - width) / 2)),
    width = width,
    height = height,
    style = "minimal",
    border = "rounded",
    title = " " .. tostring(title) .. " ",
    title_pos = "center",
  })
  local function close()
    util.close_window(winid)
    util.delete_buffer(bufnr)
  end
  vim.keymap.set("n", "q", close, { buffer = bufnr, nowait = true, silent = true })
  vim.keymap.set("n", "<Esc>", close, { buffer = bufnr, nowait = true, silent = true })
  vim.api.nvim_create_autocmd("BufWipeout", {
    buffer = bufnr,
    once = true,
    callback = function()
      util.close_window(winid)
    end,
  })
end

local function show_entry(entry)
  if entry.node ~= nil then
    rpc.request("graph.get_node_details", {
      id = entry.node.id,
      include_yaml = true,
    }, function(err, details)
      if err ~= nil then
        open_details({
          util.resource_label(entry.node.id),
          "",
          "error: " .. util.format_error(err),
        }, "Knect: " .. util.node_label(entry.node))
        return
      end
      open_details(topology.details_lines(details), "Knect: " .. util.node_label(entry.node))
    end)
    return
  end
  if entry.message ~= nil then
    util.notify(entry.message)
  end
end

local function present(title, entries, on_select)
  if #entries == 0 then
    util.notify("nothing to show in " .. title)
    return
  end
  if config.get().telescope and telescope.available() then
    local ok, opened = pcall(telescope.pick, {
      title = title,
      entries = entries,
      on_select = on_select or show_entry,
    })
    if ok and opened then
      return
    end
  end
  local labels = {}
  for index, entry in ipairs(entries) do
    labels[index] = entry.label
  end
  vim.ui.select(labels, { prompt = title, kind = "knect" }, function(_, index)
    local entry = entries[index]
    if entry ~= nil then
      local select = on_select or show_entry
      select(entry)
    end
  end)
end

local function entries_from_topology(kind, graph)
  local entries = {}
  for _, node in ipairs(graph.nodes or {}) do
    local include = false
    if kind == "services" then
      include = node.category == "k_service"
    elseif kind == "brokers" then
      include = node.category == "broker"
    end
    if include then
      entries[#entries + 1] = {
        label = node_label(node),
        ordinal = util.node_label(node) .. " " .. tostring(node.category),
        node = node,
      }
    end
  end
  table.sort(entries, function(left, right)
    return left.label < right.label
  end)
  return entries
end

local function collect_violations(graph, callback)
  local entries = {}
  for _, warning in ipairs(graph.warnings or {}) do
    entries[#entries + 1] = {
      label = string.format(
        "[%s] warning %s: %s",
        tostring(warning.severity or "info"),
        tostring(warning.code or "warning"),
        tostring(warning.message or "")
      ),
      severity = warning.severity or "info",
      message = warning.message,
    }
  end

  local nodes = graph.nodes or {}
  local limit = math.min(#nodes, 200)
  if limit == 0 then
    callback(entries)
    return
  end
  local remaining = limit
  for index = 1, limit do
    local node = nodes[index]
    rpc.request("graph.get_node_details", { id = node.id }, function(err, details)
      if err == nil then
        for _, policy in ipairs(details.policies or {}) do
          local result = tostring(policy.result or "unknown")
          if result == "fail" or result == "warn" or result == "error" then
            entries[#entries + 1] = {
              label = string.format(
                "[%s] %s %s/%s - %s: %s",
                tostring(policy.severity or "info"),
                result,
                tostring(policy.kind or "policy"),
                tostring(policy.name or "?"),
                util.node_label(node),
                tostring(policy.message or "")
              ),
              severity = policy.severity or "info",
              node = node,
              policy = policy,
            }
          end
        end
        for _, warning in ipairs(details.warnings or {}) do
          entries[#entries + 1] = {
            label = string.format(
              "[%s] warning %s - %s: %s",
              tostring(warning.severity or "info"),
              tostring(warning.code or "warning"),
              util.node_label(node),
              tostring(warning.message or "")
            ),
            severity = warning.severity or "info",
            node = node,
            message = warning.message,
          }
        end
      end
      remaining = remaining - 1
      if remaining == 0 then
        table.sort(entries, function(left, right)
          local left_rank = severity_rank(left.severity)
          local right_rank = severity_rank(right.severity)
          if left_rank ~= right_rank then
            return left_rank < right_rank
          end
          return left.label < right.label
        end)
        callback(entries)
      end
    end)
  end
end

--- Load picker entries for one command.
---@param kind string "services"|"brokers"|"violations"
---@param callback fun(entries:table[])
function M.load(kind, callback)
  rpc.request("graph.get_topology", {}, function(err, graph)
    if err ~= nil then
      util.notify("cannot load topology: " .. util.format_error(err), vim.log.levels.ERROR)
      callback({})
      return
    end
    if kind == "violations" then
      collect_violations(graph, callback)
      return
    end
    callback(entries_from_topology(kind, graph))
  end)
end

--- Open a services, brokers, or violations picker.
---@param kind string "services"|"brokers"|"violations"
---@param opts table|nil { on_select?: fun(entry:table) }
function M.open(kind, opts)
  opts = opts or {}
  local titles = {
    services = "Knect services",
    brokers = "Knect brokers",
    violations = "Knect violations",
  }
  local title = titles[kind] or ("Knect " .. tostring(kind))
  M.load(kind, function(entries)
    present(title, entries, opts.on_select)
  end)
end

--- Pick a Broker or Trigger and return `{ kind, namespace, name }`.
---@param callback fun(target:table|nil)
function M.select_target(callback)
  rpc.request("graph.get_topology", {}, function(err, graph)
    if err ~= nil then
      util.notify("cannot load topology: " .. util.format_error(err), vim.log.levels.ERROR)
      callback(nil)
      return
    end
    local entries = {}
    for _, node in ipairs(graph.nodes or {}) do
      if node.category == "broker" or node.category == "trigger" then
        entries[#entries + 1] = {
          label = string.format(
            "[%s] %s <%s> - %s",
            tostring(node.health or "unknown"),
            util.node_label(node),
            tostring(node.category or "?"),
            tostring(node.summary or "")
          ),
          target = {
            kind = node.category,
            namespace = node.namespace or "default",
            name = node.name,
          },
        }
      end
    end
    table.sort(entries, function(left, right)
      return left.label < right.label
    end)
    present("Tap target", entries, function(entry)
      callback(entry.target)
    end)
  end)
end

M._show_entry = show_entry

return M
