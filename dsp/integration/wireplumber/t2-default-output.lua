-- SPDX-License-Identifier: GPL-3.0-or-later
--
-- Prefer the jack-backed T2 headphone sink over the T2 speaker DSP sink.
-- Both nodes remain ordinary WirePlumber targets; changing the selected
-- default lets WirePlumber's standard follow-default-target policy move
-- existing streams.

local log = Log.open_topic ("s-t2-default-output")

local function isHeadphones (props)
  local alsa_id = props["alsa.id"]
  return alsa_id ~= nil and alsa_id:match ("^t2%-") ~= nil and
      props["device.profile.name"] == "HiFi: Headphones: sink"
end

local function isSpeakers (props)
  local name = props["node.name"]
  return name ~= nil and name:match ("^audio_effect%.t2%-.+%-speakers$") ~= nil
end

SimpleEventHook {
  name = "t2-default-output/select",
  after = {
    "default-nodes/find-selected-default-node",
    "default-nodes/find-stored-default-node",
  },
  before = { "default-nodes/find-best-default-node" },
  interests = {
    EventInterest {
      Constraint { "event.type", "=", "select-default-node" },
      Constraint { "default-node.type", "=", "audio.sink" },
    },
  },
  execute = function (event)
    local available = event:get_data ("available-nodes")
    available = available and available:parse ()
    if not available then
      return
    end

    local headphones = nil
    local speakers = nil
    local selected = event:get_data ("selected-node")
    local selected_is_t2 = false

    for _, props in ipairs (available) do
      if isHeadphones (props) then
        headphones = props["node.name"]
      elseif isSpeakers (props) then
        speakers = props["node.name"]
      end
      if props["node.name"] == selected and
          (isHeadphones (props) or isSpeakers (props)) then
        selected_is_t2 = true
      end
    end

    -- Do not override a user-selected USB, HDMI or Bluetooth output. With no
    -- stored selection, the normal priority policy chooses the initial sink.
    if not selected_is_t2 then
      return
    end

    local target = headphones or speakers
    if target == nil or target == selected then
      return
    end

    log:info ("selecting T2 output " .. target)
    event:set_data ("selected-node", target)
    -- find-best-default-node treats route priorities above 15000 as final.
    event:set_data ("selected-node-priority", 30000)
    event:set_data ("selected-route-priority", 30000)
  end,
}:register ()
