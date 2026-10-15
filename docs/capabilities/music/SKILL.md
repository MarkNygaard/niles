---
name: music
description: Music, song, songs, radio, station, Spotify, Sonos, speaker, speakers, volume, louder, quieter, playlist, album, artist, play, playing, pause, skip, mute, unmute, TV, television, telly, Netflix, YouTube, HDMI.
version: 1.0.0
---
# Music, speakers and TV

```yaml
music:
  play: "play <artist|song|album|playlist> in the <room>" | "play <thing> on Spotify" | "play <thing>"
  radio: "play the radio" | "play <station> on the radio in the <room>"
  spread: "play it in the <room> too" | "play it everywhere"
  resume: "play the <room>" | "resume music in the <room>"
  pause: "pause the <room>" | "stop the music" | "stop the music in the <room>"
  skip: "next song in the <room>" | "<room> next" | "skip" | "previous" | "go back"
  volume: "set the <room> volume to <n>%" | "volume up in the <room>" | "<room> volume down"   # ±10
  tv: "turn on the TV" | "TV off" | "put on Netflix on the TV" | "switch the TV to HDMI 2" | "pause the TV"
  tv_volume: "set the TV volume to <n>" | "turn the TV up" | "TV volume down" | "mute the TV" | "unmute the TV"   # ±10
  tools:
    play_music: room + query; Sonos favorite, radio station, or Spotify artist/song/album/playlist; from_spotify when they said Spotify
    play_radio: room + station; no station = what that room played last; favorites, then TuneIn
    play_elsewhere: room to add, or none for every room; from_room = where it plays now
    pause_music: room optional; without one, stops the music wherever it plays
    resume_music / skip_track (back=true for previous) / music_volume (level 0-100 or change ±)
    tv_power / tv_open_app / tv_input / tv_playback / tv_message / tv_status: the LG TV
    tv_volume: action set (with level 0-100) / up / down / mute / unmute; the TV's own speakers
  notes:
    - play_music and play_radio list choices when several match; call again with spotify_uri+name or station_id
    - the whole room plays together and replaces what was playing, the TV included
    - no room said: the room of the satellite; "play X" with no room in the app's chat has no room, so ask
    - "stop" with no timer to stop means the music; the TV's sound is not the music
    - in a room with a Sonos the TV's sound plays through it; its volume is music_volume. TV sound is never spread
    - in a room with no Sonos the TV's own speakers are the sound: music_volume turns them, and so does tv_volume
    - only the TV mutes; a Sonos has no mute
    - TV apps and inputs are matched by name against what the TV lists (Netflix, YouTube, DR TV, TV 2 Play, Disney+)
    - tv_message shows a short text on the screen over whatever is on
  examples:
    - User: "play Gravity by John Mayer in the kitchen" → play_music room=kitchen query="Gravity by John Mayer"
    - User: "put on P1 in the bedroom" → play_radio room=bedroom station=P1
    - User: "a bit louder" → music_volume change=+10 in this room
    - User: "put YouTube on" → tv_open_app app=YouTube
```
