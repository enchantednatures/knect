local config = require("knect.config")
local rpc = require("knect.rpc")
local support = require("knect.support")

local function status_fixture()
  return {
    version = "0.1.0",
    schema_version = 1,
    context = "test",
    generation = 1,
    watchers = {},
    uptime_secs = 1,
    actions_enabled = false,
    tap_count = 0,
    started_at = vim.NIL,
    cache_enabled = false,
    last_resync = vim.NIL,
  }
end

local socket_path
local http_server

describe("knect.http fallback", function()
before_each(function()
  rpc._reset()
  socket_path = support.temp_socket_path()
  http_server = nil
  config.setup({ socket_path = socket_path, timeout_ms = 3000 })
end)

after_each(function()
  rpc._reset()
  if http_server ~= nil then
    http_server:close()
    http_server = nil
  end
  config.reset()
end)

  it("posts JSON-RPC through vim.system and curl", function()
    if type(vim.system) ~= "function" then
      pending("vim.system is unavailable")
      return
    end
    http_server = support.start_http_mock({
      ["daemon.status"] = status_fixture(),
    })
    config.setup({
      socket_path = socket_path,
      transport = "http",
      http_addr = http_server.url,
      timeout_ms = 3000,
    })

    local result
    local captured
    rpc.request("daemon.status", {}, function(err, value)
      captured = err
      result = value
    end)

    support.wait_for(function()
      return result ~= nil or captured ~= nil
    end, 5000, "HTTP response")
    assert.is_nil(captured)
    assert.are.equal("0.1.0", result.version)
    assert.are.equal(1, http_server:request_count("daemon.status"))
    assert.are.equal("http", rpc._stats().transport)
  end)

  it("falls back to HTTP when the socket is unreachable", function()
    if type(vim.system) ~= "function" then
      pending("vim.system is unavailable")
      return
    end
    http_server = support.start_http_mock({
      ["daemon.status"] = status_fixture(),
    })
    config.setup({
      socket_path = socket_path .. ".missing",
      transport = "auto",
      http_addr = http_server.url,
      timeout_ms = 3000,
    })

    local result
    local captured
    rpc.request("daemon.status", {}, function(err, value)
      captured = err
      result = value
    end)

    support.wait_for(function()
      return result ~= nil or captured ~= nil
    end, 5000, "HTTP fallback response")
    assert.is_nil(captured)
    assert.are.equal("0.1.0", result.version)
    assert.are.equal("http", rpc._stats().transport)
  end)

  it("reports a clear error when curl is unavailable", function()
    if type(vim.system) ~= "function" then
      pending("vim.system is unavailable")
      return
    end
    config.setup({
      socket_path = socket_path,
      transport = "http",
      http_addr = "http://127.0.0.1:1",
      curl = "knect-missing-curl-binary",
      timeout_ms = 1000,
    })
    local captured
    rpc.request("daemon.status", {}, function(err)
      captured = err
    end)
    support.wait_for(function()
      return captured ~= nil
    end, 2000, "curl error")
    assert.is_truthy(tostring(captured.message):find("curl", 1, true))
  end)
end)
