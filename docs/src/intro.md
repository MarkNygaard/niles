# Introduction

**Niles** (NYE-uls) — *Neural Intelligence, Lightweight Edge System* — is an open-source, AI-first home automation system designed to replace Home Assistant for users who want sub-second voice interactions, code/config over UI, and a voice assistant with real LLM intelligence as a first-class citizen.

It runs locally on small hardware, bridges Zigbee devices via Zigbee2MQTT, and exposes a voice interface through ESPHome-based satellites with a three-tier intent pipeline: regex → fast LLM → smart LLM.

The canonical architecture spec lives at the repo root; this site is the navigable view. Start with [Overall architecture](architecture.md) for the full system picture, or jump to [Voice loop](voice-loop.md) for how commands flow through the pipeline.
