-- Live YAML hints and pre-flight validation for Knative manifests.
--
-- The module deliberately does not depend on Treesitter or a YAML parser. A
-- focused line scanner identifies the manifest identity and the handful of
-- fields that receive virtual text. Live state comes from
-- `graph.get_node_details`; policy findings come from `policy.validate_manifest`
-- after a debounced buffer change or save.

local config = require("knect.config")
local diagnostics = require("knect.diagnostics")
local rpc = require("knect.rpc")
local util = require("knect.util")

local M = {}

local KNATIVE_GROUPS = {
  ["serving.knative.dev"] = true,
  ["eventing.knative.dev"] = true,
}

local FIELD_KEYS = {
  image = true,
  envFrom = true,
  secretRef = true,
  volumeMounts = true,
}

local FIELD_ORDER = { "image", "envFrom", "secretRef", "volumeMounts" }

local namespace = vim.api.nvim_create_namespace("knect.yaml")
local attached = {}
local highlights_ready = false

local options = {
  auto_attach = true,
  hints = true,
  diagnostics = true,
  debounce_ms = nil,
}

local function is_table(value)
  return type(value) == "table"
end

local function as_list(value)
  if not is_table(value) then
    return {}
  end
  return value
end

local function as_text(value)
  if value == nil or value == vim.NIL then
    return nil
  end
  if type(value) == "string" then
    return value
  end
  return tostring(value)
end

local function truncate(value, limit)
  value = as_text(value) or ""
  limit = limit or 100
  if #value <= limit then
    return value
  end
  return value:sub(1, limit - 3) .. "..."
end

local function ensure_highlights()
  if highlights_ready then
    return
  end
  local definitions = {
    KnectYamlHint = "Comment",
    KnectYamlSeparator = "Comment",
    KnectYamlHealthy = "DiagnosticOk",
    KnectYamlWarning = "DiagnosticWarn",
    KnectYamlError = "DiagnosticError",
    KnectYamlInfo = "DiagnosticInfo",
  }
  for name, link in pairs(definitions) do
    if vim.fn.hlexists(name) == 0 then
      pcall(vim.api.nvim_set_hl, 0, name, { link = link })
    end
  end
  highlights_ready = true
end

local function strip_comment(line)
  local quote
  local escaped = false
  for index = 1, #line do
    local char = line:sub(index, index)
    if quote ~= nil then
      if escaped then
        escaped = false
      elseif char == "\\" then
        escaped = true
      elseif char == quote then
        quote = nil
      end
    elseif char == '"' or char == "'" then
      quote = char
    elseif char == "#" and (index == 1 or line:sub(index - 1, index - 1):match("%s")) then
      return line:sub(1, index - 1)
    end
  end
  return line
end

local function scalar(value)
  value = vim.trim(value or "")
  local first = value:sub(1, 1)
  local last = value:sub(-1)
  if (first == '"' and last == '"') or (first == "'" and last == "'") then
    return value:sub(2, -2)
  end
  return value
end

local function parse_entry(body)
  local spaces = body:match("^(%s*)") or ""
  local indent = #spaces
  local rest = body:sub(indent + 1)
  local key_indent = indent
  local dash = rest:match("^%-%s*")
  local list = dash ~= nil
  if dash ~= nil then
    key_indent = indent + #dash
    rest = rest:sub(#dash + 1)
  end
  local key, value = rest:match("^([%w%._%-/]+)%s*:%s*(.*)$")
  if key == nil then
    return nil
  end
  return {
    key = key,
    value = value,
    indent = indent,
    key_indent = key_indent,
    list = list,
  }
end

--- True when an apiVersion belongs to Knative Serving or Eventing.
---@param api_version string|nil
---@return boolean
function M.is_knative_api(api_version)
  if type(api_version) ~= "string" then
    return false
  end
  local group = api_version:match("^([^/]+)/")
  return group ~= nil and KNATIVE_GROUPS[group] == true
end

local function scan_document(lines, start_line)
  local api_version
  local kind
  local name
  local doc_namespace
  local in_metadata = false
  local metadata_indent
  local first_field_line
  local fields = {}
  for _, field in ipairs(FIELD_ORDER) do
    fields[field] = {}
  end

  for offset, raw in ipairs(lines) do
    local line_no = start_line + offset - 1
    local body = strip_comment(raw)
    if body:match("%S") ~= nil then
      local entry = parse_entry(body)
      local indent = entry and entry.indent or #(body:match("^(%s*)") or "")
      if indent == 0 then
        in_metadata = false
        metadata_indent = nil
        if entry ~= nil then
          if entry.key == "apiVersion" then
            api_version = scalar(entry.value)
          elseif entry.key == "kind" then
            kind = scalar(entry.value)
          elseif entry.key == "metadata" then
            in_metadata = true
          end
        end
      elseif in_metadata then
        if metadata_indent == nil then
          metadata_indent = indent
        end
        if indent == metadata_indent and entry ~= nil then
          if entry.key == "name" and name == nil then
            name = scalar(entry.value)
          elseif entry.key == "namespace" and doc_namespace == nil then
            doc_namespace = scalar(entry.value)
          end
        end
      end

      if entry ~= nil and FIELD_KEYS[entry.key] then
        fields[entry.key][#fields[entry.key] + 1] = {
          line = line_no,
          indent = indent,
          key_indent = entry.key_indent,
          value = scalar(entry.value),
          list = entry.list,
        }
        first_field_line = first_field_line or line_no
      end
    end
  end

  local group = api_version and api_version:match("^([^/]+)/") or nil
  return {
    api_version = api_version,
    group = group,
    kind = kind,
    name = name,
    namespace = (doc_namespace ~= nil and doc_namespace ~= "") and doc_namespace or nil,
    knative = group ~= nil and KNATIVE_GROUPS[group] == true,
    fields = fields,
    first_field_line = first_field_line,
    start_line = start_line,
  }
end

local function split_documents(text)
  local documents = {}
  local current = { start_line = 1, lines = {} }
  local line_no = 0
  for line in (text .. "\n"):gmatch("(.-)\n") do
    line_no = line_no + 1
    if line:match("^%s*%-%-%-%s*$") then
      if #current.lines > 0 then
        documents[#documents + 1] = current
      end
      current = { start_line = line_no + 1, lines = {} }
    else
      current.lines[#current.lines + 1] = line
    end
  end
  if #current.lines > 0 then
    documents[#documents + 1] = current
  end
  return documents
end

--- Scan a buffer body for Knative identity and hint lines.
---
--- Returns the first Knative document when one exists. When the buffer is
--- valid YAML but not Knative, the returned table has `knative = false` so
--- callers can distinguish "ignored" from "unparseable".
---@param text string
---@return table|nil manifest
---@return string|nil error
function M.scan(text)
  if type(text) ~= "string" or text:match("%S") == nil then
    return nil, "buffer is empty"
  end
  local documents = split_documents(text)
  local scanned = {}
  for _, document in ipairs(documents) do
    scanned[#scanned + 1] = scan_document(document.lines, document.start_line)
  end
  for _, candidate in ipairs(scanned) do
    if candidate.knative then
      if candidate.api_version == nil or candidate.kind == nil or candidate.name == nil then
        return nil, "Knative manifest is missing apiVersion, kind, or metadata.name"
      end
      return candidate
    end
  end
  for _, candidate in ipairs(scanned) do
    if candidate.api_version ~= nil or candidate.kind ~= nil then
      return candidate
    end
  end
  return nil, "missing apiVersion"
end

--- Scan a list of lines (convenience wrapper).
---@param lines string[]
---@return table|nil manifest
---@return string|nil error
function M.scan_lines(lines)
  return M.scan(table.concat(lines or {}, "\n"))
end

local function parse_number(value)
  local number = tonumber(scalar(value))
  if number == nil then
    return nil
  end
  return math.floor(number)
end

local function scan_live_status(yaml)
  local live = {}
  if type(yaml) ~= "string" or yaml == "" then
    return live
  end
  local traffic_indent
  local traffic_entries = {}
  local current

  local function finish_entry()
    if current ~= nil then
      traffic_entries[#traffic_entries + 1] = current
      current = nil
    end
  end

  for raw in (yaml .. "\n"):gmatch("(.-)\n") do
    local body = strip_comment(raw)
    if body:match("%S") ~= nil then
      local spaces = body:match("^(%s*)") or ""
      local indent = #spaces
      local rest = body:sub(indent + 1)
      local dash = rest:match("^%-%s*")
      local content = rest
      if dash ~= nil then
        content = rest:sub(#dash + 1)
      end
      local key, value = content:match("^([%w%._%-/]+)%s*:%s*(.*)$")

      if traffic_indent ~= nil then
        local is_list = dash ~= nil
        if indent < traffic_indent or (indent == traffic_indent and not is_list and key ~= "traffic") then
          finish_entry()
          traffic_indent = nil
        end
      end

      if key == "traffic" and scalar(value) == "" then
        finish_entry()
        traffic_indent = indent
        traffic_entries = {}
      elseif traffic_indent ~= nil and key ~= nil then
        if dash ~= nil then
          finish_entry()
          current = {}
        end
        if current ~= nil then
          if key == "percent" then
            current.percent = parse_number(value)
          elseif key == "revisionName" then
            current.revision = scalar(value)
          elseif key == "tag" then
            current.tag = scalar(value)
          elseif key == "latestRevision" and scalar(value) == "true" then
            current.latest = true
          end
        end
      end

      if key == "latestReadyRevisionName" and live.latest_ready == nil then
        live.latest_ready = scalar(value)
      elseif key == "latestCreatedRevisionName" and live.latest_created == nil then
        live.latest_created = scalar(value)
      elseif key == "actualReplicas" and live.actual == nil then
        live.actual = parse_number(value)
      elseif key == "desiredReplicas" and live.desired == nil then
        live.desired = parse_number(value)
      elseif key == "readyReplicas" and live.ready == nil then
        live.ready = parse_number(value)
      end
    end
  end
  finish_entry()
  live.traffic = traffic_entries
  return live
end

local function traffic_text(entries)
  local parts = {}
  for _, entry in ipairs(entries or {}) do
    if entry.percent ~= nil or entry.revision ~= nil then
      local label = ""
      if entry.percent ~= nil then
        label = tostring(entry.percent) .. "%"
      end
      if entry.revision ~= nil then
        label = label ~= "" and (label .. " " .. entry.revision) or entry.revision
      elseif entry.latest then
        label = label ~= "" and (label .. " latest") or "latest"
      end
      if entry.tag ~= nil and entry.tag ~= "" then
        label = label .. " (" .. entry.tag .. ")"
      end
      parts[#parts + 1] = label
    end
  end
  if #parts == 0 then
    return nil
  end
  return table.concat(parts, ", ")
end

local function revision_display(value)
  local name = as_text(value)
  if name == nil or name == "" then
    return nil
  end
  local digits = name:match("(%d+)$")
  return digits or name
end

local function pod_text(metrics, live, summary)
  local metrics_table = is_table(metrics) and metrics or {}
  local ready = tonumber(metrics_table.ready_replicas)
  local total = tonumber(metrics_table.replicas)
  if ready == nil then
    ready = live.ready or live.actual
  end
  if total == nil then
    total = live.desired
  end
  if ready == nil then
    ready = tonumber(summary:match("ready=(%d+)"))
  end
  if ready == nil or total == nil then
    local observed, desired = summary:match("(%d+)%s*/%s*(%d+)%s+replicas")
    if observed ~= nil then
      ready = ready or tonumber(observed)
      total = total or tonumber(desired)
    end
  end
  if total == nil then
    total = tonumber(summary:match("(%d+)%s+replicas"))
  end
  if ready ~= nil and total ~= nil and ready ~= total then
    return string.format("%d/%d", math.floor(ready), math.floor(total))
  end
  local value = ready or total
  if value ~= nil then
    return tostring(math.floor(value))
  end
  return nil
end

local function derive_status(details, scan)
  local status = {}
  if not is_table(details) then
    return status
  end
  local metrics = is_table(details.metrics) and details.metrics or {}
  local live = scan_live_status(details.yaml)
  local identity = is_table(details.id) and details.id or {}
  local summary = as_text(details.summary) or ""
  local revision

  if as_text(identity.kind) == "Revision" or as_text(details.category) == "revision" then
    revision = as_text(details.name)
  end
  if revision == nil then
    revision = live.latest_ready or live.latest_created
  end
  if revision == nil then
    revision = summary:match("[Rr]evision%s+([%w%.%-]+)")
      or summary:match("([%w%.%-]+%-%d%d%d%d%d)")
  end
  if revision ~= nil then
    status.revision = revision_display(revision)
  end
  status.pods = pod_text(metrics, live, summary)
  status.traffic = traffic_text(live.traffic)
  if status.traffic == nil and summary:match("%d+%%") then
    status.traffic = summary
  end
  status.health = as_text(details.health)
  return status
end

local function same_id(left, right)
  return is_table(left)
    and is_table(right)
    and as_text(left.api_version) == as_text(right.api_version)
    and as_text(left.kind) == as_text(right.kind)
    and as_text(left.namespace) == as_text(right.namespace)
    and as_text(left.name) == as_text(right.name)
end

local LINK_VIAS = {
  secret = true,
  config_map = true,
  env = true,
  operator_crd = true,
  dns = true,
}

local function link_hints(details)
  local hints = {}
  local seen = {}
  if not is_table(details) then
    return hints
  end
  local details_id = is_table(details.id) and details.id or nil
  for _, link in ipairs(as_list(details.links)) do
    if is_table(link) then
      local via = as_text(link.via)
      if via == nil or LINK_VIAS[via] then
        local target = is_table(link.target) and link.target or nil
        local source = is_table(link.source) and link.source or nil
        local resource
        if target ~= nil and same_id(target, details_id) then
          resource = source
        else
          resource = target or source
        end
        local name = resource and as_text(resource.name) or nil
        if name ~= nil and name ~= "" then
          local health = as_text(link.health)
          local label = health ~= nil and (name .. " " .. health) or name
          if not seen[label] then
            seen[label] = true
            hints[#hints + 1] = { text = label, health = health }
          end
        end
      end
    end
  end
  return hints
end

local POLICY_RESULTS = {
  fail = true,
  error = true,
  warn = true,
}

local function policy_label(record)
  local kind = as_text(record.kind)
  if kind ~= nil and kind ~= "" and kind ~= "other" then
    return kind
  end
  local name = as_text(record.name)
  if name ~= nil and name ~= "" then
    return name
  end
  return "policy"
end

local function policy_hints(details, validation)
  local hints = {}
  local seen = {}
  local function add(record)
    if not is_table(record) then
      return
    end
    local result = as_text(record.result)
    if result ~= nil and not POLICY_RESULTS[result] then
      return
    end
    local message = as_text(record.message) or "policy finding"
    local label = policy_label(record) .. ": " .. message
    if not seen[label] then
      seen[label] = true
      hints[#hints + 1] = {
        text = truncate(label, 120),
        severity = as_text(record.severity),
        result = result,
      }
    end
  end

  local function add_warning(warning)
    if not is_table(warning) then
      return
    end
    local code = (as_text(warning.code) or ""):lower()
    local message = (as_text(warning.message) or ""):lower()
    local severity = as_text(warning.severity)
    if code:find("rwo", 1, true) or code:find("storage", 1, true) then
      return
    end
    local policy_related = code:find("kyverno", 1, true) ~= nil
      or code:find("policy", 1, true) ~= nil
      or message:find("kyverno", 1, true) ~= nil
      or message:find("policy", 1, true) ~= nil
      or severity == "critical"
      or severity == "high"
    if policy_related then
      add({
        kind = as_text(warning.code) or "warning",
        message = as_text(warning.message),
        severity = severity,
        result = "warn",
      })
    end
  end

  if is_table(details) then
    for _, record in ipairs(as_list(details.policies)) do
      add(record)
    end
    for _, warning in ipairs(as_list(details.warnings)) do
      add_warning(warning)
    end
  end
  if is_table(validation) then
    for _, record in ipairs(as_list(validation.violations)) do
      add(record)
    end
    for _, warning in ipairs(as_list(validation.warnings)) do
      add_warning(warning)
    end
  end
  return hints
end

local RWO_MODES = {
  read_write_once = true,
  read_write_once_pod = true,
}

local function has_rwo(record)
  for _, mode in ipairs(as_list(record.access_modes)) do
    if RWO_MODES[as_text(mode)] then
      return true
    end
  end
  return false
end

local function storage_conflict_hints(details)
  local hints = {}
  local seen = {}
  if not is_table(details) then
    return hints
  end
  local function add(text_value, severity)
    if text_value ~= nil and text_value ~= "" and not seen[text_value] then
      seen[text_value] = true
      hints[#hints + 1] = { text = text_value, severity = severity }
    end
  end
  for _, record in ipairs(as_list(details.storage)) do
    if is_table(record) then
      local rwo = has_rwo(record)
      local mounted_by = #as_list(record.mounted_by)
      local emitted = false
      for _, warning in ipairs(as_list(record.warnings)) do
        if is_table(warning) then
          local conflict = is_table(warning.conflict) and warning.conflict or nil
          if conflict ~= nil or (rwo and mounted_by > 1) then
            local pvc = is_table(record.pvc) and as_text(record.pvc.name) or nil
            if pvc == nil and conflict ~= nil and is_table(conflict.pvc) then
              pvc = as_text(conflict.pvc.name)
            end
            pvc = pvc or "PVC"
            local max_scale = conflict and tonumber(conflict.max_scale) or nil
            local message = as_text(warning.message)
            if (message == nil or message == "") and conflict ~= nil then
              message = as_text(conflict.reason)
            end
            local label
            if max_scale ~= nil then
              label = string.format("RWO %s (%d replicas)", pvc, math.floor(max_scale))
            elseif message ~= nil and message ~= "" then
              label = string.format("RWO %s: %s", pvc, message)
            else
              label = string.format("RWO %s", pvc)
            end
            add(label, as_text(warning.severity) or (conflict ~= nil and "high" or nil))
            emitted = true
          end
        end
      end
      if not emitted and rwo and mounted_by > 1 then
        local pvc = is_table(record.pvc) and as_text(record.pvc.name) or "PVC"
        add(string.format("RWO %s (%d replicas)", pvc, mounted_by), "high")
      end
    end
  end
  return hints
end

local function severity_group(severity)
  if severity == "critical" or severity == "high" then
    return "KnectYamlError"
  elseif severity == "medium" then
    return "KnectYamlWarning"
  elseif severity == "low" or severity == "info" or severity == "unknown" then
    return "KnectYamlInfo"
  end
  return "KnectYamlHint"
end

local function health_group(health)
  if health == "healthy" then
    return "KnectYamlHealthy"
  elseif health == "degraded" or health == "progressing" then
    return "KnectYamlWarning"
  elseif health == "blocked" then
    return "KnectYamlError"
  end
  return "KnectYamlHint"
end

local function policy_group(hint)
  if hint.result == "fail" or hint.result == "error" then
    return "KnectYamlError"
  end
  if hint.severity == "critical" or hint.severity == "high" then
    return "KnectYamlError"
  end
  if hint.result == "warn" then
    return "KnectYamlWarning"
  end
  return severity_group(hint.severity)
end

local function build_marks(state)
  local scan = state.scan
  local by_line = {}
  local function add(line, value, group)
    if line == nil or value == nil or value == "" then
      return
    end
    local chunks = by_line[line]
    if chunks == nil then
      chunks = {}
      by_line[line] = chunks
    end
    if #chunks > 0 then
      chunks[#chunks + 1] = { " · ", "KnectYamlSeparator" }
    end
    chunks[#chunks + 1] = { value, group or "KnectYamlHint" }
  end

  local status = derive_status(state.details, scan)
  local image_parts = {}
  if status.revision ~= nil then
    image_parts[#image_parts + 1] = "rev " .. status.revision
  end
  if status.pods ~= nil then
    image_parts[#image_parts + 1] = status.pods .. " pods"
  end
  if status.traffic ~= nil then
    image_parts[#image_parts + 1] = "traffic " .. status.traffic
  end
  local image_text = table.concat(image_parts, " · ")
  for _, entry in ipairs(scan.fields.image) do
    add(entry.line, image_text, "KnectYamlHint")
  end

  local links = link_hints(state.details)
  if #links > 0 then
    for _, field in ipairs({ "envFrom", "secretRef" }) do
      for _, entry in ipairs(scan.fields[field]) do
        for _, hint in ipairs(links) do
          add(entry.line, hint.text, health_group(hint.health))
        end
      end
    end
  end

  local storage = storage_conflict_hints(state.details)
  if #storage > 0 then
    for _, entry in ipairs(scan.fields.volumeMounts) do
      for _, hint in ipairs(storage) do
        add(entry.line, hint.text, severity_group(hint.severity))
      end
    end
  end

  local policies = policy_hints(state.details, state.validation)
  if #policies > 0 then
    local target = scan.first_field_line or 1
    if #scan.fields.image > 0 then
      target = scan.fields.image[1].line
    end
    for _, hint in ipairs(policies) do
      add(target, hint.text, policy_group(hint))
    end
  end

  local lines = {}
  for line in pairs(by_line) do
    lines[#lines + 1] = line
  end
  table.sort(lines)
  local marks = {}
  for _, line in ipairs(lines) do
    marks[#marks + 1] = { line = line, chunks = by_line[line] }
  end
  return marks
end

local function render(state)
  local bufnr = state.bufnr
  if not vim.api.nvim_buf_is_valid(bufnr) then
    return
  end
  pcall(vim.api.nvim_buf_clear_namespace, bufnr, namespace, 0, -1)

  if options.hints and state.scan ~= nil and state.scan.knative then
    for _, mark in ipairs(build_marks(state)) do
      pcall(vim.api.nvim_buf_set_extmark, bufnr, namespace, mark.line - 1, 0, {
        virt_text = mark.chunks,
        virt_text_pos = "eol",
        hl_mode = "combine",
        priority = 100,
        invalidate = true,
      })
    end
  end

  if options.diagnostics then
    diagnostics.publish(
      bufnr,
      diagnostics.merge({
        diagnostics.from_details(state.details, state.scan),
        diagnostics.from_validation(state.validation, state.scan),
      })
    )
  else
    diagnostics.clear(bufnr)
  end
end

local function cancel(state, key)
  local id = state[key]
  if id ~= nil then
    pcall(rpc.cancel, id)
    state[key] = nil
  end
end

local function request_details(state)
  if not state.active then
    return
  end
  state.details_seq = state.details_seq + 1
  local sequence = state.details_seq
  cancel(state, "details_request")
  local bufnr = state.bufnr
  local id = state.scan.id
  state.details_request = rpc.request("graph.get_node_details", {
    id = id,
    include_yaml = true,
  }, function(err, result)
    if attached[bufnr] ~= state or state.details_seq ~= sequence then
      return
    end
    state.details_request = nil
    if err ~= nil then
      state.details = nil
    else
      state.details = is_table(result) and result or nil
    end
    render(state)
  end)
end

local function run_validation(state)
  if not state.active or attached[state.bufnr] ~= state then
    return
  end
  if not vim.api.nvim_buf_is_valid(state.bufnr) then
    return
  end
  state.validation_seq = state.validation_seq + 1
  local sequence = state.validation_seq
  cancel(state, "validation_request")
  local bufnr = state.bufnr
  local text = table.concat(util.buffer_lines(bufnr), "\n")
  local params = {
    manifest = text,
    default_kind = state.scan.kind,
  }
  if state.scan.namespace ~= nil then
    params.namespace = state.scan.namespace
  end
  state.validation_request = rpc.request("policy.validate_manifest", params, function(err, result)
    if attached[bufnr] ~= state or state.validation_seq ~= sequence then
      return
    end
    state.validation_request = nil
    if err ~= nil then
      state.validation = nil
    else
      state.validation = is_table(result) and result or nil
    end
    render(state)
  end)
end

local function identity_key(scan)
  return table.concat({
    scan.api_version or "",
    scan.kind or "",
    scan.namespace or "",
    scan.name or "",
  }, "|")
end

local function build_id(scan)
  local id = {
    api_version = scan.api_version,
    kind = scan.kind,
    name = scan.name,
  }
  if scan.namespace ~= nil then
    id.namespace = scan.namespace
  end
  return id
end

local function refresh(state, opts)
  opts = opts or {}
  local bufnr = state.bufnr
  if not vim.api.nvim_buf_is_valid(bufnr) then
    M.detach(bufnr)
    return
  end
  local scan = M.scan(table.concat(util.buffer_lines(bufnr), "\n"))
  state.scan = scan
  if scan == nil or not scan.knative then
    state.active = false
    state.key = nil
    state.details = nil
    state.validation = nil
    state.details_seq = state.details_seq + 1
    state.validation_seq = state.validation_seq + 1
    cancel(state, "details_request")
    cancel(state, "validation_request")
    render(state)
    return
  end

  state.active = true
  scan.id = build_id(scan)
  local key = identity_key(scan)
  if key ~= state.key then
    state.key = key
    state.details = nil
    state.validation = nil
    state.details_seq = state.details_seq + 1
    state.validation_seq = state.validation_seq + 1
    cancel(state, "details_request")
    cancel(state, "validation_request")
  end
  render(state)
  if opts.fetch_details ~= false then
    request_details(state)
  end
end

local function schedule_change(state)
  if state.debounce == nil then
    local delay = options.debounce_ms or config.get().debounce_ms or 250
    state.debounce = util.debounce(function()
      if attached[state.bufnr] ~= state then
        return
      end
      refresh(state, { fetch_details = true })
      run_validation(state)
    end, delay)
  end
  state.debounce()
end

local function install_buffer_autocmds(state)
  local bufnr = state.bufnr
  local group = vim.api.nvim_create_augroup("KnectYamlBuf" .. tostring(bufnr), { clear = true })
  state.augroup = group
  for _, event in ipairs({ "TextChanged", "TextChangedI", "BufWritePost" }) do
    vim.api.nvim_create_autocmd(event, {
      group = group,
      buffer = bufnr,
      callback = function()
        schedule_change(state)
      end,
    })
  end
  vim.api.nvim_create_autocmd("FileType", {
    group = group,
    buffer = bufnr,
    callback = function()
      local filetype = vim.bo[bufnr].filetype
      if filetype ~= "yaml" and filetype ~= "yml" then
        M.detach(bufnr)
      end
    end,
  })
end

local function is_yaml_buffer(bufnr)
  local filetype = vim.bo[bufnr].filetype
  return filetype == "yaml" or filetype == "yml"
end

--- Attach live hints and diagnostics to one buffer.
---
--- Attaching an already attached buffer refreshes its scan without creating
--- duplicate extmarks or autocmds.
---@param bufnr integer|nil
---@return table|nil state
function M.attach(bufnr)
  bufnr = bufnr or vim.api.nvim_get_current_buf()
  if not vim.api.nvim_buf_is_valid(bufnr) or not vim.api.nvim_buf_is_loaded(bufnr) then
    return nil
  end
  if not is_yaml_buffer(bufnr) then
    return nil
  end
  ensure_highlights()

  local state = attached[bufnr]
  if state ~= nil then
    local was_active = state.active
    refresh(state, { fetch_details = false })
    if not was_active and state.active then
      request_details(state)
    end
    return state
  end

  state = {
    bufnr = bufnr,
    active = false,
    key = nil,
    scan = nil,
    details = nil,
    validation = nil,
    details_seq = 0,
    validation_seq = 0,
    details_request = nil,
    validation_request = nil,
    debounce = nil,
    augroup = nil,
  }
  attached[bufnr] = state
  install_buffer_autocmds(state)
  refresh(state, { fetch_details = true })
  return state
end

--- Remove hints, diagnostics, timers, and autocmds for one buffer.
---@param bufnr integer|nil
function M.detach(bufnr)
  bufnr = bufnr or vim.api.nvim_get_current_buf()
  local state = attached[bufnr]
  if state == nil then
    if vim.api.nvim_buf_is_valid(bufnr) then
      pcall(vim.api.nvim_buf_clear_namespace, bufnr, namespace, 0, -1)
      diagnostics.clear(bufnr)
    end
    return
  end
  attached[bufnr] = nil
  state.details_seq = state.details_seq + 1
  state.validation_seq = state.validation_seq + 1
  cancel(state, "details_request")
  cancel(state, "validation_request")
  if state.augroup ~= nil then
    pcall(vim.api.nvim_del_augroup_by_id, state.augroup)
    state.augroup = nil
  end
  if vim.api.nvim_buf_is_valid(bufnr) then
    pcall(vim.api.nvim_buf_clear_namespace, bufnr, namespace, 0, -1)
    diagnostics.clear(bufnr)
  end
end

--- Detach every buffer (used by tests and when hints are disabled).
function M.detach_all()
  local buffers = {}
  for bufnr in pairs(attached) do
    buffers[#buffers + 1] = bufnr
  end
  for _, bufnr in ipairs(buffers) do
    M.detach(bufnr)
  end
end

--- Refresh live state for one attached buffer.
---@param bufnr integer|nil
function M.refresh(bufnr)
  bufnr = bufnr or vim.api.nvim_get_current_buf()
  local state = attached[bufnr]
  if state ~= nil then
    refresh(state, { fetch_details = true })
  end
end

--- Run pre-flight validation for one attached buffer immediately.
---@param bufnr integer|nil
function M.validate(bufnr)
  bufnr = bufnr or vim.api.nvim_get_current_buf()
  local state = attached[bufnr]
  if state ~= nil then
    run_validation(state)
  end
end

local function install_global_autocmds()
  local group = vim.api.nvim_create_augroup("KnectYaml", { clear = true })
  vim.api.nvim_create_autocmd("FileType", {
    group = group,
    pattern = { "yaml", "yml" },
    callback = function(args)
      M.attach(args.buf)
    end,
  })
  vim.api.nvim_create_autocmd({ "BufReadPost", "BufNewFile" }, {
    group = group,
    pattern = { "*.yaml", "*.yml" },
    callback = function(args)
      M.attach(args.buf)
    end,
  })
  vim.api.nvim_create_autocmd({ "BufDelete", "BufWipeout" }, {
    group = group,
    callback = function(args)
      M.detach(args.buf)
    end,
  })
end

local function scan_existing_buffers()
  for _, bufnr in ipairs(vim.api.nvim_list_bufs()) do
    if vim.api.nvim_buf_is_loaded(bufnr) and is_yaml_buffer(bufnr) then
      M.attach(bufnr)
    end
  end
end

--- Configure auto-attach and debounce behavior.
---@param opts table|nil { auto_attach?: boolean, hints?: boolean, diagnostics?: boolean, debounce_ms?: number }
---@return table
function M.setup(opts)
  opts = opts or {}
  options.auto_attach = opts.auto_attach ~= false
  options.hints = opts.hints ~= false
  options.diagnostics = opts.diagnostics ~= false
  options.debounce_ms = opts.debounce_ms
  ensure_highlights()

  if options.auto_attach then
    install_global_autocmds()
    scan_existing_buffers()
  else
    pcall(vim.api.nvim_del_augroup_by_name, "KnectYaml")
    M.detach_all()
  end
  return M
end

--- Namespace used for extmarks (also handy in tests).
---@return integer
function M.namespace()
  return namespace
end

--- Introspection used by the headless test suite.
---@param bufnr integer|nil
---@return table|nil|integer
function M._state(bufnr)
  if bufnr ~= nil then
    return attached[bufnr]
  end
  local count = 0
  for _ in pairs(attached) do
    count = count + 1
  end
  return count
end

return M
