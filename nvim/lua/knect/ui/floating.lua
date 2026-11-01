-- Dependency-free floating-window topology adapter.

local util = require("knect.util")

local M = {}

local Controller = {}
Controller.__index = Controller

local function layout()
  local columns = math.max(20, vim.o.columns)
  local screen_lines = math.max(8, vim.o.lines - vim.o.cmdheight)
  local tree_width = math.max(20, math.floor(columns * 0.45))
  local details_width = columns - tree_width - 5
  if details_width < 18 then
    details_width = math.max(18, columns - tree_width - 2)
    tree_width = math.max(16, columns - details_width - 2)
  end
  local height = math.max(8, math.floor(screen_lines * 0.72))
  local row = math.max(1, math.floor((screen_lines - height) / 2) - 1)
  local left = math.max(0, math.floor((columns - tree_width - details_width - 2) / 2))
  return {
    tree = { row = row, col = left, width = tree_width, height = height },
    details = {
      row = row,
      col = left + tree_width + 2,
      width = details_width,
      height = height,
    },
  }
end

local function open_window(bufnr, config, title, enter)
  return vim.api.nvim_open_win(bufnr, enter, {
    relative = "editor",
    row = config.row,
    col = config.col,
    width = config.width,
    height = config.height,
    style = "minimal",
    border = "rounded",
    title = " " .. title .. " ",
    title_pos = "center",
  })
end

--- Create two floating windows: a topology tree and a details side pane.
---@param opts table callbacks: on_next, on_prev, on_inspect, on_refresh, on_close
---@return table controller
function M.new(opts)
  local geometry = layout()
  local tree_buf = util.scratch_buffer("knect://topology", {
    filetype = "knect-topology",
    bufhidden = "hide",
  })
  local details_buf = util.scratch_buffer("knect://details", {
    filetype = "knect-details",
    bufhidden = "hide",
  })

  local tree_win = open_window(tree_buf, geometry.tree, "Knect topology", true)
  local details_win = open_window(details_buf, geometry.details, "Details", false)

  vim.api.nvim_set_option_value("wrap", false, { win = tree_win })
  vim.api.nvim_set_option_value("number", false, { win = tree_win })
  vim.api.nvim_set_option_value("cursorline", true, { win = tree_win })
  vim.api.nvim_set_option_value("wrap", true, { win = details_win })
  vim.api.nvim_set_option_value("number", false, { win = details_win })

  local controller = setmetatable({
    kind = "floating",
    tree_buf = tree_buf,
    details_buf = details_buf,
    tree_win = tree_win,
    details_win = details_win,
    closed = false,
  }, Controller)

  local function map(lhs, callback)
    vim.keymap.set("n", lhs, callback, { buffer = tree_buf, nowait = true, silent = true })
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
    if controller.details_win and vim.api.nvim_win_is_valid(controller.details_win) then
      vim.api.nvim_set_current_win(controller.details_win)
    end
  end)

  vim.keymap.set("n", "q", opts.on_close, { buffer = details_buf, nowait = true, silent = true })
  vim.keymap.set("n", "<Esc>", opts.on_close, { buffer = details_buf, nowait = true, silent = true })
  vim.keymap.set("n", "<Tab>", function()
    controller:focus_tree()
  end, { buffer = details_buf, nowait = true, silent = true })

  controller:set_details({ "Select a node to inspect it." })
  return controller
end

function Controller:set_content(lines)
  if not self.closed and vim.api.nvim_buf_is_valid(self.tree_buf) then
    util.set_buffer_lines(self.tree_buf, lines)
  end
end

function Controller:set_details(lines)
  if not self.closed and vim.api.nvim_buf_is_valid(self.details_buf) then
    util.set_buffer_lines(self.details_buf, lines)
  end
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
  util.close_window(self.details_win)
  util.close_window(self.tree_win)
  util.delete_buffer(self.details_buf)
  util.delete_buffer(self.tree_buf)
end

return M
