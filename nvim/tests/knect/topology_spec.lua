local config = require("knect.config")
local rpc = require("knect.rpc")
local support = require("knect.support")
local topology = require("knect.topology")

local socket_path
local server

local function start_server()
  local graph = support.contract("topology.sample.json")
  graph.nodes[#graph.nodes + 1] = {
    id = {
      api_version = "sources.knative.dev/v1",
      kind = "PingSource",
      namespace = "default",
      name = "orders-ping",
    },
    category = "source",
    name = "orders-ping",
    namespace = "default",
    health = "healthy",
    summary = "PingSource orders-ping every minute",
    labels = {},
    annotations = {},
    badges = {},
    uid = "source-uid",
    first_seen = "2026-10-31T12:00:00Z",
    last_seen = "2026-10-31T12:05:00Z",
  }
  graph.stats.node_count = #graph.nodes
  server = support.start_mock(socket_path, {
    ["graph.get_topology"] = graph,
    ["graph.get_node_details"] = support.contract("node_details.sample.json"),
  })
end

describe("knect.topology", function()
before_each(function()
  rpc._reset()
  socket_path = support.temp_socket_path()
  config.setup({ socket_path = socket_path, timeout_ms = 3000, nui = false, telescope = false })
  start_server()
end)

after_each(function()
  topology.close()
  rpc._reset()
  if server ~= nil then
    server:close()
    server = nil
  end
  config.reset()
end)

  it("renders the floating fallback tree and loads node details", function()
    topology.open({ ui = "floating" })
    support.wait_for(function()
      local session = topology._session()
      return session ~= nil and session.topology ~= nil and #session.lines > 0
    end, 3000, "topology render")

    local session = topology._session()
    assert.are.equal("floating", session.adapter.kind)
    local rendered = table.concat(session.lines, "\n")
    assert.is_truthy(rendered:find("Services", 1, true))
    assert.is_truthy(rendered:find("Brokers", 1, true))
    assert.is_truthy(rendered:find("Sources", 1, true))
    assert.is_truthy(rendered:find("orders-ping", 1, true))
    assert.is_truthy(rendered:find("hello", 1, true))
    assert.is_truthy(rendered:find("owns", 1, true))
    assert.are.equal("knect://topology", vim.api.nvim_buf_get_name(session.adapter.tree_buf))

    support.wait_for(function()
      local lines = vim.api.nvim_buf_get_lines(session.adapter.details_buf, 0, -1, false)
      return table.concat(lines, "\n"):find("hello-00001", 1, true) ~= nil
    end, 3000, "node details")

    topology.select_next()
    topology.select_prev()
    topology.close()
    assert.is_nil(topology._session())
    assert.is_false(vim.api.nvim_buf_is_valid(session.adapter.tree_buf))
  end)

  it("uses Nui when it is installed", function()
    local nui = require("knect.ui.nui")
    if not nui.available() then
      pending("Nui.nvim is not installed")
      return
    end
    config.setup({ socket_path = socket_path, timeout_ms = 3000, nui = true })
    topology.open({ ui = "nui" })
    support.wait_for(function()
      local session = topology._session()
      return session ~= nil and session.topology ~= nil
    end, 3000, "nui topology")
    local session = topology._session()
    assert.are.equal("nui", session.adapter.kind)
    local rendered = table.concat(session.lines, "\n")
    assert.is_truthy(rendered:find("hello", 1, true))
    topology.close()
  end)
end)
