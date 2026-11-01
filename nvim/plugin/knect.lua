if vim.g.loaded_knect == 1 then
  return
end
vim.g.loaded_knect = 1

require("knect").ensure_commands()
-- Live YAML hints are intrusive by design. The headless suite disables
-- auto-attach so unrelated tests never issue daemon requests; the yaml spec
-- enables it explicitly.
require("knect.yaml").setup({ auto_attach = vim.g.knect_test ~= 1 })
