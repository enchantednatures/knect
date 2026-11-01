-- Three-way mutation diff: the local buffer, the live redacted object, and
-- managedFields attribution. Buffers are scratch; vim.diff supplies the local
-- vs live patch summary and Neovim's native diff mode handles the panes.

local rpc = require("knect.rpc")
local util = require("knect.util")

local M = {}

local current

local function fail(message)
  util.notify(message, vim.log.levels.ERROR)
  return nil
end

local function find_identity(opts)
  if opts.id ~= nil then
    return opts.id, nil, opts.text
  end
  local bufnr = opts.bufnr or vim.api.nvim_get_current_buf()
  local filetype = vim.bo[bufnr].filetype
  local lines = util.buffer_lines(bufnr)
  local text = table.concat(lines, "\n")
  if filetype ~= "yaml" and filetype ~= "yml" and not text:match("^%s*apiVersion%s*:") then
    return nil, "current buffer is not a Kubernetes YAML manifest", text
  end
  local id, err = util.parse_manifest(text)
  if id == nil then
    return nil, "cannot identify manifest: " .. tostring(err), text
  end
  return id, nil, text
end

local function attribution_lines(diff)
  local lines = {
    "# managedFields attribution",
    string.format("%-18s %-32s %-8s %s", "# side", "owner", "changed", "path"),
  }
  local diff_lines = diff.lines or {}
  for _, line in ipairs(diff_lines) do
    lines[#lines + 1] = string.format(
      "%-18s %-32s %-8s %s",
      tostring(line.side or "unknown"),
      tostring(line.owner or "-"),
      line.changed and "yes" or "no",
      tostring(line.path or "-")
    )
    if line.text ~= nil and line.text ~= "" then
      lines[#lines + 1] = "    " .. tostring(line.text)
    end
  end
  if #diff_lines == 0 then
    lines[#lines + 1] = "# no line attribution returned"
    if diff.policy_mutators ~= nil then
      lines[#lines + 1] = "# policy mutators document"
      vim.list_extend(lines, util.split_lines(diff.policy_mutators))
    end
  end
  return lines
end

local function scratch(name, lines, filetype)
  local bufnr = util.scratch_buffer(name, {
    filetype = filetype,
    bufhidden = "hide",
  })
  util.set_buffer_lines(bufnr, lines)
  return bufnr
end

local function render(controller)
  local local_win = controller.local_win
  if local_win == nil or not vim.api.nvim_win_is_valid(local_win) then
    local_win = vim.api.nvim_get_current_win()
  else
    vim.api.nvim_set_current_win(local_win)
  end
  if controller.local_buf ~= nil and vim.api.nvim_buf_is_valid(controller.local_buf) then
    vim.api.nvim_win_set_buf(local_win, controller.local_buf)
  end
  vim.cmd("vsplit")
  local live_win = vim.api.nvim_get_current_win()
  vim.api.nvim_win_set_buf(live_win, controller.live_buf)
  vim.cmd("vsplit")
  local attribution_win = vim.api.nvim_get_current_win()
  vim.api.nvim_win_set_buf(attribution_win, controller.attribution_buf)

  controller.local_win = local_win
  controller.live_win = live_win
  controller.attribution_win = attribution_win
  controller.wins = { local_win, live_win, attribution_win }
  controller.bufs = { controller.live_buf, controller.attribution_buf }

  for _, winid in ipairs(controller.wins) do
    pcall(vim.api.nvim_win_call, winid, function()
      vim.cmd("diffthis")
    end)
  end
  pcall(vim.cmd, "wincmd =")
  if vim.api.nvim_win_is_valid(local_win) then
    vim.api.nvim_set_current_win(local_win)
  end

  local function close()
    M.close()
  end
  vim.keymap.set("n", "q", close, { buffer = controller.live_buf, nowait = true, silent = true })
  vim.keymap.set("n", "q", close, { buffer = controller.attribution_buf, nowait = true, silent = true })
  vim.keymap.set("n", "<Esc>", close, { buffer = controller.live_buf, nowait = true, silent = true })
  vim.keymap.set("n", "<Esc>", close, { buffer = controller.attribution_buf, nowait = true, silent = true })
end

--- Open the three-way diff for the current YAML buffer (or `opts.id`).
---@param opts table|nil { id?: table, bufnr?: integer, text?: string }
---@return table|nil controller
function M.open(opts)
  opts = opts or {}
  if current ~= nil then
    M.close()
  end

  local id, identity_error, local_text = find_identity(opts)
  if id == nil then
    return fail("KnectDiff: " .. tostring(identity_error))
  end
  local local_buf = opts.bufnr or vim.api.nvim_get_current_buf()
  local local_win = vim.api.nvim_get_current_win()
  local_text = local_text or table.concat(util.buffer_lines(local_buf), "\n")

  local details_result
  local diff_result
  local errors = {}
  local remaining = 2

  local function finish_request()
    remaining = remaining - 1
    if remaining > 0 then
      return
    end
    if type(details_result) ~= "table" or type(diff_result) ~= "table" then
      local parts = {}
      for _, err in ipairs(errors) do
        parts[#parts + 1] = util.format_error(err)
      end
      fail("KnectDiff: cannot build diff: " .. table.concat(parts, "; "))
      return
    end

    local live_text = details_result.yaml
    if live_text == nil or live_text == vim.NIL then
      live_text = "# daemon did not return YAML for this object\n"
    end
    local live_lines = util.split_lines(live_text)
    local managed_lines = attribution_lines(diff_result)

    local live_buf = scratch(
      string.format("knect://diff/live/%s", tostring(id.name)),
      live_lines,
      "yaml"
    )
    local attribution_buf = scratch(
      string.format("knect://diff/managed/%s", tostring(id.name)),
      managed_lines,
      "yaml"
    )

    local controller = {
      id = id,
      local_text = local_text,
      local_buf = local_buf,
      local_win = local_win,
      live_text = live_text,
      attribution = managed_lines,
      live_buf = live_buf,
      attribution_buf = attribution_buf,
      raw = nil,
    }
    local ok, diff_output = pcall(vim.diff, local_text, live_text, {
      result_type = "unified",
      ctxlen = 3,
    })
    if ok then
      controller.raw = diff_output
    end
    render(controller)
    current = controller
  end

  rpc.request("graph.get_node_details", {
    id = id,
    include_yaml = true,
  }, function(err, result)
    if err ~= nil then
      errors[#errors + 1] = err
    else
      details_result = result
    end
    finish_request()
  end)

  rpc.request("mutations.get_diff", {
    id = id,
    mode = "three_way",
  }, function(err, result)
    if err ~= nil then
      errors[#errors + 1] = err
    else
      diff_result = result
    end
    finish_request()
  end)

  return {
    pending = true,
    id = id,
  }
end

--- Close the active diff and restore the local window.
function M.close()
  local controller = current
  if controller == nil then
    return
  end
  current = nil
  for _, winid in ipairs(controller.wins or {}) do
    if winid and vim.api.nvim_win_is_valid(winid) then
      pcall(vim.api.nvim_win_call, winid, function()
        vim.cmd("diffoff")
      end)
    end
  end
  util.close_window(controller.attribution_win)
  util.close_window(controller.live_win)
  util.delete_buffer(controller.attribution_buf)
  util.delete_buffer(controller.live_buf)
end

--- Introspection used by the test suite.
---@return table|nil
function M._current()
  return current
end

M._attribution = attribution_lines

return M
