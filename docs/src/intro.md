# Introduction

**Niles** (NYE-uls), *Neural Intelligence, Lightweight Edge System*, is an open-source, AI-first home automation system designed to replace Home Assistant for users who want sub-second voice interactions, code/config over UI, and a voice assistant with real LLM intelligence as a first-class citizen.

It runs locally on small hardware, bridges Zigbee devices via Zigbee2MQTT, and exposes a voice interface through ESP32-S3 satellites (a reSpeaker XVF3800 with Niles's own firmware and an on-device "Niles" wake word). Commands go through a regex router first and a fast LLM after it, with an optional stronger model above that.

Living with Niles: start with [Talking to Niles](talking.md) for what you can say, and [Lighting and scenes](lighting.md) for what it does with your lights. The rest of **Using Niles** covers the app: the chat, what Niles saves for you, approvals, automations, add-ons and accounts, and who can see what.

Building or running Niles: the canonical architecture spec lives at the repo root, and this site is the navigable view. Start with [Overall architecture](architecture.md) for the full system picture, or [Voice loop](voice-loop.md) for how commands flow through the pipeline.
