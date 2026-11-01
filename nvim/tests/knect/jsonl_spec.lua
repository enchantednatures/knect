local jsonl = require("knect.jsonl")

describe("knect.jsonl", function()
  it("buffers partial chunks and decodes multiple lines", function()
    local messages = {}
    local errors = {}
    local decoder = jsonl.decoder(function(message)
      messages[#messages + 1] = message
    end, function(line, err)
      errors[#errors + 1] = { line = line, err = err }
    end)

    decoder:feed('{"a":1}\n{"b"')
    assert.are.equal(1, #messages)
    assert.are.equal('{"b"', decoder:pending())

    decoder:feed(':2}\r\n\n')
    assert.are.equal(2, #messages)
    assert.are.equal(2, messages[2].b)
    assert.are.equal(0, #errors)
    assert.are.equal("", decoder:pending())
  end)

  it("reports malformed lines and keeps decoding", function()
    local messages = {}
    local errors = {}
    local decoder = jsonl.decoder(function(message)
      messages[#messages + 1] = message
    end, function(line, err)
      errors[#errors + 1] = { line = line, err = err }
    end)

    decoder:feed("not-json\n{\"ok\":true}\n")
    assert.are.equal(1, #errors)
    assert.are.equal("not-json", errors[1].line)
    assert.are.equal(1, #messages)
    assert.is_true(messages[1].ok)
  end)

  it("flushes a trailing line at the end of the stream", function()
    local messages = {}
    local decoder = jsonl.decoder(function(message)
      messages[#messages + 1] = message
    end, function() end)
    decoder:feed('{"tail":true}')
    assert.are.equal(0, #messages)
    decoder:finish()
    assert.are.equal(1, #messages)
    assert.is_true(messages[1].tail)
  end)

  it("encodes empty parameter objects as JSON objects", function()
    assert.are.equal("{}", jsonl.encode(vim.empty_dict()))
    assert.are.equal("[]", jsonl.encode({}))
  end)
end)
