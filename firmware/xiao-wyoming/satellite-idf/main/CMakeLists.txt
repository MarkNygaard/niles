idf_component_register(
  SRCS "main.cpp" "micro_features_generator.cc"
  INCLUDE_DIRS "."
  # Embed the wake-word model as a binary blob; referenced in main.cpp as
  # _binary_nyles_tflite_start / _end.
  EMBED_FILES "nyles.tflite"
  # esp_driver_i2s = the v5.x I2S standard-mode driver. esp-tflite-micro is
  # pulled in as a managed component (idf_component.yml), auto-linked.
  # WiFi + sockets for streaming to niles (Stage 2): esp_wifi/nvs_flash/
  # esp_event/esp_netif (+ lwip sockets via esp_netif).
  REQUIRES esp_driver_i2s esp_wifi nvs_flash esp_event esp_netif lwip
)
