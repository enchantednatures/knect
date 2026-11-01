-- Minimal Neovim init for the headless Plenary suite.

local source = debug.getinfo(1, "S").source:sub(2)
local nvim_dir = vim.fn.fnamemodify(source, ":p:h:h")
local repo_root = vim.env.KNECT_REPO_ROOT
if repo_root == nil or repo_root == "" then
  repo_root = vim.fn.fnamemodify(nvim_dir, ":h")
end

vim.opt.runtimepath:prepend(nvim_dir)
package.path = nvim_dir .. "/tests/?.lua;" .. nvim_dir .. "/tests/?/init.lua;" .. package.path

local deps = vim.env.KNECT_NVIM_DEPS
if deps ~= nil and deps ~= "" then
  for _, name in ipairs({ "plenary.nvim", "nui.nvim", "telescope.nvim" }) do
    local path = deps .. "/" .. name
    if vim.fn.isdirectory(path) == 1 then
      vim.opt.runtimepath:prepend(path)
    end
  end
end

vim.opt.swapfile = false
vim.opt.undofile = false
vim.opt.shadafile = "NONE"
vim.opt.more = false
vim.opt.shortmess:append("I")
vim.g.knect_test = 1
vim.g.knect_repo_root = repo_root

vim.cmd("runtime! plugin/knect.lua")
