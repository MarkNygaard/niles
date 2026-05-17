# ESPHome satellite firmware

Reference ESPHome configuration for the reSpeaker XVF3800 + XIAO ESP32-S3 satellites. Populated in **Phase 3** (Tier 0/1 voice loop) per [ARCHITECTURE.md](../../ARCHITECTURE.md#satellite-firmware).

Will include:
- Wake-word detection (microWakeWord)
- I2S audio capture from the XVF3800 DSP
- Wyoming-protocol streaming to the Niles service
- LED ring + button mappings
