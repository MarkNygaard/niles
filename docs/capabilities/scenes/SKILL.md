---
name: scenes
description: Scenes, scene, snapshot, cosy, cozy; named lighting setups saved and recalled.
version: 1.0.0
---
# Scenes

```yaml
scenes:
  save: "save this as <name>" | "save the <room> as <name>" | "save this as <name> in the <room>" | "save this scene as <name>" | "save <name>"
  apply: "apply <name>" | "<name> scene" | "scene <name>" | "apply <name> in the <room>"
  apply_by_name: "turn on <name>" | "activate <name>" | "put on <name>" | "set the lights to <name>"
  list: "list my scenes" | "show me my scenes" | "what scenes do I have"
  delete: "delete the <name> scene" | "remove scene <name>"
  exit: "back to normal" | "back to normal in the <room>"
  tools: apply_scene(name, room?)
  notes:
    - the phrasings above are answered without the LLM when they name a saved scene; anything else reaches you
    - apply a scene with apply_scene, however it was asked for ("turn on the scene movie", "movie time"); its description lists the saved scenes
    - no tool saves or deletes a scene; for those, tell the person the phrasing that works
    - an unknown name comes back with the saved ones: offer the closest rather than guessing
    - a scene captures every light (and lamp plug) in the room, or the whole house when no room is said
    - "in the <room>" at the end names the room when the house has that room; otherwise it is part of the name
    - off is captured too: applying turns off lights that were off at save
    - saving under an existing name replaces it; names ignore case and spacing
    - applied lights go into manual mode, off the daylight curve, until "back to normal" or off and on again
    - "turn on <name>" only means a scene when a scene by that name exists; a room or device wins
    - the app shows saved scenes as buttons to apply; saving is by voice or chat only
    - to recreate a past state without a scene: device_state_snapshot_at, then set_device per light
  examples:
    - User: "save the living room as cosy" → saved, scope=living_room
    - User: "save this as cosy in the living room" → saved as cosy, scope=living_room
    - User: "cosy scene" → applies cosy
    - User: "turn on cosy" → applies cosy if it exists
    - User: "turn on the movie scene" → apply_scene(name="movie")
    - User: "cosy in the kitchen, please" → apply_scene(name="cosy", room="kitchen")
    - User: "make it like last night" → device_state_snapshot_at, then set_device
```
