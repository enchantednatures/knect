local config = require("knect.config")

describe("knect commands", function()
  it("registers every user command", function()
    for _, name in ipairs({
      "Knect",
      "KnectServices",
      "KnectBrokers",
      "KnectViolations",
      "KnectDiff",
      "KnectTap",
      "KnectTapStop",
    }) do
      assert.are.equal(2, vim.fn.exists(":" .. name), name .. " should be registered")
    end
  end)

  it("re-registers commands safely when setup is called again", function()
    local plugin = require("knect")
    assert.has_no.errors(function()
      plugin.setup({ timeout_ms = 4000 })
      plugin.setup({ timeout_ms = 5000 })
    end)
    assert.are.equal(5000, config.get().timeout_ms)
    assert.are.equal(2, vim.fn.exists(":Knect"))
  end)

  it("exposes guarded optional integrations", function()
    local nui = require("knect.ui.nui")
    local telescope = require("knect.ui.telescope")
    assert.has_no.errors(function()
      nui.available()
      telescope.available()
    end)

    config.setup({ telescope = false })
    assert.is_false(telescope.available())
    config.setup({ nui = false })
    assert.is_false(config.get().nui)
  end)

  it("degrades when an optional integration is broken", function()
    local nui = require("knect.ui.nui")
    local saved_split = package.loaded["nui.split"]
    local saved_nui = package.loaded["nui"]
    package.loaded["nui.split"] = { broken = true }
    package.loaded["nui"] = { Split = "not callable" }
    assert.is_false(nui.available())
    package.loaded["nui.split"] = saved_split
    package.loaded["nui"] = saved_nui
  end)

  it("runs the health check without raising", function()
    local health = require("knect.health")
    assert.has_no.errors(function()
      health.check()
    end)
  end)
end)
