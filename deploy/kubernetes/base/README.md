# Kubernetes base (Kustomize)

Kustomize base for the multi-node Kubernetes deployment path. Populated in **Phase 11** per [ARCHITECTURE.md](../../../ARCHITECTURE.md#kubernetes).

Will contain Deployment + Service + PVC for: Mosquitto, Zigbee2MQTT, Piper TTS, and Niles, plus the `niles-config` ConfigMap and `niles-secrets` Secret references.
