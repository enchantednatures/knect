-- Live CloudEvent tap. Opens a scratch buffer, subscribes to the UDS event
-- stream, calls tap.start, and appends every tap.event with its CloudEvent
-- attributes and pretty-printed payload.

local config = require("knect.config")
local pickers = require("knect.pickers")
local rpc = require("knect.rpc")
local util = require("knect.util")

local M = {}

local state
local sequence = 0

local function buffer_append(st, lines)
  if st == nil or st.closed or not vim.api.nvim_buf_is_valid(st.buf) then
    return
  end
  local count = vim.api.nvim_buf_line_count(st.buf)
  vim.api.nvim_set_option_value("modifiable", true, { buf = st.buf })
  vim.api.nvim_buf_set_lines(st.buf, count, count, false, lines)
  vim.api.nvim_set_option_value("modifiable", false, { buf = st.buf })
  if st.win ~= nil and vim.api.nvim_win_is_valid(st.win) then
    local new_count = vim.api.nvim_buf_line_count(st.buf)
    pcall(vim.api.nvim_win_set_cursor, st.win, { new_count, 0 })
  end
end

local function append_event(event)
  local st = state
  if st == nil or st.closed then
    return
  end
  local data = event.data or {}
  st.event_count = (st.event_count or 0) + 1
  local lines = {
    string.format(
      "-- event seq=%s  received=%s",
      tostring(event.seq or "?"),
      tostring(data.received_at or event.ts or "?")
    ),
    "ce-id:     " .. tostring(data.ce_id or "?"),
    "ce-type:   " .. tostring(data.ce_type or "?"),
    "ce-source: " .. tostring(data.ce_source or "?"),
    "ce-time:   " .. tostring(data.ce_time or "?"),
  }
  if data.ce_subject ~= nil and data.ce_subject ~= vim.NIL then
    lines[#lines + 1] = "ce-subject: " .. tostring(data.ce_subject)
  end
  lines[#lines + 1] = "payload:"
  local payload = util.pretty_json(data.payload or {})
  for _, line in ipairs(util.split_lines(payload)) do
    lines[#lines + 1] = "  " .. line
  end
  lines[#lines + 1] = ""
  buffer_append(st, lines)
end

local function create_window(bufnr, title)
  vim.cmd("botright split")
  local winid = vim.api.nvim_get_current_win()
  vim.api.nvim_win_set_buf(winid, bufnr)
  vim.api.nvim_win_set_height(winid, math.min(16, math.max(8, math.floor(vim.o.lines / 3))))
  vim.api.nvim_set_option_value("wrap", false, { win = winid })
  vim.api.nvim_set_option_value("number", false, { win = winid })
  pcall(function()
    vim.api.nvim_win_set_config(winid, { title = " " .. title .. " ", title_pos = "left" })
  end)
  return winid
end

local function parse_target(fargs)
  if #fargs == 0 then
    return nil, nil
  end
  local kind = "broker"
  local namespace
  local name
  if #fargs == 1 then
    local left, right = fargs[1]:match("^([^/]+)/(.+)$")
    if left == nil then
      return nil, "usage: :KnectTap [broker|trigger] [namespace/name]"
    end
    namespace, name = left, right
  elseif #fargs == 2 then
    local left, right = fargs[2]:match("^([^/]+)/(.+)$")
    if left ~= nil then
      kind, namespace, name = fargs[1], left, right
    else
      namespace, name = fargs[1], fargs[2]
    end
  elseif #fargs == 3 then
    kind, namespace, name = fargs[1], fargs[2], fargs[3]
  else
    return nil, "usage: :KnectTap [broker|trigger] [namespace/name]"
  end
  kind = tostring(kind):lower()
  if kind ~= "broker" and kind ~= "trigger" then
    return nil, "tap target kind must be broker or trigger"
  end
  if namespace == "" or name == "" then
    return nil, "tap target namespace and name are required"
  end
  return { kind = kind, namespace = namespace, name = name }, nil
end

--- Open a live tap for a Broker or Trigger.
---@param target table { kind: "broker"|"trigger", namespace: string, name: string }
---@return table|nil state
function M.open(target)
  if type(target) ~= "table" or target.kind == nil or target.namespace == nil or target.name == nil then
    util.notify("tap target must include kind, namespace, and name", vim.log.levels.ERROR)
    return nil
  end
  local preview = tostring(target.kind):lower()
  if preview ~= "broker" and preview ~= "trigger" then
    util.notify("tap target kind must be broker or trigger", vim.log.levels.ERROR)
    return nil
  end
  if state ~= nil then
    M.close()
  end

  sequence = sequence + 1
  local bufnr = util.scratch_buffer("knect://tap/" .. tostring(sequence), {
    filetype = "knect-tap",
    bufhidden = "hide",
  })
  local st = {
    buf = bufnr,
    win = create_window(bufnr, "Knect tap"),
    target = target,
    tap_id = nil,
    event_count = 0,
    closed = false,
    sub = nil,
  }
  state = st

  buffer_append(st, {
    string.format("Knect tap: %s %s/%s", preview, target.namespace, target.name),
    "starting tap...",
  })

  st.sub = rpc.subscribe_events(function(event)
    if event.kind == "tap.event" then
      append_event(event)
    end
  end, {
    on_error = function(err)
      buffer_append(st, { "-- event stream error: " .. util.format_error(err) })
    end,
  })

  vim.api.nvim_create_autocmd("BufWipeout", {
    buffer = bufnr,
    once = true,
    callback = function()
      M.close({ from_buffer = true })
    end,
  })

  local cfg = config.get()
  rpc.request("tap.start", {
    target = target,
    max_events = cfg.tap.max_events,
    ttl_secs = cfg.tap.ttl_secs,
  }, function(err, handle)
    if state ~= st or st.closed then
      return
    end
    if err ~= nil then
      buffer_append(st, { "-- tap.start failed: " .. util.format_error(err) })
      util.notify("tap.start failed: " .. util.format_error(err), vim.log.levels.ERROR)
      return
    end
    st.tap_id = handle and handle.id
    buffer_append(st, {
      string.format(
        "-- tap active id=%s expires=%s",
        tostring(handle and handle.id or "?"),
        tostring(handle and handle.expires or "?")
      ),
      "",
    })
    util.notify("tap active: " .. tostring(handle and handle.id or "?"))
  end)

  return st
end

--- Parse command arguments and open a tap, or prompt for a Broker/Trigger.
---@param fargs string[]
function M.open_command(fargs)
  local target, err = parse_target(fargs or {})
  if err ~= nil then
    util.notify(err, vim.log.levels.ERROR)
    return
  end
  if target == nil then
    pickers.select_target(function(selected)
      if selected ~= nil then
        M.open(selected)
      end
    end)
    return
  end
  M.open(target)
end

--- Stop the tap and close its buffer.
---@param opts table|nil { from_buffer?: boolean }
function M.close(opts)
  opts = opts or {}
  local st = state
  if st == nil then
    return
  end
  state = nil
  st.closed = true
  if st.sub ~= nil then
    pcall(st.sub.stop)
    st.sub = nil
  end
  if st.tap_id ~= nil then
    rpc.request("tap.stop", { tap_id = st.tap_id })
    st.tap_id = nil
  end
  if not opts.from_buffer then
    util.close_window(st.win)
    util.delete_buffer(st.buf)
  end
end

--- `:KnectTapStop`.
function M.stop()
  M.close()
end

--- Introspection used by the test suite.
---@return table|nil
function M._state()
  return state
end

M._parse_target = parse_target

return M
