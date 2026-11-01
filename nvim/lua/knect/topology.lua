-- Topology explorer: fetch graph.get_topology, render an ASCII tree/DAG, and
-- show graph.get_node_details in a side pane. The Nui adapter is optional.

local config = require("knect.config")
local rpc = require("knect.rpc")
local util = require("knect.util")
local floating = require("knect.ui.floating")
local nui = require("knect.ui.nui")

local M = {}

local session

local function node_key(id)
  if type(id) ~= "table" then
    return ""
  end
  return table.concat({
    id.api_version or "",
    id.kind or "",
    id.namespace or "",
    id.name or "",
  }, "|")
end

local function compare_nodes(left, right)
  local ln = left.namespace or ""
  local rn = right.namespace or ""
  if ln ~= rn then
    return ln < rn
  end
  if left.category ~= right.category then
    return tostring(left.category) < tostring(right.category)
  end
  return tostring(left.name) < tostring(right.name)
end

local function edge_sort(left, right)
  if left.kind ~= right.kind then
    return tostring(left.kind) < tostring(right.kind)
  end
  return node_key(left.to) < node_key(right.to)
end

local function is_source(node)
  return node.category == "source"
    or node.category == "sink_binding"
    or node.category == "subscription"
    or node.category == "channel"
end

local function groups_for(node)
  if node.category == "k_service" then
    return "services"
  end
  if node.category == "broker" then
    return "brokers"
  end
  if is_source(node) then
    return "sources"
  end
  return "other"
end

local function truncate(text, limit)
  text = tostring(text or "")
  limit = limit or 140
  if #text <= limit then
    return text
  end
  return text:sub(1, limit - 3) .. "..."
end

local function build_lines(topology)
  local lines = {
    string.format(
      "Knect topology  context=%s  generation=%s",
      tostring(topology.context or "?"),
      tostring(topology.generation or "?")
    ),
  }
  local stats = topology.stats or {}
  lines[#lines + 1] = string.format(
    "%d nodes / %d edges / %d warnings",
    stats.node_count or #(topology.nodes or {}),
    stats.edge_count or #(topology.edges or {}),
    stats.warning_count or #(topology.warnings or {})
  )
  lines[#lines + 1] = ""

  local nodes = topology.nodes or {}
  local sorted = vim.deepcopy(nodes)
  table.sort(sorted, compare_nodes)

  local edges_by_from = {}
  for _, edge in ipairs(topology.edges or {}) do
    local key = node_key(edge.from)
    edges_by_from[key] = edges_by_from[key] or {}
    edges_by_from[key][#edges_by_from[key] + 1] = edge
  end
  for _, list in pairs(edges_by_from) do
    table.sort(list, edge_sort)
  end

  local by_key = {}
  for _, node in ipairs(sorted) do
    by_key[node_key(node.id)] = node
  end

  local line_nodes = {}
  local selectable = {}
  local group_titles = {
    services = "Services",
    brokers = "Brokers",
    sources = "Sources",
    other = "Other resources",
  }
  local group_order = { "services", "brokers", "sources", "other" }
  local buckets = {}
  for _, node in ipairs(sorted) do
    local bucket = groups_for(node)
    buckets[bucket] = buckets[bucket] or {}
    buckets[bucket][#buckets[bucket] + 1] = node
  end

  for _, group in ipairs(group_order) do
    local members = buckets[group] or {}
    if #members > 0 then
      lines[#lines + 1] = string.format("%s (%d)", group_titles[group], #members)
      local namespaces = {}
      for _, node in ipairs(members) do
        local namespace = node.namespace or "(cluster)"
        namespaces[namespace] = namespaces[namespace] or {}
        namespaces[namespace][#namespaces[namespace] + 1] = node
      end
      local namespace_names = vim.tbl_keys(namespaces)
      table.sort(namespace_names)
      for _, namespace in ipairs(namespace_names) do
        lines[#lines + 1] = "  " .. namespace
        for _, node in ipairs(namespaces[namespace]) do
          local label = string.format(
            "    |-- [%s] %s <%s> - %s",
            tostring(node.health or "unknown"),
            node.name or "?",
            tostring(node.id and node.id.kind or node.category or "?"),
            truncate(node.summary, 110)
          )
          lines[#lines + 1] = label
          line_nodes[#lines] = node
          selectable[#selectable + 1] = #lines

          local edges = edges_by_from[node_key(node.id)] or {}
          for _, edge in ipairs(edges) do
            local target = by_key[node_key(edge.to)]
            local target_label = target and util.node_label(target) or node_key(edge.to)
            local edge_line = string.format(
              "      |~ %s -> %s",
              tostring(edge.kind or "unknown"),
              target_label
            )
            if edge.label ~= nil then
              edge_line = edge_line .. " (" .. tostring(edge.label) .. ")"
            end
            lines[#lines + 1] = truncate(edge_line, 150)
            line_nodes[#lines] = target or node
            selectable[#selectable + 1] = #lines
          end
        end
      end
      lines[#lines + 1] = ""
    end
  end

  if #nodes == 0 then
    lines[#lines + 1] = "(no resources discovered)"
    lines[#lines + 1] = ""
  end

  local warnings = topology.warnings or {}
  if #warnings > 0 then
    lines[#lines + 1] = string.format("Warnings (%d)", #warnings)
    for _, warning in ipairs(warnings) do
      lines[#lines + 1] = string.format(
        "  ! [%s] %s: %s",
        tostring(warning.severity or "info"),
        tostring(warning.code or "warning"),
        truncate(warning.message, 120)
      )
    end
  end

  return lines, line_nodes, selectable
end

local function policy_lines(policies)
  local lines = {}
  for _, policy in ipairs(policies or {}) do
    lines[#lines + 1] = string.format(
      "  [%s] %s %s/%s: %s",
      tostring(policy.severity or "info"),
      tostring(policy.result or "?"),
      tostring(policy.kind or "policy"),
      tostring(policy.name or "?"),
      truncate(policy.message, 120)
    )
    if policy.details and policy.details.remediation then
      lines[#lines + 1] = "        remediation: " .. tostring(policy.details.remediation)
    end
  end
  return lines
end

local function mutation_lines(mutations)
  local lines = {}
  for _, mutation in ipairs(mutations or {}) do
    lines[#lines + 1] = string.format(
      "  %s (%s) %s: %d field(s)",
      tostring(mutation.manager or "?"),
      tostring(mutation.manager_kind or "?"),
      tostring(mutation.operation or "?"),
      #(mutation.paths or {})
    )
    for _, path in ipairs(mutation.paths or {}) do
      lines[#lines + 1] = string.format(
        "      %s%s%s",
        tostring(path.path or "?"),
        path.value_preview and (" = " .. tostring(path.value_preview)) or "",
        path.yaml_line and (" (line " .. tostring(path.yaml_line) .. ")") or ""
      )
    end
  end
  return lines
end

local function link_lines(links)
  local lines = {}
  for _, link in ipairs(links or {}) do
    lines[#lines + 1] = string.format(
      "  %s -> %s via %s (confidence %.2f)",
      util.resource_label(link.source),
      util.resource_label(link.target),
      tostring(link.via or "?"),
      tonumber(link.confidence) or 0
    )
  end
  return lines
end

local function storage_lines(storage)
  local lines = {}
  for _, record in ipairs(storage or {}) do
    lines[#lines + 1] = string.format(
      "  %s [%s] %s %s",
      util.resource_label(record.pvc),
      table.concat(record.access_modes or {}, ","),
      tostring(record.phase or "?"),
      tostring(record.capacity or "?")
    )
    for _, warning in ipairs(record.warnings or {}) do
      lines[#lines + 1] = string.format(
        "      ! [%s] %s: %s",
        tostring(warning.severity or "info"),
        tostring(warning.code or "warning"),
        truncate(warning.message, 120)
      )
    end
  end
  return lines
end

--- Render NodeDetails as readable text. Shared with the picker details view.
---@param details table
---@return string[]
function M.details_lines(details)
  if type(details) ~= "table" then
    return { "(no details)" }
  end
  local lines = {}
  lines[#lines + 1] = string.format(
    "%s <%s>  [%s]",
    util.resource_label(details.id),
    tostring(details.category or "?"),
    tostring(details.health or "unknown")
  )
  lines[#lines + 1] = "Summary: " .. tostring(details.summary or "")
  if details.id then
    lines[#lines + 1] = "ID: " .. tostring(details.id.api_version) .. "/" .. tostring(details.id.kind)
  end
  lines[#lines + 1] = ""

  local policies = details.policies or {}
  if #policies > 0 then
    lines[#lines + 1] = string.format("Policies (%d)", #policies)
    vim.list_extend(lines, policy_lines(policies))
    lines[#lines + 1] = ""
  end

  local mutations = details.mutations or {}
  if #mutations > 0 then
    lines[#lines + 1] = string.format("Managed fields (%d)", #mutations)
    vim.list_extend(lines, mutation_lines(mutations))
    lines[#lines + 1] = ""
  end

  local links = details.links or {}
  if #links > 0 then
    lines[#lines + 1] = string.format("Links (%d)", #links)
    vim.list_extend(lines, link_lines(links))
    lines[#lines + 1] = ""
  end

  local storage = details.storage or {}
  if #storage > 0 then
    lines[#lines + 1] = string.format("Storage (%d)", #storage)
    vim.list_extend(lines, storage_lines(storage))
    lines[#lines + 1] = ""
  end

  local warnings = details.warnings or {}
  if #warnings > 0 then
    lines[#lines + 1] = string.format("Warnings (%d)", #warnings)
    for _, warning in ipairs(warnings) do
      lines[#lines + 1] = string.format(
        "  ! [%s] %s: %s",
        tostring(warning.severity or "info"),
        tostring(warning.code or "warning"),
        truncate(warning.message, 120)
      )
    end
    lines[#lines + 1] = ""
  end

  local metrics = details.metrics
  if metrics ~= nil and metrics ~= vim.NIL then
    lines[#lines + 1] = "Metrics"
    lines[#lines + 1] = "  " .. util.pretty_json(metrics)
    lines[#lines + 1] = ""
  end

  if details.yaml ~= nil and details.yaml ~= vim.NIL then
    lines[#lines + 1] = "YAML (redacted)"
    local yaml_lines = util.split_lines(details.yaml)
    for index = 1, math.min(#yaml_lines, 300) do
      lines[#lines + 1] = "  " .. yaml_lines[index]
    end
    if #yaml_lines > 300 then
      lines[#lines + 1] = "  ... (truncated)"
    end
  end

  if #lines == 0 then
    lines[#lines + 1] = "(no details)"
  end
  return lines
end

local function set_error(s, message)
  s.error = message
  s.line_nodes = {}
  s.selectable = {}
  s.adapter:set_content({
    "Knect topology",
    "",
    "error: " .. message,
    "",
    "r: retry    q: close",
  })
  s.adapter:set_details({ "Cannot load topology.", "", message })
end

local function inspect_node(s, node)
  if s == nil or s.closed or node == nil then
    return
  end
  local captured = s
  captured.inspect_seq = (captured.inspect_seq or 0) + 1
  local sequence = captured.inspect_seq
  rpc.request("graph.get_node_details", {
    id = node.id,
    include_yaml = true,
  }, function(err, details)
    if session ~= captured or captured.closed or captured.inspect_seq ~= sequence then
      return
    end
    if err ~= nil then
      captured.adapter:set_details({
        util.resource_label(node.id),
        "",
        "error: " .. util.format_error(err),
      })
      return
    end
    captured.adapter:set_details(M.details_lines(details))
  end)
end

local function render(s)
  local topology = s.topology
  local lines, line_nodes, selectable = build_lines(topology)
  s.lines = lines
  s.line_nodes = line_nodes
  s.selectable = selectable

  local desired = s.selectable[1]
  if s.selected_node_key ~= nil then
    for line, node in pairs(line_nodes) do
      if node_key(node.id) == s.selected_node_key then
        desired = line
        break
      end
    end
  end
  s.selected = desired or 1
  s.adapter:set_content(lines)
  s.adapter:set_cursor(s.selected)
  local selected_node = line_nodes[s.selected]
  if selected_node ~= nil then
    s.selected_node_key = node_key(selected_node.id)
    inspect_node(s, selected_node)
  else
    s.adapter:set_details({ "No nodes in the current topology." })
  end
end

--- Fetch the topology and re-render.
function M.refresh()
  local s = session
  if s == nil or s.closed then
    return
  end
  s.adapter:set_details({ "Loading topology..." })
  rpc.request("graph.get_topology", {}, function(err, topology)
    if session ~= s or s.closed then
      return
    end
    if err ~= nil then
      set_error(s, util.format_error(err))
      util.notify("topology refresh failed: " .. util.format_error(err), vim.log.levels.WARN)
      return
    end
    s.error = nil
    s.topology = topology
    render(s)
  end)
end

local function move_selection(delta)
  local s = session
  if s == nil or s.closed or #s.selectable == 0 then
    return
  end
  local position = 1
  for index, line in ipairs(s.selectable) do
    if line == s.selected then
      position = index
      break
    end
  end
  position = ((position - 1 + delta) % #s.selectable) + 1
  s.selected = s.selectable[position]
  s.adapter:set_cursor(s.selected)
  local node = s.line_nodes[s.selected]
  if node ~= nil then
    s.selected_node_key = node_key(node.id)
    inspect_node(s, node)
  end
end

--- Move to the next selectable node/edge.
function M.select_next()
  move_selection(1)
end

--- Move to the previous selectable node/edge.
function M.select_prev()
  move_selection(-1)
end

--- Inspect the currently selected node.
function M.inspect_selected()
  local s = session
  if s == nil or s.closed then
    return
  end
  local node = s.line_nodes[s.selected]
  if node ~= nil then
    inspect_node(s, node)
  end
end

--- Close the explorer.
function M.close()
  local s = session
  if s == nil then
    return
  end
  session = nil
  s.closed = true
  s.adapter:close()
end

--- Open the topology explorer.
---@param opts table|nil { ui?: "nui"|"floating", select_id?: table }
---@return table|nil session
function M.open(opts)
  opts = opts or {}
  if session ~= nil then
    M.close()
  end

  local s = {
    opts = opts,
    closed = false,
    topology = nil,
    lines = {},
    line_nodes = {},
    selectable = {},
    selected = 1,
    error = nil,
  }
  session = s

  local callbacks = {
    on_next = M.select_next,
    on_prev = M.select_prev,
    on_inspect = M.inspect_selected,
    on_refresh = M.refresh,
    on_close = M.close,
  }

  local adapter
  local cfg = config.get()
  local use_nui = opts.ui == "nui" or (opts.ui == nil and cfg.nui)
  if use_nui and nui.available() then
    local ok, controller, err = pcall(nui.new, callbacks)
    if ok and controller ~= nil then
      adapter = controller
    else
      util.notify("Nui topology unavailable, using floating windows: " .. tostring(err or controller), vim.log.levels.DEBUG)
    end
  end
  if adapter == nil then
    adapter = floating.new(callbacks)
  end
  s.adapter = adapter

  if opts.select_id ~= nil then
    s.selected_node_key = node_key(opts.select_id)
  end

  M.refresh()
  return s
end

--- Introspection used by the test suite.
---@return table|nil
function M._session()
  return session
end

return M
