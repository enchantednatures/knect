local config = require("knect.config")
local diff = require("knect.diff")
local rpc = require("knect.rpc")
local support = require("knect.support")

local socket_path
local server
local original_notify

local manifest = table.concat({
  "apiVersion: serving.knative.dev/v1",
  "kind: Revision",
  "metadata:",
  "  name: hello-00001",
  "  namespace: default",
  "spec:",
  "  containers:",
  "    - image: ghcr.io/example/hello:1.0.0",
}, "\n")

local function start_server()
  server = support.start_mock(socket_path, {
    ["graph.get_node_details"] = support.contract("node_details.sample.json"),
    ["mutations.get_diff"] = support.contract("mutation_diff.sample.json"),
  })
end

local function yaml_buffer(text)
  local bufnr = vim.api.nvim_create_buf(false, true)
  vim.api.nvim_buf_set_lines(bufnr, 0, -1, false, vim.split(text, "\n", { plain = true }))
  vim.api.nvim_set_option_value("filetype", "yaml", { buf = bufnr })
  vim.api.nvim_set_current_buf(bufnr)
  return bufnr
end

describe("knect.diff", function()
before_each(function()
  rpc._reset()
  socket_path = support.temp_socket_path()
  original_notify = vim.notify
  config.setup({ socket_path = socket_path, timeout_ms = 3000 })
  start_server()
end)

after_each(function()
  diff.close()
  vim.notify = original_notify
  rpc._reset()
  if server ~= nil then
    server:close()
    server = nil
  end
  config.reset()
end)

  it("opens three panes with live YAML and managedFields attribution", function()
    yaml_buffer(manifest)
    local pending = diff.open()
    assert.is_not_nil(pending)
    support.wait_for(function()
      return diff._current() ~= nil
    end, 3000, "diff render")

    local current = diff._current()
    assert.are.equal(3, #current.wins)
    assert.is_not_nil(current.raw)
    assert.is_truthy(current.raw:find("image", 1, true))
    assert.are.equal("hello-00001", current.id.name)

    local live = table.concat(vim.api.nvim_buf_get_lines(current.live_buf, 0, -1, false), "\n")
    assert.is_truthy(live:find("REDACTED", 1, true))
    local attribution = table.concat(current.attribution, "\n")
    assert.is_truthy(attribution:find("managedFields", 1, true))
    assert.is_truthy(attribution:find("kubectl-client-side-apply", 1, true))
    assert.are.equal(1, server:request_count("graph.get_node_details"))
    assert.are.equal(1, server:request_count("mutations.get_diff"))

    diff.close()
    assert.is_nil(diff._current())
    assert.is_false(vim.api.nvim_buf_is_valid(current.live_buf))
  end)

  it("fails clearly when the buffer is not Kubernetes YAML", function()
    local bufnr = vim.api.nvim_create_buf(false, true)
    vim.api.nvim_buf_set_lines(bufnr, 0, -1, false, { "just some text", "not: [valid" })
    vim.api.nvim_set_option_value("filetype", "text", { buf = bufnr })
    vim.api.nvim_set_current_buf(bufnr)

    local notified
    vim.notify = function(message, level)
      notified = { message = message, level = level }
    end

    assert.is_nil(diff.open())
    assert.is_not_nil(notified)
    assert.is_truthy(notified.message:find("not a Kubernetes YAML manifest", 1, true))
    assert.are.equal(vim.log.levels.ERROR, notified.level)
  end)

  it("parses an alternative metadata indentation", function()
    local identity = require("knect.util").parse_manifest(
      "apiVersion: eventing.knative.dev/v1\nkind: Trigger\nmetadata:\n    name: orders-to-hello\n    namespace: default\n"
    )
    assert.are.equal("eventing.knative.dev/v1", identity.api_version)
    assert.are.equal("Trigger", identity.kind)
    assert.are.equal("orders-to-hello", identity.name)
    assert.are.equal("default", identity.namespace)
  end)
end)
