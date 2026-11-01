-- Namespace-scoped diagnostics for Knative YAML buffers.
--
-- The daemon returns policy findings and storage conflicts as structured
-- records. This module turns them into `vim.diagnostic` entries anchored to the
-- closest manifest line the focused scanner found. Diagnostics are always
-- namespaced so the plugin can clear its own entries without disturbing another
-- source.

local M = {}

local ns = vim.api.nvim_create_namespace("knect.diagnostics")

local SEV = vim.diagnostic.severity

local severity_by_name = {
  critical = SEV.ERROR,
  high = SEV.ERROR,
  medium = SEV.WARN,
  low = SEV.INFO,
  info = SEV.INFO,
  unknown = SEV.INFO,
}

local severity_by_result = {
  fail = SEV.ERROR,
  error = SEV.ERROR,
  warn = SEV.WARN,
  pass = SEV.INFO,
  skip = SEV.INFO,
}

local FIELD_ORDER = { "image", "envFrom", "secretRef", "volumeMounts" }

local function is_table(value)
  return type(value) == "table"
end

local function as_list(value)
  if not is_table(value) then
    return {}
  end
  return value
end

local function text(value)
  if value == nil or value == vim.NIL then
    return nil
  end
  if type(value) == "string" then
    return value
  end
  return tostring(value)
end

local function first_field_line(scan)
  if is_table(scan) and is_table(scan.fields) then
    for _, field in ipairs(FIELD_ORDER) do
      local entries = scan.fields[field]
      if is_table(entries) and #entries > 0 then
        return entries[1].line or 1
      end
    end
  end
  return 1
end

local function field_line(scan, field)
  if is_table(scan) and is_table(scan.fields) then
    local entries = scan.fields[field]
    if is_table(entries) and #entries > 0 then
      return entries[1].line or 1
    end
  end
  return nil
end

local function line_for_path(path, scan)
  local value = text(path)
  if value == nil then
    return first_field_line(scan)
  end
  value = value:lower()
  if value:find("volumemount", 1, true) or value:find("volume", 1, true) then
    return field_line(scan, "volumeMounts") or first_field_line(scan)
  end
  if value:find("secret", 1, true) then
    return field_line(scan, "secretRef") or first_field_line(scan)
  end
  if value:find("envfrom", 1, true) or value:find("/env", 1, true) then
    return field_line(scan, "envFrom") or first_field_line(scan)
  end
  if value:find("image", 1, true) then
    return field_line(scan, "image") or first_field_line(scan)
  end
  return first_field_line(scan)
end

local function policy_label(record)
  local kind = text(record.kind)
  if kind ~= nil and kind ~= "" and kind ~= "other" then
    return kind
  end
  local name = text(record.name)
  if name ~= nil and name ~= "" then
    return name
  end
  return "policy"
end

local function policy_message(record)
  local label = policy_label(record)
  local message = text(record.message) or "policy finding"
  local prefix = label .. ": "
  if message:sub(1, #prefix):lower() == prefix:lower() then
    return message
  end
  return prefix .. message
end

local function policy_severity(record)
  local result = text(record.result)
  if result ~= nil and severity_by_result[result] ~= nil then
    return severity_by_result[result]
  end
  local severity = text(record.severity)
  return severity_by_name[severity] or SEV.INFO
end

local function make_diagnostic(opts)
  local line = opts.line or 1
  if line < 1 then
    line = 1
  end
  local lnum = line - 1
  return {
    lnum = lnum,
    col = 0,
    end_lnum = lnum,
    end_col = 0,
    severity = opts.severity or SEV.INFO,
    message = opts.message or "knect finding",
    source = "knect",
    code = opts.code,
    user_data = opts.user_data,
  }
end

local function policy_diagnostic(record, scan)
  local details = is_table(record.details) and record.details or {}
  local path = details.path or details.field or details.field_path
  return make_diagnostic({
    line = line_for_path(path, scan),
    severity = policy_severity(record),
    message = policy_message(record),
    code = text(record.name) or text(record.id) or "policy",
    user_data = {
      kind = "policy",
      policy_kind = text(record.kind),
      policy_name = text(record.name),
    },
  })
end

local function warning_diagnostic(warning, scan)
  local code = text(warning.code) or "warning"
  local message = text(warning.message) or "knect warning"
  local lower_code = code:lower()
  local lower_message = message:lower()
  local storage_related = lower_code:find("rwo", 1, true) ~= nil
    or lower_code:find("storage", 1, true) ~= nil
    or lower_message:find("readwriteonce", 1, true) ~= nil
    or lower_message:find("read_write_once", 1, true) ~= nil
  local line
  if storage_related then
    line = field_line(scan, "volumeMounts") or first_field_line(scan)
    if lower_message:find("rwo", 1, true) == nil then
      message = "RWO " .. message
    end
  else
    line = field_line(scan, "image") or first_field_line(scan)
  end
  return make_diagnostic({
    line = line,
    severity = severity_by_name[text(warning.severity)] or SEV.INFO,
    message = message,
    code = code,
    user_data = { kind = storage_related and "storage" or "warning" },
  })
end

local RWO_MODES = {
  read_write_once = true,
  read_write_once_pod = true,
}

local function has_rwo(record)
  for _, mode in ipairs(as_list(record.access_modes)) do
    if RWO_MODES[text(mode)] then
      return true
    end
  end
  return false
end

local function pvc_name(record, conflict)
  local pvc = record.pvc
  local name = is_table(pvc) and text(pvc.name) or nil
  if name == nil and is_table(conflict) and is_table(conflict.pvc) then
    name = text(conflict.pvc.name)
  end
  return name or "PVC"
end

local function storage_diagnostic(record, warning, scan)
  local conflict = is_table(warning.conflict) and warning.conflict or nil
  local name = pvc_name(record, conflict)
  local reason = text(warning.message)
  if (reason == nil or reason == "") and conflict ~= nil then
    reason = text(conflict.reason)
  end
  local max_scale = conflict and tonumber(conflict.max_scale) or nil
  local message
  if reason ~= nil and reason ~= "" then
    message = string.format("RWO %s: %s", name, reason)
  elseif max_scale ~= nil then
    message = string.format("RWO %s: mounted by %d replicas", name, max_scale)
  else
    message = string.format("RWO %s: multiple concurrent consumers", name)
  end
  return make_diagnostic({
    line = field_line(scan, "volumeMounts") or first_field_line(scan),
    severity = severity_by_name[text(warning.severity)] or SEV.WARN,
    message = message,
    code = text(warning.code) or "rwo_conflict",
    user_data = { kind = "storage", pvc = name },
  })
end

local function storage_diagnostics(details, scan)
  local items = {}
  for _, record in ipairs(as_list(details.storage)) do
    if is_table(record) then
      local warnings = as_list(record.warnings)
      local emitted = false
      for _, warning in ipairs(warnings) do
        if is_table(warning) then
          local conflict = is_table(warning.conflict) and warning.conflict or nil
          if conflict ~= nil or (has_rwo(record) and #as_list(record.mounted_by) > 1) then
            items[#items + 1] = storage_diagnostic(record, warning, scan)
            emitted = true
          end
        end
      end
      if not emitted and has_rwo(record) and #as_list(record.mounted_by) > 1 then
        items[#items + 1] = storage_diagnostic(record, {}, scan)
      end
    end
  end
  return items
end

--- Diagnostics derived from a `ValidationResult`.
---@param result table|nil
---@param scan table|nil result from `knect.yaml.scan`
---@return table[]
function M.from_validation(result, scan)
  local items = {}
  if not is_table(result) then
    return items
  end
  for _, record in ipairs(as_list(result.violations)) do
    if is_table(record) then
      items[#items + 1] = policy_diagnostic(record, scan)
    end
  end
  for _, warning in ipairs(as_list(result.warnings)) do
    if is_table(warning) then
      items[#items + 1] = warning_diagnostic(warning, scan)
    end
  end
  return items
end

--- Diagnostics derived from `NodeDetails` policies, warnings, and storage.
---@param details table|nil
---@param scan table|nil result from `knect.yaml.scan`
---@return table[]
function M.from_details(details, scan)
  local items = {}
  if not is_table(details) then
    return items
  end
  for _, record in ipairs(as_list(details.policies)) do
    if is_table(record) then
      local result = text(record.result)
      if result == "fail" or result == "error" or result == "warn" then
        items[#items + 1] = policy_diagnostic(record, scan)
      end
    end
  end
  for _, warning in ipairs(as_list(details.warnings)) do
    if is_table(warning) then
      items[#items + 1] = warning_diagnostic(warning, scan)
    end
  end
  vim.list_extend(items, storage_diagnostics(details, scan))
  return items
end

--- Merge diagnostic lists, dropping duplicates and sorting by line.
---@param lists table[]
---@return table[]
function M.merge(lists)
  local seen = {}
  local merged = {}
  for _, list in ipairs(lists or {}) do
    for _, item in ipairs(as_list(list)) do
      if is_table(item) then
        local key = table.concat({
          tostring(item.lnum or 0),
          tostring(item.code or ""),
          tostring(item.message or ""),
          tostring(item.severity or 0),
        }, "|")
        if not seen[key] then
          seen[key] = true
          merged[#merged + 1] = item
        end
      end
    end
  end
  table.sort(merged, function(left, right)
    local ll = left.lnum or 0
    local rl = right.lnum or 0
    if ll ~= rl then
      return ll < rl
    end
    return tostring(left.message or "") < tostring(right.message or "")
  end)
  return merged
end

--- Publish diagnostics for one buffer under the knect namespace.
---@param bufnr integer
---@param items table[]
function M.publish(bufnr, items)
  if not vim.api.nvim_buf_is_valid(bufnr) then
    return
  end
  local line_count = vim.api.nvim_buf_line_count(bufnr)
  local max_line = math.max(0, line_count - 1)
  local lines = vim.api.nvim_buf_get_lines(bufnr, 0, -1, false)
  local cleaned = {}
  for _, item in ipairs(items or {}) do
    if is_table(item) then
      local lnum = tonumber(item.lnum) or 0
      lnum = math.max(0, math.min(max_line, math.floor(lnum)))
      local end_lnum = tonumber(item.end_lnum) or lnum
      end_lnum = math.max(lnum, math.min(max_line, math.floor(end_lnum)))
      local entry = vim.deepcopy(item)
      entry.lnum = lnum
      entry.end_lnum = end_lnum
      local line_length = #(lines[lnum + 1] or "")
      if entry.end_col == nil or (end_lnum == lnum and entry.end_col <= (entry.col or 0)) then
        entry.end_col = line_length
      end
      cleaned[#cleaned + 1] = entry
    end
  end
  vim.diagnostic.set(ns, bufnr, cleaned)
end

--- Clear every diagnostic published for a buffer.
---@param bufnr integer
function M.clear(bufnr)
  if bufnr == nil or not vim.api.nvim_buf_is_valid(bufnr) then
    return
  end
  vim.diagnostic.reset(ns, bufnr)
end

--- Clear diagnostics for every buffer in the knect namespace.
function M.clear_all()
  vim.diagnostic.reset(ns)
end

--- Namespace used by this module (also handy in tests).
---@return integer
function M.namespace()
  return ns
end

return M
