local diagnostics = require("knect.diagnostics")
local yaml = require("knect.yaml")

local SEV = vim.diagnostic.severity

local MANIFEST = table.concat({
  "apiVersion: serving.knative.dev/v1",
  "kind: Revision",
  "metadata:",
  "  name: hello-00002",
  "  namespace: default",
  "spec:",
  "  containers:",
  "    - image: ghcr.io/example/hello:2.0.0",
  "      envFrom:",
  "        - secretRef:",
  "            name: redis-secret",
  "      volumeMounts:",
  "        - name: redis-data",
  "          mountPath: /data",
}, "\n")

local function scan()
  local result = assert(yaml.scan(MANIFEST))
  return result
end

local function policy(overrides)
  local record = {
    id = "policy-1",
    name = "require-labels",
    namespace = vim.NIL,
    kind = "kyverno",
    resource = vim.NIL,
    severity = "medium",
    result = "warn",
    message = "missing label",
    observed_at = vim.NIL,
    details = {},
  }
  for key, value in pairs(overrides or {}) do
    record[key] = value
  end
  return record
end

local function storage_record()
  return {
    pvc = {
      api_version = "v1",
      kind = "PersistentVolumeClaim",
      namespace = "default",
      name = "redis-data",
    },
    volume = "redis-data",
    storage_class = "ceph-rbd",
    access_modes = { "read_write_once" },
    capacity = "20Gi",
    phase = "Bound",
    mounted_by = {
      {
        api_version = "serving.knative.dev/v1",
        kind = "Revision",
        namespace = "default",
        name = "hello-00001",
      },
      {
        api_version = "serving.knative.dev/v1",
        kind = "Revision",
        namespace = "default",
        name = "hello-00002",
      },
    },
    warnings = {
      {
        code = "rwo_multi_mount",
        message = "ReadWriteOnce claim is mounted by more than one replica",
        severity = "high",
        conflict = {
          pvc = {
            api_version = "v1",
            kind = "PersistentVolumeClaim",
            namespace = "default",
            name = "redis-data",
          },
          access_mode = "read_write_once",
          max_scale = 2,
          reason = "RWO volumes cannot be attached to multiple nodes",
          affected = {},
        },
      },
    },
  }
end

describe("knect.diagnostics", function()
  after_each(function()
    diagnostics.clear_all()
  end)

  it("maps ValidationResult violations and warnings to diagnostics", function()
    local items = diagnostics.from_validation({
      valid = false,
      mutated_manifest = vim.NIL,
      violations = {
        policy({ result = "fail", severity = "high", details = { path = "/spec/containers/0/image" } }),
      },
      warnings = {
        { code = "admission_warning", message = "preflight warning", severity = "low", resource = vim.NIL },
      },
    }, scan())

    assert.are.equal(2, #items)
    assert.are.equal("kyverno: missing label", items[1].message)
    assert.are.equal(SEV.ERROR, items[1].severity)
    assert.are.equal(7, items[1].lnum)
    assert.are.equal("knect", items[1].source)
    assert.are.equal("preflight warning", items[2].message)
    assert.are.equal(SEV.INFO, items[2].severity)
  end)

  it("maps detail policies and RWO storage conflicts to the volumeMount line", function()
    local items = diagnostics.from_details({
      policies = {
        policy({ result = "warn", severity = "medium" }),
      },
      warnings = {},
      storage = { storage_record() },
    }, scan())

    assert.are.equal(2, #items)
    assert.are.equal("kyverno: missing label", items[1].message)
    assert.are.equal(SEV.WARN, items[1].severity)
    assert.is_truthy(items[2].message:find("RWO redis-data", 1, true))
    assert.are.equal(SEV.ERROR, items[2].severity)
    assert.are.equal(11, items[2].lnum)
  end)

  it("deduplicates merged diagnostics", function()
    local one = policy({ result = "fail" })
    local merged = diagnostics.merge({
      diagnostics.from_validation({ violations = { one } }, scan()),
      diagnostics.from_validation({ violations = { one } }, scan()),
    })
    assert.are.equal(1, #merged)
  end)

  it("anchors validation RWO warnings to the volumeMount line", function()
    local items = diagnostics.from_validation({
      warnings = {
        { code = "rwo_multi_mount", message = "mounted twice", severity = "high", resource = vim.NIL },
      },
    }, scan())
    assert.are.equal(1, #items)
    assert.are.equal(11, items[1].lnum)
    assert.are.equal(SEV.ERROR, items[1].severity)
    assert.is_truthy(items[1].message:find("RWO mounted twice", 1, true))
  end)

  it("publishes and clears diagnostics per buffer in its own namespace", function()
    local first = vim.api.nvim_create_buf(false, true)
    local second = vim.api.nvim_create_buf(false, true)
    vim.api.nvim_buf_set_lines(first, 0, -1, false, { "apiVersion: x", "kind: Y" })
    vim.api.nvim_buf_set_lines(second, 0, -1, false, { "apiVersion: x", "kind: Y" })

    local items = diagnostics.from_validation({ violations = { policy({ result = "fail" }) } }, scan())
    diagnostics.publish(first, items)
    diagnostics.publish(second, items)

    assert.are.equal(1, #vim.diagnostic.get(first, { namespace = diagnostics.namespace() }))
    assert.are.equal(1, #vim.diagnostic.get(second, { namespace = diagnostics.namespace() }))

    diagnostics.clear(first)
    assert.are.equal(0, #vim.diagnostic.get(first, { namespace = diagnostics.namespace() }))
    assert.are.equal(1, #vim.diagnostic.get(second, { namespace = diagnostics.namespace() }))
  end)
end)
