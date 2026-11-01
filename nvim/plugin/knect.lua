if vim.g.loaded_knect == 1 then
  return
end
vim.g.loaded_knect = 1

require("knect").ensure_commands()
