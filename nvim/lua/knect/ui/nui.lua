-- Nui.nvim topology adapter. Loaded with pcall; the floating adapter is used
-- when Nui is absent or fails to construct.

local util = require("knect.util")

local M = {}

local function is_callable(value)
  if type(value) == "function" then
    return true
  end
  if type(value) ~= "table" then
    return false
  end
  local metatable = getmetatable(value)
  return metatable ~= nil and metatable.__call ~= nil
end

local function load_nui_module(name)
  local ok, module = pcall(require, "nui." .. name)
  if ok and is_callable(module) then
    return module
  end
  local ok_nui, nui = pcall(require, "nui")
  local class_name = name:sub(1, 1):upper() .. name:sub(2)
  if ok_nui and type(nui) == "table" and is_callable(nui[class_name]) then
    return nui[class_name]
  end
  return nil
end

--- Return true when the Nui split component can be loaded.
---@return boolean
function M.available()
  return load_nui_module("split") ~= nil
end

local Controller = {}
Controller.__index = Controller

--- Create and mount a split-based explorer.
---@param opts table callbacks: on_next, on_prev, on_inspect, on_refresh, on_close
---@return table|nil controller
---@return string|nil error
function M.new(opts)
  local Split = load_nui_module("split")
  if Split == nil then
    return nil, "nui.split is not available"
  end

  local details, tree
  local ok, err = pcall(function()
    details = Split({
      position = "right",
      size = "35%",
      relative = "editor",
      enter = false,
      buf_options = {
        modifiable = false,
        filetype = "knect-details",
        swapfile = false,
      },
      win_options = {
        wrap = true,
        number = false,
        relativenumber = false,
        signcolumn = "no",
        cursorline = false,
        winhighlight = "Normal:NormalFloat,FloatBorder:FloatBorder",
      },
    })
    tree = Split({
      position = "right",
      size = "45%",
      relative = "editor",
      enter = false,
      buf_options = {
        modifiable = false,
        filetype = "knect-topology",
        swapfile = false,
      },
      win_options = {
        wrap = false,
        number = false,
        relativenumber = false,
        signcolumn = "no",
        cursorline = true,
        winhighlight = "Normal:NormalFloat,FloatBorder:FloatBorder",
      },
    })
    details:mount()
    tree:mount()
  end)
  if not ok then
    pcall(function()
      if tree then
        tree:unmount()
      end
      if details then
        details:unmount()
      end
    end)
    return nil, tostring(err)
  end

  pcall(vim.api.nvim_buf_set_name, tree.bufnr, "knect://topology")
  pcall(vim.api.nvim_buf_set_name, details.bufnr, "knect://details")
  if tree.winid and vim.api.nvim_win_is_valid(tree.winid) then
    vim.api.nvim_set_current_win(tree.winid)
  end

  local controller = setmetatable({
    kind = "nui",
    tree = tree,
    details = details,
    tree_buf = tree.bufnr,
    details_buf = details.bufnr,
    tree_win = tree.winid,
    details_win = details.winid,
    closed = false,
  }, Controller)

  controller:map(tree.bufnr, opts)
  vim.api.nvim_set_option_value("modifiable", false, { buf = tree.bufnr })
  vim.api.nvim_set_option_value("modifiable", false, { buf = details.bufnr })
  controller:set_details({ "Select a node to inspect it." })
  return controller
end

function Controller:map(bufnr, opts)
  local function map(lhs, callback)
    vim.keymap.set("n", lhs, callback, { buffer = bufnr, nowait = true, silent = true })
  end
  map("j", opts.on_next)
  map("<Down>", opts.on_next)
  map("k", opts.on_prev)
  map("<Up>", opts.on_prev)
  map("<CR>", opts.on_inspect)
  map("r", opts.on_refresh)
  map("q", opts.on_close)
  map("<Esc>", opts.on_close)
  map("<Tab>", function()
    if self.details_win and vim.api.nvim_win_is_valid(self.details_win) then
      vim.api.nvim_set_current_win(self.details_win)
    end
  end)
end

function Controller:set_content(lines)
  if self.closed or not vim.api.nvim_buf_is_valid(self.tree_buf) then
    return
  end
  util.set_buffer_lines(self.tree_buf, lines)
end

function Controller:set_details(lines)
  if self.closed or not vim.api.nvim_buf_is_valid(self.details_buf) then
    return
  end
  util.set_buffer_lines(self.details_buf, lines)
end

function Controller:set_cursor(line)
  if self.closed or not self.tree_win or not vim.api.nvim_win_is_valid(self.tree_win) then
    return
  end
  local count = vim.api.nvim_buf_line_count(self.tree_buf)
  line = math.max(1, math.min(line or 1, count))
  pcall(vim.api.nvim_win_set_cursor, self.tree_win, { line, 0 })
end

function Controller:focus_tree()
  if self.tree_win and vim.api.nvim_win_is_valid(self.tree_win) then
    vim.api.nvim_set_current_win(self.tree_win)
  end
end

function Controller:focus_details()
  if self.details_win and vim.api.nvim_win_is_valid(self.details_win) then
    vim.api.nvim_set_current_win(self.details_win)
  end
end

function Controller:close()
  if self.closed then
    return
  end
  self.closed = true
  pcall(function()
    self.tree:unmount()
  end)
  pcall(function()
    self.details:unmount()
  end)
end

return M
