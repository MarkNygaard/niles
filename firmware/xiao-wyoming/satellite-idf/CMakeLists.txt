# ESP-IDF project root for the niles voice satellite (XIAO ESP32-S3).
# Build/flash/monitor via the Espressif IDF VS Code extension, or:
#   idf.py set-target esp32s3 && idf.py build flash monitor
cmake_minimum_required(VERSION 3.16)
include($ENV{IDF_PATH}/tools/cmake/project.cmake)
project(niles_satellite)
