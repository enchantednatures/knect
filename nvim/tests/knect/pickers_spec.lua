local config = require("knect.config")
local pickers = require("knect.pickers")
local rpc = require("knect.rpc")
local support = require("knect.support")

local socket_path
local server
local original_select

local function start_server()
  server = support.start_mock(socket_path, {
    ["graph.get_topology"] = support.contract("topology.sample.json"),
    ["graph.get_node_details"] = support.contract("node_details.sample.json"),
  })
end

describe("knect.pickers", function()
before_each(function()
  rpc._reset()
  socket_path = support.temp_socket_path()
  original_select = vim.ui.select
  config.setup({ socket_path = socket_path, timeout_ms = 3000, telescope = false })
  start_server()
end)

after_each(function()
  vim.ui.select = original_select
  rpc._reset()
  if server ~= nil then
    server:close()
    server = nil
  end
  config.reset()
end)

  it("falls back to vim.ui.select for services", function()
    local captured
    local selected
    vim.ui.select = function(items, opts, on_choice)
      captured = { items = items, prompt = opts.prompt }
      on_choice(items[1], 1)
    end

    pickers.open("services", {
      on_select = function(entry)
        selected = entry
      end,
    })

    support.wait_for(function()
      return selected ~= nil
    end, 3000, "service selection")
    assert.are.equal("Knect services", captured.prompt)
    assert.is_truthy(table.concat(captured.items, "\n"):find("hello", 1, true))
    assert.are.equal("hello", selected.node.name)
    assert.are.equal("k_service", selected.node.category)
  end)

  it("collects policy violations from node details", function()
    local captured
    local selected
    vim.ui.select = function(items, opts, on_choice)
      captured = { items = items, prompt = opts.prompt }
      on_choice(items[1], 1)
    end

    pickers.open("violations", {
      on_select = function(entry)
        selected = entry
      end,
    })

    support.wait_for(function()
      return selected ~= nil
    end, 5000, "violation selection")
    assert.are.equal("Knect violations", captured.prompt)
    local rendered = table.concat(captured.items, "\n")
    assert.is_truthy(rendered:find("kyverno", 1, true))
    assert.is_truthy(rendered:find("require-labels", 1, true))
  end)

  it("guards Telescope availability behind configuration", function()
    local telescope = require("knect.ui.telescope")
    config.setup({ telescope = true })
    local installed = telescope.available()
    config.setup({ telescope = false })
    assert.is_false(telescope.available())
    config.setup({ telescope = true })
    assert.are.equal(installed, telescope.available())
  end)
end)
