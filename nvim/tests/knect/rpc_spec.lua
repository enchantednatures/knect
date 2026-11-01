local config = require("knect.config")
local rpc = require("knect.rpc")
local support = require("knect.support")

local socket_path
local server

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

local function subscribe_handler(params, ctx)
  ctx.respond({ subscription_id = "sub-1" })
end

describe("knect.rpc", function()
before_each(function()
  rpc._reset()
  socket_path = support.temp_socket_path()
  server = nil
  config.setup({
    socket_path = socket_path,
    timeout_ms = 2000,
    reconnect = { initial_ms = 20, max_ms = 100, multiplier = 2 },
  })
end)

after_each(function()
  rpc._reset()
  if server ~= nil then
    server:close()
    server = nil
  end
  config.reset()
end)

  it("correlates out-of-order responses by request id", function()
    server = support.start_mock(socket_path, {
      ["test.echo"] = function(params, ctx)
        ctx.later(params.delay or 0, function()
          ctx.respond(params)
        end)
      end,
    })

    local received = {}
    rpc.request("test.echo", { value = "slow", delay = 60 }, function(err, result)
      assert.is_nil(err)
      received[#received + 1] = result.value
    end)
    rpc.request("test.echo", { value = "fast", delay = 5 }, function(err, result)
      assert.is_nil(err)
      received[#received + 1] = result.value
    end)

    support.wait_for(function()
      return #received == 2
    end, 3000, "both echo responses")
    assert.are.same({ "fast", "slow" }, received)
    assert.are.equal(0, rpc._stats().pending)
    assert.are.equal("uds", rpc._stats().transport)
  end)

  it("calls daemon.status over the UDS JSONL transport", function()
    server = support.start_mock(socket_path, {
      ["daemon.status"] = status_fixture(),
    })
    local result
    local captured
    rpc.request("daemon.status", {}, function(err, value)
      captured = err
      result = value
    end)
    support.wait_for(function()
      return result ~= nil or captured ~= nil
    end, 3000, "daemon.status")
    assert.is_nil(captured)
    assert.are.equal("0.1.0", result.version)
    assert.are.equal(1, result.schema_version)
    assert.are.equal(1, server:request_count("daemon.status"))
  end)

  it("cancels a request when the deadline expires", function()
    server = support.start_mock(socket_path, {
      ["test.hang"] = function() end,
    })

    local captured
    rpc.request("test.hang", {}, function(err)
      captured = err
    end, { timeout_ms = 80 })

    support.wait_for(function()
      return captured ~= nil
    end, 2000, "timeout callback")
    assert.are.equal(-32011, captured.code)
    assert.is_true(captured.data.retryable)
    assert.are.equal(0, rpc._stats().pending)
  end)

  it("dispatches SseEvent lines to subscribers", function()
    server = support.start_mock(socket_path, {
      ["events.subscribe"] = function(params, ctx)
        ctx.respond({ subscription_id = "sub-1" })
        ctx.later(20, function()
          ctx.notify(support.tap_event())
        end)
      end,
    })

    local events = {}
    local handle = rpc.subscribe_events(function(event)
      events[#events + 1] = event
    end)

    support.wait_for(function()
      return #events == 1
    end, 3000, "tap event")
    assert.are.equal("tap.event", events[1].kind)
    assert.are.equal("ce-abc-123", events[1].data.ce_id)
    assert.are.equal("sub-1", handle.id())
    assert.is_true(rpc._stats().subscription_active)
    handle.stop()
  end)

  it("reconnects with backoff and resubscribes the event stream", function()
    local events = {}
    server = support.start_mock(socket_path, {
      ["events.subscribe"] = subscribe_handler,
    })
    local handle = rpc.subscribe_events(function(event)
      events[#events + 1] = event
    end)
    support.wait_for(function()
      return rpc._stats().subscription_active
    end, 2000, "initial subscription")

    server:emit(support.tap_event())
    support.wait_for(function()
      return #events == 1
    end, 2000, "first event")

    server:close()
    server = nil
    support.wait_for(function()
      local stats = rpc._stats()
      return stats.status == "reconnecting" or stats.status == "idle"
    end, 2000, "disconnect")

    server = support.start_mock(socket_path, {
      ["events.subscribe"] = subscribe_handler,
    })
    support.wait_for(function()
      return rpc._stats().status == "connected"
    end, 4000, "reconnect")
    support.wait_for(function()
      return rpc._stats().subscription_active
    end, 4000, "resubscribe")

    server:emit(support.tap_event({ ce_id = "ce-after-reconnect" }))
    support.wait_for(function()
      return #events == 2
    end, 2000, "event after reconnect")
    assert.are.equal("ce-after-reconnect", events[2].data.ce_id)
    handle.stop()
  end)

  it("rejects in-flight requests on close", function()
    server = support.start_mock(socket_path, {
      ["daemon.status"] = status_fixture(),
    })
    local captured
    rpc.connect(function() end)
    rpc.request("daemon.status", {}, function(err)
      captured = err
    end)
    rpc.close()
    support.wait_for(function()
      return captured ~= nil
    end, 2000, "close rejection")
    assert.are.equal(-32012, captured.code)
    assert.are.equal("closed", rpc._stats().status)
  end)
end)
