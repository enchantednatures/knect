-- Telescope.nvim picker adapter. All requires are guarded so Telescope stays
-- an optional integration.

local config = require("knect.config")

local M = {}

--- Return the Telescope modules required to open a picker, or nil.
---@return table|nil
local function telescope_modules()
  if not config.get().telescope then
    return nil
  end
  local ok_pickers, pickers = pcall(require, "telescope.pickers")
  local ok_finders, finders = pcall(require, "telescope.finders")
  local ok_config, telescope_config = pcall(require, "telescope.config")
  local ok_actions, actions = pcall(require, "telescope.actions")
  local ok_state, action_state = pcall(require, "telescope.actions.state")
  if not (ok_pickers and ok_finders and ok_config and ok_actions and ok_state) then
    return nil
  end
  return {
    pickers = pickers,
    finders = finders,
    values = telescope_config.values,
    actions = actions,
    action_state = action_state,
  }
end

--- True when Telescope can be used right now.
---@return boolean
function M.available()
  return telescope_modules() ~= nil
end

--- Open a Telescope picker.
---@param opts table { title: string, entries: table[], on_select: fun(entry:table) }
---@return boolean opened
function M.pick(opts)
  local modules = telescope_modules()
  if modules == nil then
    return false
  end
  local ok, err = pcall(function()
    modules.pickers
      .new({}, {
        prompt_title = opts.title,
        finder = modules.finders.new_table({
          results = opts.entries,
          entry_maker = function(entry)
            return {
              value = entry,
              display = entry.label,
              ordinal = entry.ordinal or entry.label,
            }
          end,
        }),
        sorter = modules.values.generic_sorter({}),
        attach_mappings = function(prompt_bufnr, map)
          local function select()
            local selection = modules.action_state.get_selected_entry()
            modules.actions.close(prompt_bufnr)
            if selection ~= nil and opts.on_select ~= nil then
              opts.on_select(selection.value, selection)
            end
          end
          map("i", "<CR>", select)
          map("n", "<CR>", select)
          return true
        end,
      })
      :find()
  end)
  if not ok then
    return false, tostring(err)
  end
  return true
end

return M
