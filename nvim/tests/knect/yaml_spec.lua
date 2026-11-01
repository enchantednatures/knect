local config = require("knect.config")
local diagnostics = require("knect.diagnostics")
local rpc = require("knect.rpc")
local support = require("knect.support")
local yaml = require("knect.yaml")

local socket_path
local server
local created_buffers = {}

local KNATIVE_MANIFEST = {
  "apiVersion: serving.knative.dev/v1",
  "kind: Revision",
  "metadata:",
  "  name: hello-00002",
  "  namespace: default",
  "spec:",
  "  containers:",
  "    - name: user-container",
  "      image: ghcr.io/example/hello:2.0.0",
  "      envFrom:",
  "        - secretRef:",
  "            name: redis-secret",
  "      volumeMounts:",
  "        - name: redis-data",
  "          mountPath: /data",
}

local NON_KNATIVE_MANIFEST = {
  "apiVersion: apps/v1",
  "kind: Deployment",
  "metadata:",
  "  name: hello",
  "  namespace: default",
  "spec:",
  "  template:",
  "    spec:",
  "      containers:",
  "        - image: nginx:1.25",
}

local function details_fixture()
  local details = support.contract("node_details.sample.json")
  details.id.name = "hello-00002"
  details.name = "hello-00002"
  details.summary = "Revision hello-00002 ready=2"
  details.metrics.replicas = 2
  details.metrics.ready_replicas = 2
  details.yaml = details.yaml .. table.concat({
    "status:",
    "  latestReadyRevisionName: hello-00002",
    "  traffic:",
    "  - revisionName: hello-00001",
    "    percent: 90",
    "    tag: stable",
    "  - revisionName: hello-00002",
    "    percent: 10",
    "    tag: canary",
    "",
  }, "\n")
  return details
end

local function validation_fixture()
  return {
    valid = false,
    mutated_manifest = vim.NIL,
    violations = {
      {
        id = "validation-1",
        name = "require-labels",
        namespace = vim.NIL,
        kind = "kyverno",
        resource = vim.NIL,
        severity = "high",
        result = "fail",
        message = "missing label",
        observed_at = vim.NIL,
        details = {},
      },
    },
    warnings = {
      {
        code = "kyverno_mutation",
        message = "preflight warning",
        severity = "medium",
        resource = vim.NIL,
      },
    },
  }
end

local function start_server(overrides)
  if server ~= nil then
    server:close()
    server = nil
  end
  local responses = {
    ["graph.get_node_details"] = details_fixture(),
    ["policy.validate_manifest"] = validation_fixture(),
  }
  for method, value in pairs(overrides or {}) do
    responses[method] = value
  end
  server = support.start_mock(socket_path, responses)
end

local function make_buffer(lines)
  local bufnr = vim.api.nvim_create_buf(false, true)
  vim.api.nvim_buf_set_lines(bufnr, 0, -1, false, lines)
  vim.api.nvim_set_option_value("filetype", "yaml", { buf = bufnr })
  created_buffers[#created_buffers + 1] = bufnr
  return bufnr
end

local function extmarks(bufnr)
  return vim.api.nvim_buf_get_extmarks(bufnr, yaml.namespace(), 0, -1, { details = true })
end

local function text_by_line(bufnr)
  local lines = {}
  for _, mark in ipairs(extmarks(bufnr)) do
    local row = mark[2]
    local details = mark[4]
    local chunks = {}
    for _, chunk in ipairs(details.virt_text or {}) do
      chunks[#chunks + 1] = chunk[1]
    end
    lines[row] = table.concat(chunks)
  end
  return lines
end

local function knect_diagnostics(bufnr)
  return vim.diagnostic.get(bufnr, { namespace = diagnostics.namespace() })
end

local function diagnostic_text(bufnr)
  local parts = {}
  for _, item in ipairs(knect_diagnostics(bufnr)) do
    parts[#parts + 1] = item.message
  end
  return table.concat(parts, "\n")
end

describe("knect.yaml", function()
  before_each(function()
    rpc._reset()
    socket_path = support.temp_socket_path()
    config.setup({ socket_path = socket_path, timeout_ms = 3000, debounce_ms = 40 })
    start_server()
    yaml.setup({ auto_attach = true, debounce_ms = 40 })
  end)

  after_each(function()
    yaml.setup({ auto_attach = false })
    for _, bufnr in ipairs(created_buffers) do
      if vim.api.nvim_buf_is_valid(bufnr) then
        pcall(vim.api.nvim_buf_delete, bufnr, { force = true })
      end
    end
    created_buffers = {}
    rpc._reset()
    if server ~= nil then
      server:close()
      server = nil
    end
    config.reset()
  end)

  it("scans Knative identity and hint lines without a YAML parser", function()
    local text = table.concat({
      "apiVersion: serving.knative.dev/v1",
      "kind: Service",
      "metadata:",
      "  name: hello",
      "  namespace: default",
      "spec:",
      "  template:",
      "    spec:",
      "      containers:",
      "        - image: ghcr.io/example/hello:1.0.0",
      "          envFrom:",
      "            - secretRef:",
      "                name: first",
      "        - image: ghcr.io/example/sidecar:1.0.0",
      "          volumeMounts:",
      "            - name: data",
      "            - name: cache",
    }, "\n")

    local scan, err = yaml.scan(text)
    assert.is_nil(err)
    assert.is_true(scan.knative)
    assert.are.equal("serving.knative.dev/v1", scan.api_version)
    assert.are.equal("Service", scan.kind)
    assert.are.equal("hello", scan.name)
    assert.are.equal("default", scan.namespace)
    assert.are.equal(2, #scan.fields.image)
    assert.are.equal(1, #scan.fields.envFrom)
    assert.are.equal(1, #scan.fields.secretRef)
    assert.are.equal(1, #scan.fields.volumeMounts)
    assert.are.equal(10, scan.fields.image[1].line)
    assert.are.equal(14, scan.fields.image[2].line)
    assert.are.equal(12, scan.fields.secretRef[1].line)
    assert.are.equal(15, scan.fields.volumeMounts[1].line)

    local eventing = yaml.scan("apiVersion: eventing.knative.dev/v1\nkind: Broker\nmetadata:\n  name: orders\n")
    assert.is_true(eventing.knative)

    local plain = yaml.scan(table.concat(NON_KNATIVE_MANIFEST, "\n"))
    assert.is_false(plain.knative)
  end)

  it("auto-attaches and renders live hints for every hint field", function()
    local bufnr = make_buffer(KNATIVE_MANIFEST)
    support.wait_for(function()
      return #extmarks(bufnr) >= 4 and server:request_count("graph.get_node_details") == 1
    end, 4000, "live extmarks")

    local by_line = text_by_line(bufnr)
    assert.are.equal(4, #extmarks(bufnr))
    for _, mark in ipairs(extmarks(bufnr)) do
      assert.are.equal("eol", mark[4].virt_text_pos)
    end
    assert.is_truthy(by_line[8]:find("rev 00002 · 2 pods", 1, true))
    assert.is_truthy(by_line[8]:find("traffic 90% hello-00001 (stable)", 1, true))
    assert.is_truthy(by_line[8]:find("kyverno:", 1, true))
    assert.is_truthy(by_line[9]:find("redis-master healthy", 1, true))
    assert.is_truthy(by_line[10]:find("redis-master healthy", 1, true))
    assert.is_truthy(by_line[12]:find("RWO redis-data", 1, true))
    assert.is_nil(by_line[0])
    assert.are.equal(1, server:request_count("graph.get_node_details"))
    assert.are.equal(0, server:request_count("policy.validate_manifest"))
  end)

  it("auto-attaches eventing manifests", function()
    local bufnr = make_buffer({
      "apiVersion: eventing.knative.dev/v1",
      "kind: Trigger",
      "metadata:",
      "  name: orders-to-hello",
      "  namespace: default",
      "spec:",
      "  broker: default",
      "  subscriber:",
      "    ref:",
      "      kind: Service",
      "      name: hello",
    })
    support.wait_for(function()
      return server:request_count("graph.get_node_details") == 1
    end, 3000, "eventing details")
    local state = yaml._state(bufnr)
    assert.is_not_nil(state)
    assert.is_true(state.active)
    assert.are.equal("eventing.knative.dev/v1", state.scan.api_version)
  end)

  it("ignores non-Knative YAML buffers", function()
    local bufnr = make_buffer(NON_KNATIVE_MANIFEST)
    vim.wait(150)
    assert.are.equal(0, #extmarks(bufnr))
    assert.are.equal(0, #knect_diagnostics(bufnr))
    assert.are.equal(0, server:request_count("graph.get_node_details"))
    assert.are.equal(0, server:request_count("policy.validate_manifest"))
    local state = yaml._state(bufnr)
    assert.is_not_nil(state)
    assert.is_false(state.active)
  end)

  it("debounces policy.validate_manifest on buffer changes", function()
    local bufnr = make_buffer(KNATIVE_MANIFEST)
    support.wait_for(function()
      return server:request_count("graph.get_node_details") == 1
    end, 3000, "initial details")
    vim.wait(120)

    local before = server:request_count("policy.validate_manifest")
    for _ = 1, 3 do
      vim.api.nvim_exec_autocmds("TextChanged", { buffer = bufnr })
    end
    assert.are.equal(before, server:request_count("policy.validate_manifest"))

    support.wait_for(function()
      return server:request_count("policy.validate_manifest") == before + 1
    end, 3000, "debounced validation")
    vim.wait(120)
    assert.are.equal(before + 1, server:request_count("policy.validate_manifest"))

    support.wait_for(function()
      local rendered = diagnostic_text(bufnr)
      return rendered:find("kyverno: missing label", 1, true) ~= nil
        and rendered:find("preflight warning", 1, true) ~= nil
    end, 3000, "validation diagnostics")
    support.wait_for(function()
      local rendered = text_by_line(bufnr)[8] or ""
      return rendered:find("kyverno_mutation: preflight warning", 1, true) ~= nil
    end, 3000, "validation extmark refresh")

    local after_change = server:request_count("policy.validate_manifest")
    vim.api.nvim_exec_autocmds("BufWritePost", { buffer = bufnr })
    support.wait_for(function()
      return server:request_count("policy.validate_manifest") == after_change + 1
    end, 3000, "save validation")
  end)

  it("does not duplicate extmarks or requests when attached repeatedly", function()
    local bufnr = make_buffer(KNATIVE_MANIFEST)
    support.wait_for(function()
      return #extmarks(bufnr) == 4
    end, 3000, "initial extmarks")

    yaml.attach(bufnr)
    yaml.attach(bufnr)
    vim.wait(100)
    assert.are.equal(4, #extmarks(bufnr))
    assert.are.equal(1, server:request_count("graph.get_node_details"))
  end)

  it("clears hints and diagnostics on detach and buffer deletion", function()
    local bufnr = make_buffer(KNATIVE_MANIFEST)
    support.wait_for(function()
      return #extmarks(bufnr) == 4 and #knect_diagnostics(bufnr) > 0
    end, 4000, "initial hints and diagnostics")

    yaml.detach(bufnr)
    assert.are.equal(0, #extmarks(bufnr))
    assert.are.equal(0, #knect_diagnostics(bufnr))
    assert.is_nil(yaml._state(bufnr))

    local reattached = make_buffer(KNATIVE_MANIFEST)
    support.wait_for(function()
      return #extmarks(reattached) == 4 and #knect_diagnostics(reattached) > 0
    end, 4000, "reattached hints")
    vim.api.nvim_buf_delete(reattached, { force = true })
    assert.is_nil(yaml._state(reattached))
  end)

  it("clears hints when the buffer stops being Knative", function()
    local bufnr = make_buffer(KNATIVE_MANIFEST)
    support.wait_for(function()
      return #extmarks(bufnr) == 4
    end, 3000, "initial extmarks")

    vim.api.nvim_buf_set_lines(bufnr, 0, -1, false, NON_KNATIVE_MANIFEST)
    vim.api.nvim_exec_autocmds("TextChanged", { buffer = bufnr })
    support.wait_for(function()
      return #extmarks(bufnr) == 0 and #knect_diagnostics(bufnr) == 0
    end, 3000, "cleared hints")
    local state = yaml._state(bufnr)
    assert.is_not_nil(state)
    assert.is_false(state.active)
  end)

  it("handles an absent node without raising or rendering stale hints", function()
    start_server({
      ["graph.get_node_details"] = function(_, ctx)
        ctx.respond_error({
          code = -32001,
          message = "resource not found",
          data = { retryable = false },
        })
      end,
    })
    local bufnr = make_buffer(KNATIVE_MANIFEST)
    support.wait_for(function()
      return server:request_count("graph.get_node_details") == 1
    end, 3000, "absent node response")
    vim.wait(80)
    assert.are.equal(0, #extmarks(bufnr))
  end)
end)
