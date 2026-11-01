local config = require("knect.config")
local rpc = require("knect.rpc")
local support = require("knect.support")

local socket_path
local server
local original_select

describe("knect command smoke tests", function()
before_each(function()
  rpc._reset()
  socket_path = support.temp_socket_path()
  original_select = vim.ui.select
  config.setup({ socket_path = socket_path, timeout_ms = 3000, nui = false, telescope = false })
  server = support.start_mock(socket_path, {
    ["graph.get_topology"] = support.contract("topology.sample.json"),
    ["graph.get_node_details"] = support.contract("node_details.sample.json"),
    ["mutations.get_diff"] = support.contract("mutation_diff.sample.json"),
    ["events.subscribe"] = function(params, ctx)
      ctx.respond({ subscription_id = "sub-1" })
    end,
    ["tap.start"] = function(params, ctx)
      ctx.respond(support.tap_handle())
    end,
    ["tap.stop"] = function(params, ctx)
      ctx.respond({ removed = true })
    end,
  })
end)

after_each(function()
  require("knect.topology").close()
  require("knect.tap").close()
  require("knect.diff").close()
  vim.ui.select = original_select
  rpc._reset()
  if server ~= nil then
    server:close()
    server = nil
  end
  config.reset()
end)

  it("executes :Knect through the command layer", function()
    vim.cmd("Knect")
    local topology = require("knect.topology")
    support.wait_for(function()
      local session = topology._session()
      return session ~= nil and session.topology ~= nil
    end, 3000, ":Knect topology")
    assert.is_truthy(table.concat(topology._session().lines, "\n"):find("hello", 1, true))
    topology.close()
  end)

  it("executes :KnectServices through the fallback picker", function()
    local prompt
    vim.ui.select = function(items, opts, on_choice)
      prompt = opts.prompt
      on_choice(items[1], 1)
    end
    vim.cmd("KnectServices")
    support.wait_for(function()
      return prompt ~= nil
    end, 3000, ":KnectServices picker")
    assert.are.equal("Knect services", prompt)
  end)

  it("executes :KnectTap and :KnectTapStop", function()
    local tap = require("knect.tap")
    vim.cmd("KnectTap broker default/orders")
    support.wait_for(function()
      local state = tap._state()
      return state ~= nil and state.tap_id ~= nil
    end, 3000, ":KnectTap")
    assert.are.equal(1, server:request_count("tap.start"))
    vim.cmd("KnectTapStop")
    support.wait_for(function()
      return tap._state() == nil and server:request_count("tap.stop") == 1
    end, 3000, ":KnectTapStop")
  end)
end)
