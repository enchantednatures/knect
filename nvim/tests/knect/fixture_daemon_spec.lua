local config = require("knect.config")
local rpc = require("knect.rpc")

local uv = vim.uv or vim.loop

local fixture_path = vim.env.KNECT_FIXTURE_SOCKET or "/tmp/knectd-fixture.sock"

describe("knect fixture daemon", function()
  it("answers daemon.status over the UDS JSONL transport", function()
    if uv.fs_stat(fixture_path) == nil then
      pending("fixture daemon not available at " .. fixture_path)
      return
    end

    rpc._reset()
    config.setup({ socket_path = fixture_path, timeout_ms = 5000, transport = "uds" })
    local status
    local captured
    rpc.request("daemon.status", {}, function(err, result)
      captured = err
      status = result
    end)
    assert.is_true(vim.wait(5000, function()
      return status ~= nil or captured ~= nil
    end, 10))
    rpc.close()
    config.reset()

    assert.is_nil(captured)
    assert.are.equal(1, status.schema_version)
    assert.is_not_nil(status.version)
    assert.is_not_nil(status.context)

    rpc._reset()
    config.setup({ socket_path = fixture_path, timeout_ms = 5000, transport = "uds" })
    local graph
    local graph_error
    rpc.request("graph.get_topology", {}, function(err, result)
      graph_error = err
      graph = result
    end)
    assert.is_true(vim.wait(5000, function()
      return graph ~= nil or graph_error ~= nil
    end, 10))
    rpc.close()
    config.reset()

    assert.is_nil(graph_error)
    assert.are.equal(1, graph.schema_version)
    assert.is_not_nil(graph.nodes)
  end)
end)
