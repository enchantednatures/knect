-- Newline-delimited JSON framing.
--
-- The daemon writes exactly one JSON object per line. Chunks from a pipe are
-- arbitrary, so the decoder buffers partial lines and tolerates CRLF.

local M = {}

--- Encode one value as a single JSON line (without the trailing newline).
---@param value any
---@return string|nil
---@return string|nil
function M.encode(value)
  local ok, encoded = pcall(vim.json.encode, value)
  if not ok then
    return nil, tostring(encoded)
  end
  return encoded
end

--- Decode one JSON line.
---@param line string
---@return any|nil
---@return string|nil
function M.decode(line)
  local ok, value = pcall(vim.json.decode, line)
  if not ok then
    return nil, tostring(value)
  end
  return value
end

--- Create a streaming line decoder.
---
--- `on_message` receives every decoded JSON value. `on_error` receives the
--- offending line and the decoder error. Both are called synchronously from
--- `feed`, which itself is safe to call from a libuv read callback.
---@param on_message fun(value:any)
---@param on_error fun(line:string, err:string)
---@return table
function M.decoder(on_message, on_error)
  local pending = ""

  local decoder = {}

  function decoder:push(line)
    if line:sub(-1) == "\r" then
      line = line:sub(1, -2)
    end
    if line:match("^%s*$") then
      return
    end
    local value, err = M.decode(line)
    if value == nil then
      on_error(line, err or "invalid JSON")
      return
    end
    on_message(value)
  end

  --- Feed an arbitrary chunk from the socket.
  ---@param chunk string
  function decoder:feed(chunk)
    if type(chunk) ~= "string" or chunk == "" then
      return
    end
    pending = pending .. chunk
    while true do
      local newline = pending:find("\n", 1, true)
      if newline == nil then
        break
      end
      local line = pending:sub(1, newline - 1)
      pending = pending:sub(newline + 1)
      self:push(line)
    end
  end

  --- Decode any trailing bytes not terminated by a newline.
  function decoder:finish()
    if pending ~= "" then
      local line = pending
      pending = ""
      self:push(line)
    end
  end

  --- Bytes held back waiting for a newline.
  ---@return string
  function decoder:pending()
    return pending
  end

  function decoder:reset()
    pending = ""
  end

  return decoder
end

return M
