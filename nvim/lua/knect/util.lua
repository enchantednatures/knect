-- Small helpers shared by the knect.nvim client and its UI adapters.

local M = {}

local uv = vim.uv or vim.loop

--- Schedule a callback; safe to call from libuv fast-event contexts.
---@param fn function
function M.schedule(fn)
  if vim.in_fast_event() then
    vim.schedule(fn)
  else
    fn()
  end
end

--- Notify with a consistent prefix.
---@param message string
---@param level integer|nil
function M.notify(message, level)
  vim.notify("knect: " .. message, level or vim.log.levels.INFO)
end

--- Render an RPC error table as a human-readable string.
---@param err table|string|nil
---@return string
function M.format_error(err)
  if err == nil then
    return "unknown error"
  end
  if type(err) == "string" then
    return err
  end
  local message = err.message or "rpc error"
  if err.code ~= nil then
    message = string.format("%s (code %s)", message, tostring(err.code))
  end
  local details = err.data and err.data.details
  if details and details ~= "" then
    message = message .. ": " .. details
  end
  return message
end

--- True when a decoded JSON table is a list rather than a map.
---@param value any
---@return boolean
function M.is_list(value)
  if vim.islist then
    return vim.islist(value)
  end
  if type(value) ~= "table" then
    return false
  end
  local count = 0
  for key in pairs(value) do
    if type(key) ~= "number" then
      return false
    end
    count = count + 1
  end
  return count == #value
end

local function encode_scalar(value)
  if value == vim.NIL then
    return "null"
  end
  local kind = type(value)
  if kind == "string" then
    return vim.json.encode(value)
  end
  if kind == "number" then
    if value ~= value or value == math.huge or value == -math.huge then
      return "null"
    end
    return tostring(value)
  end
  if kind == "boolean" then
    return value and "true" or "false"
  end
  return "null"
end

local function encode_value(value, depth, seen)
  local kind = type(value)
  if kind ~= "table" then
    return encode_scalar(value)
  end
  if seen[value] then
    return "null"
  end
  seen[value] = true
  local indent = string.rep("  ", depth)
  local next_indent = string.rep("  ", depth + 1)
  local parts = {}
  if M.is_list(value) then
    for _, item in ipairs(value) do
      parts[#parts + 1] = next_indent .. encode_value(item, depth + 1, seen)
    end
    seen[value] = nil
    if #parts == 0 then
      return "[]"
    end
    return "[\n" .. table.concat(parts, ",\n") .. "\n" .. indent .. "]"
  end
  local keys = {}
  for key in pairs(value) do
    if type(key) == "string" then
      keys[#keys + 1] = key
    end
  end
  table.sort(keys)
  for _, key in ipairs(keys) do
    parts[#parts + 1] = next_indent
      .. vim.json.encode(key)
      .. ": "
      .. encode_value(value[key], depth + 1, seen)
  end
  seen[value] = nil
  if #parts == 0 then
    return "{}"
  end
  return "{\n" .. table.concat(parts, ",\n") .. "\n" .. indent .. "}"
end

--- Pretty-print a JSON-compatible value with deterministic key ordering.
---@param value any
---@return string
function M.pretty_json(value)
  local ok, rendered = pcall(encode_value, value, 0, {})
  if not ok then
    return tostring(value)
  end
  return rendered
end

--- Split text into lines without a trailing empty element.
---@param text string|nil
---@return string[]
function M.split_lines(text)
  if text == nil or text == "" then
    return {}
  end
  local lines = {}
  for line in (text .. "\n"):gmatch("(.-)\n") do
    lines[#lines + 1] = line
  end
  return lines
end

--- Read a buffer (0 means the current buffer) as a list of lines.
---@param bufnr integer|nil
---@return string[]
function M.buffer_lines(bufnr)
  return vim.api.nvim_buf_get_lines(bufnr or 0, 0, -1, false)
end

--- Replace a buffer's contents while temporarily enabling modification.
---@param bufnr integer
---@param lines string[]
function M.set_buffer_lines(bufnr, lines)
  local normalized = {}
  for _, line in ipairs(lines or {}) do
    line = type(line) == "string" and line or tostring(line)
    if line:find("\n", 1, true) then
      for part in (line .. "\n"):gmatch("(.-)\n") do
        normalized[#normalized + 1] = part
      end
    else
      normalized[#normalized + 1] = line
    end
  end
  vim.api.nvim_set_option_value("modifiable", true, { buf = bufnr })
  vim.api.nvim_buf_set_lines(bufnr, 0, -1, false, normalized)
  vim.api.nvim_set_option_value("modifiable", false, { buf = bufnr })
end

--- Create an unlisted scratch buffer with an optional name and filetype.
---@param name string|nil
---@param opts table|nil
---@return integer
function M.scratch_buffer(name, opts)
  opts = opts or {}
  local bufnr = vim.api.nvim_create_buf(false, true)
  if name ~= nil and name ~= "" then
    pcall(vim.api.nvim_buf_set_name, bufnr, name)
  end
  vim.api.nvim_set_option_value("buftype", "nofile", { buf = bufnr })
  vim.api.nvim_set_option_value("bufhidden", opts.bufhidden or "hide", { buf = bufnr })
  vim.api.nvim_set_option_value("swapfile", false, { buf = bufnr })
  if opts.filetype then
    vim.api.nvim_set_option_value("filetype", opts.filetype, { buf = bufnr })
  end
  if opts.lines then
    M.set_buffer_lines(bufnr, opts.lines)
  end
  return bufnr
end

--- Delete a buffer if it is still valid.
---@param bufnr integer|nil
function M.delete_buffer(bufnr)
  if bufnr and vim.api.nvim_buf_is_valid(bufnr) then
    pcall(vim.api.nvim_buf_delete, bufnr, { force = true })
  end
end

--- Close a window if it is still valid and not the last window of the tab.
---@param winid integer|nil
function M.close_window(winid)
  if not winid or not vim.api.nvim_win_is_valid(winid) then
    return
  end
  local tab = vim.api.nvim_win_get_tabpage(winid)
  local wins = vim.api.nvim_tabpage_list_wins(tab)
  if #wins <= 1 then
    return
  end
  pcall(vim.api.nvim_win_close, winid, true)
end

--- Trim one layer of matching single/double quotes.
---@param value string
---@return string
local function trim_quotes(value)
  value = vim.trim(value or "")
  local first = value:sub(1, 1)
  local last = value:sub(-1)
  if (first == '"' and last == '"') or (first == "'" and last == "'") then
    return value:sub(2, -2)
  end
  return value
end

--- Parse the handful of fields needed to identify a Kubernetes manifest.
---
--- This is deliberately not a YAML parser (Task 18 owns YAML awareness); it
--- only reads top-level `apiVersion`/`kind` and `metadata.name`/`namespace`.
---@param text string
---@return table|nil id
---@return string|nil error
function M.parse_manifest(text)
  if type(text) ~= "string" or text == "" then
    return nil, "buffer is empty"
  end
  local api_version, kind, name, namespace
  local in_metadata = false
  local metadata_indent
  for line in (text .. "\n"):gmatch("(.-)\n") do
    local body = line:gsub("%s+#.*$", "")
    if body:match("^%s*$") or body:match("^%s*%-%-%-") then
      -- blank or document separator
    else
      local indent = #(body:match("^(%s*)") or "")
      if indent == 0 then
        local key, value = body:match("^([%w%._%-/]+)%s*:%s*(.*)$")
        in_metadata = false
        metadata_indent = nil
        if key == "apiVersion" then
          api_version = trim_quotes(value)
        elseif key == "kind" then
          kind = trim_quotes(value)
        elseif key == "metadata" then
          in_metadata = true
        end
      elseif in_metadata then
        if metadata_indent == nil then
          metadata_indent = indent
        end
        -- Only direct children of `metadata`; ignore labels/annotations.
        if indent == metadata_indent then
          local key, value = body:match("^%s*([%w%._%-/]+)%s*:%s*(.*)$")
          if key == "name" and name == nil then
            name = trim_quotes(value)
          elseif key == "namespace" and namespace == nil then
            namespace = trim_quotes(value)
          end
        end
      end
    end
  end
  if api_version == nil or api_version == "" then
    return nil, "missing apiVersion"
  end
  if kind == nil or kind == "" then
    return nil, "missing kind"
  end
  if name == nil or name == "" then
    return nil, "missing metadata.name"
  end
  return {
    api_version = api_version,
    kind = kind,
    namespace = (namespace ~= nil and namespace ~= "") and namespace or nil,
    name = name,
  }
end

--- Human-readable identity for a ResourceId.
---@param id table|nil
---@return string
function M.resource_label(id)
  if type(id) ~= "table" then
    return "<unknown>"
  end
  local scope = id.namespace and (id.namespace .. "/") or ""
  return string.format("%s%s (%s)", scope, id.name or "?", id.kind or "?")
end

--- Compact identity used in logs and picker labels.
---@param node table|nil
---@return string
function M.node_label(node)
  if type(node) ~= "table" then
    return "<unknown>"
  end
  local scope = node.namespace and (node.namespace .. "/") or ""
  return scope .. (node.name or "?")
end

--- Decode JSON with a result/error pair instead of raising.
---@param text string
---@return any|nil
---@return string|nil
function M.decode_json(text)
  local ok, value = pcall(vim.json.decode, text)
  if not ok then
    return nil, tostring(value)
  end
  return value
end

--- Encode JSON with a result/error pair instead of raising.
---@param value any
---@return string|nil
---@return string|nil
function M.encode_json(value)
  local ok, encoded = pcall(vim.json.encode, value)
  if not ok then
    return nil, tostring(encoded)
  end
  return encoded
end

--- Simple trailing debounce.
---@param fn function
---@param ms number
---@return function
function M.debounce(fn, ms)
  local timer
  return function(...)
    local args = { ... }
    if timer then
      timer:stop()
      timer:close()
      timer = nil
    end
    timer = uv.new_timer()
    timer:start(
      math.max(0, ms or 0),
      0,
      vim.schedule_wrap(function()
        if timer then
          timer:stop()
          timer:close()
          timer = nil
        end
        fn(unpack(args))
      end)
    )
  end
end

return M
