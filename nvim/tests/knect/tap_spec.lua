local config = require("knect.config")
local rpc = require("knect.rpc")
local support = require("knect.support")
local tap = require("knect.tap")

local socket_path
local server

local function start_server()
  server = support.start_mock(socket_path, {
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
end

describe("knect.tap", function()
before_each(function()
  rpc._reset()
  socket_path = support.temp_socket_path()
  config.setup({ socket_path = socket_path, timeout_ms = 3000, telescope = false })
  start_server()
end)

after_each(function()
  tap.close()
  rpc._reset()
  if server ~= nil then
    server:close()
    server = nil
  end
  config.reset()
end)

  it("streams tap.event entries into a scratch buffer", function()
    local state = tap.open({ kind = "broker", namespace = "default", name = "orders" })
    assert.is_not_nil(state)
    support.wait_for(function()
      return state.tap_id ~= nil
    end, 3000, "tap.start")
    support.wait_for(function()
      return rpc._stats().subscription_active
    end, 3000, "event subscription")
    assert.are.equal(1, server:request_count("tap.start"))
    assert.are.equal(1, server:request_count("events.subscribe"))

    server:emit(support.tap_event())
    support.wait_for(function()
      return state.event_count >= 1
    end, 3000, "tap event append")

    local text = table.concat(vim.api.nvim_buf_get_lines(state.buf, 0, -1, false), "\n")
    assert.is_truthy(text:find("ce-abc-123", 1, true))
    assert.is_truthy(text:find("dev.knative.sources.ping", 1, true))
    assert.is_truthy(text:find("order-42", 1, true))
    assert.is_truthy(text:find("payload", 1, true))
    assert.are.equal("knect://tap/1", vim.api.nvim_buf_get_name(state.buf))

    tap.close()
    support.wait_for(function()
      return server:request_count("tap.stop") == 1
    end, 3000, "tap.stop")
    assert.is_false(vim.api.nvim_buf_is_valid(state.buf))
    assert.is_nil(tap._state())
  end)

  it("stops the tap when its buffer is wiped", function()
    local state = tap.open({ kind = "trigger", namespace = "default", name = "orders-to-hello" })
    support.wait_for(function()
      return state.tap_id ~= nil
    end, 3000, "tap.start")
    vim.api.nvim_buf_delete(state.buf, { force = true })
    support.wait_for(function()
      return server:request_count("tap.stop") == 1
    end, 3000, "tap.stop after buffer wipe")
    assert.is_nil(tap._state())
  end)

  it("parses command arguments", function()
    local target, err = tap._parse_target({ "trigger", "default/orders-to-hello" })
    assert.is_nil(err)
    assert.are.equal("trigger", target.kind)
    assert.are.equal("default", target.namespace)
    assert.are.equal("orders-to-hello", target.name)

    local bad, bad_err = tap._parse_target({ "channel", "default/orders" })
    assert.is_nil(bad)
    assert.is_truthy(bad_err:find("broker or trigger", 1, true))
  end)
end)
