// niles voice satellite (XIAO ESP32-S3) — ESP-IDF firmware.
//
// STAGE 1 (this file): microWakeWord "nyles" detection only.
// Reads the XVF3800 I2S mic, runs the microWakeWord pipeline, and prints
// the detection probability so we can confirm + tune before building
// streaming / barge-in around it.
//
// Pipeline (verified against current microWakeWord / esp-tflite-micro):
//   I2S mic 16 kHz mono -> 30 ms sliding window (slid 10 ms) -> the
//   AUDIO PREPROCESSOR model (audio_preprocessor_int8_model_data.h, via
//   micro_features_generator) emits 40 int8 spectrogram features per slice
//   -> streaming wake-word model (nyles.tflite, internal state) ->
//   sigmoid -> 5-frame average -> fire if > 0.97.
//
// NOTE: current esp-tflite-micro dropped the C microfrontend in favour of
// this preprocessor-model approach (signal ops: Window/Rfft/FilterBank/
// PCAN/...). microWakeWord uses the same preprocessor, so its int8 output
// feeds the wake-word model directly — no manual feature quantization.
//
// Toolchain: ESP-IDF v5+/v6. Build/flash via the Espressif IDF VS Code
// extension (or `idf.py set-target esp32s3 && idf.py build flash monitor`).
//
// === Iteration surface (from-scratch port; expect tuning) ===
//   1) Invoke() error / frozen prob on the WAKE-WORD model -> its op set
//      (kResolver below) or kNumResourceVars.
//   2) Random prob / never rises on "nyles" -> a feature-quantization
//      mismatch between the preprocessor output and the wake-word model
//      input (requantize using their scale/zero_point), or the I2S slot
//      format (Philips vs MSB).
//   3) Audio garbage -> I2S slot/bit-width/pin config below.

#include <cstdio>
#include <cstring>

#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "freertos/event_groups.h"
#include "driver/i2s_std.h"
#include "esp_log.h"
#include "esp_wifi.h"
#include "esp_event.h"
#include "esp_netif.h"
#include "nvs_flash.h"
#include "lwip/sockets.h"
#include "lwip/inet.h"

#include "secrets.h"

#include "tensorflow/lite/micro/micro_interpreter.h"
#include "tensorflow/lite/micro/micro_mutable_op_resolver.h"
#include "tensorflow/lite/micro/micro_resource_variable.h"
#include "tensorflow/lite/micro/system_setup.h"
#include "tensorflow/lite/schema/schema_generated.h"

#include "micro_features_generator.h"  // GenerateFeatures, InitializeMicroFeatures, Features
#include "micro_model_settings.h"      // kFeatureSize, kAudioSampleFrequency, kFeatureDurationMs

// Embedded wake-word model (EMBED_FILES "nyles.tflite").
extern const uint8_t g_model_start[] asm("_binary_nyles_tflite_start");

static const char* TAG = "niles-ww";

// ---- audio / windowing ----
static constexpr int SAMPLE_RATE = kAudioSampleFrequency;                 // 16000
static constexpr int WINDOW_SAMPLES = kFeatureDurationMs * SAMPLE_RATE / 1000; // 480 (30 ms)
static constexpr int STRIDE_SAMPLES = 10 * SAMPLE_RATE / 1000;            // 160 (10 ms)

// XVF3800 I2S pins (match the proven VAD wiring): BCLK 8, WS 7, DIN 43,
// DOUT 44 (playback — the XVF3800 plays I2S-TX audio on its speaker).
static constexpr gpio_num_t PIN_BCLK = GPIO_NUM_8;
static constexpr gpio_num_t PIN_WS = GPIO_NUM_7;
static constexpr gpio_num_t PIN_DIN = GPIO_NUM_43;
static constexpr gpio_num_t PIN_DOUT = GPIO_NUM_44;

// ---- microWakeWord "nyles" v2 (custom model, from nyles.json) ----
// nyles.json recommends probability_cutoff 0.98, but that assumes a 5-window
// average; we fire on the single-invoke probability instead, and the interim
// "Hey Jarvis" model only peaked ~0.45 through this same preprocessor/cadence.
// So START here and TUNE from the heartbeat: say "nyles" a few times, read the
// `maxprob` it logs, then set this to roughly 60-70% of that peak — comfortably
// above the maxprob you see for ambient noise and look-alikes (miles/files).
static constexpr float PROB_CUTOFF = 0.50f;
static constexpr int WINDOW_AVG = 5;

// The XVF3800 mono downmix is low-level; the wake-word preprocessor expects
// normal-level PCM, so we amplify before feature extraction. TUNE THIS: too
// low and features floor at -128; too high and speech CLIPS (heartbeat peak
// pins at 32768), which smears the spectrogram and suppresses maxprob. Aim
// for speech peaks well under 32768. Gain 8 clipped hard on "nyles"; 4 keeps
// loud speech ~mid-scale.
static constexpr int MIC_GAIN = 4;

static i2s_chan_handle_t rx_chan = nullptr;
static int32_t i2s_buf[STRIDE_SAMPLES * 2]; // XVF3800 = 2ch / 32-bit
static int16_t window[WINDOW_SAMPLES];      // 30 ms sliding window of mono int16

// ---- wake-word model (the preprocessor model lives in micro_features_generator) ----
static constexpr int kArenaSize = 64 * 1024;
alignas(16) static uint8_t tensor_arena[kArenaSize];
static tflite::MicroInterpreter* interpreter = nullptr;
static TfLiteTensor* input = nullptr;
static TfLiteTensor* output = nullptr;
static constexpr int kNumResourceVars = 20; // streaming state vars

static void i2s_init() {
  i2s_chan_config_t chan_cfg = I2S_CHANNEL_DEFAULT_CONFIG(I2S_NUM_0, I2S_ROLE_MASTER);
  ESP_ERROR_CHECK(i2s_new_channel(&chan_cfg, nullptr, &rx_chan));
  i2s_std_config_t std_cfg = {
      .clk_cfg = I2S_STD_CLK_DEFAULT_CONFIG(SAMPLE_RATE),
      .slot_cfg = I2S_STD_PHILIPS_SLOT_DEFAULT_CONFIG(I2S_DATA_BIT_WIDTH_32BIT,
                                                      I2S_SLOT_MODE_STEREO),
      .gpio_cfg = {
          .mclk = I2S_GPIO_UNUSED,
          .bclk = PIN_BCLK,
          .ws = PIN_WS,
          .dout = I2S_GPIO_UNUSED,
          .din = PIN_DIN,
          .invert_flags = {.mclk_inv = false, .bclk_inv = false, .ws_inv = false},
      },
  };
  ESP_ERROR_CHECK(i2s_channel_init_std_mode(rx_chan, &std_cfg));
  ESP_ERROR_CHECK(i2s_channel_enable(rx_chan));
}

// Playback uses a TX channel at the reply's sample rate. The I2S port has one
// active direction at a time, so we tear down RX, play, then restore RX. (True
// duplex / barge-in is a later stage.)
static i2s_chan_handle_t tx_chan = nullptr;

static void i2s_deinit_rx() {
  if (rx_chan) {
    i2s_channel_disable(rx_chan);
    i2s_del_channel(rx_chan);
    rx_chan = nullptr;
  }
}

static void i2s_start_tx(int rate) {
  i2s_deinit_rx();
  i2s_chan_config_t chan_cfg = I2S_CHANNEL_DEFAULT_CONFIG(I2S_NUM_0, I2S_ROLE_MASTER);
  ESP_ERROR_CHECK(i2s_new_channel(&chan_cfg, &tx_chan, nullptr));
  i2s_std_config_t std_cfg = {
      .clk_cfg = I2S_STD_CLK_DEFAULT_CONFIG((uint32_t)rate),
      .slot_cfg = I2S_STD_PHILIPS_SLOT_DEFAULT_CONFIG(I2S_DATA_BIT_WIDTH_32BIT,
                                                      I2S_SLOT_MODE_STEREO),
      .gpio_cfg = {
          .mclk = I2S_GPIO_UNUSED,
          .bclk = PIN_BCLK,
          .ws = PIN_WS,
          .dout = PIN_DOUT,
          .din = I2S_GPIO_UNUSED,
          .invert_flags = {.mclk_inv = false, .bclk_inv = false, .ws_inv = false},
      },
  };
  ESP_ERROR_CHECK(i2s_channel_init_std_mode(tx_chan, &std_cfg));
  ESP_ERROR_CHECK(i2s_channel_enable(tx_chan));
}

static void i2s_stop_tx() {
  if (tx_chan) {
    i2s_channel_disable(tx_chan);
    i2s_del_channel(tx_chan);
    tx_chan = nullptr;
  }
}

static void model_init() {
  const tflite::Model* model = tflite::GetModel(g_model_start);
  if (model->version() != TFLITE_SCHEMA_VERSION) {
    ESP_LOGE(TAG, "wake model schema %lu != %d", (unsigned long)model->version(),
             TFLITE_SCHEMA_VERSION);
    abort();
  }
  // Streaming microWakeWord op set. Add any op Invoke() reports missing.
  static tflite::MicroMutableOpResolver<21> resolver;
  resolver.AddCallOnce();
  resolver.AddVarHandle();
  resolver.AddReadVariable();
  resolver.AddAssignVariable();
  resolver.AddConv2D();
  resolver.AddDepthwiseConv2D();
  resolver.AddFullyConnected();
  resolver.AddRelu();
  resolver.AddReshape();
  resolver.AddExpandDims();
  resolver.AddStridedSlice();
  resolver.AddConcatenation();
  resolver.AddSplit();
  resolver.AddSplitV(); // nyles model splits with SPLIT_V (hey_jarvis didn't)
  resolver.AddMul();
  resolver.AddAdd();
  resolver.AddMean();
  resolver.AddLogistic();
  resolver.AddQuantize();
  resolver.AddDequantize();
  resolver.AddAveragePool2D();

  // One allocator, shared by the interpreter AND the resource variables.
  // (Creating a second allocator over the same arena corrupts the streaming
  // state-variable names -> crash in VarHandlePrepare/AllocateTensors.)
  static tflite::MicroAllocator* allocator =
      tflite::MicroAllocator::Create(tensor_arena, kArenaSize);
  static tflite::MicroResourceVariables* resources =
      tflite::MicroResourceVariables::Create(allocator, kNumResourceVars);
  static tflite::MicroInterpreter static_interp(model, resolver, allocator, resources);
  interpreter = &static_interp;
  if (interpreter->AllocateTensors() != kTfLiteOk) {
    ESP_LOGE(TAG, "wake model AllocateTensors failed (arena/op?)");
    abort();
  }
  input = interpreter->input(0);
  output = interpreter->output(0);
  ESP_LOGI(TAG, "wake model: input type=%d elems=%d scale=%.6f zp=%d | output scale=%.6f zp=%d",
           input->type, (int)(input->bytes), (double)input->params.scale,
           (int)input->params.zero_point, (double)output->params.scale,
           (int)output->params.zero_point);
}

// Append one 10 ms slice to the sliding window (shift left, fill tail).
// Mono = XVF3800 left channel, top 16 bits.
static void push_slice() {
  size_t got = 0;
  i2s_channel_read(rx_chan, i2s_buf, sizeof(i2s_buf), &got, portMAX_DELAY);
  int frames = got / (sizeof(int32_t) * 2);
  if (frames > STRIDE_SAMPLES) frames = STRIDE_SAMPLES;
  memmove(window, window + STRIDE_SAMPLES, (WINDOW_SAMPLES - STRIDE_SAMPLES) * sizeof(int16_t));
  int16_t* tail = window + (WINDOW_SAMPLES - STRIDE_SAMPLES);
  for (int f = 0; f < frames; f++) {
    int32_t s = (int32_t)(i2s_buf[f * 2] >> 16) * MIC_GAIN;
    if (s > 32767) s = 32767;
    else if (s < -32768) s = -32768;
    tail[f] = (int16_t)s;
  }
  for (int f = frames; f < STRIDE_SAMPLES; f++) tail[f] = 0;
}

// ---- Wyoming streaming to niles (Stage 2) ----
static bool send_all(int sock, const void* buf, size_t len) {
  const uint8_t* p = static_cast<const uint8_t*>(buf);
  while (len) {
    int n = send(sock, p, len, 0);
    if (n <= 0) return false;
    p += n;
    len -= n;
  }
  return true;
}

// ---- Reply playback (Stage 3) ----
// Read a newline-terminated line from the socket (Wyoming headers are JSON
// lines). Returns length, or -1 on EOF/timeout.
static int sock_read_line(int sock, char* buf, int max) {
  int idx = 0;
  while (idx < max - 1) {
    char c;
    int n = recv(sock, &c, 1, 0);
    if (n <= 0) return -1;
    if (c == '\n') break;
    buf[idx++] = c;
  }
  buf[idx] = 0;
  return idx;
}

static bool sock_read_full(int sock, uint8_t* buf, int n) {
  int got = 0;
  while (got < n) {
    int r = recv(sock, buf + got, n - got, 0);
    if (r <= 0) return false;
    got += r;
  }
  return true;
}

// Parse the integer right after `key` in a JSON header line (e.g. "\"rate\":").
static long json_int_after(const char* s, const char* key) {
  const char* p = strstr(s, key);
  if (!p) return -1;
  return atol(p + strlen(key));
}

// After audio-stop, niles runs STT + intent + TTS and streams its spoken reply
// back over the same socket (audio-start{rate} / audio-chunk+PCM / audio-stop).
// Play it via I2S TX (mono 16-bit -> stereo 32-bit, L=R), reconfiguring the
// I2S rate to the reply's rate, then restore RX for wake detection.
static void play_reply(int sock) {
  struct timeval tv = {.tv_sec = 10, .tv_usec = 0}; // niles needs time to think
  setsockopt(sock, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));

  char line[192];
  uint8_t pcm[1024];
  static int32_t stereo[512 * 2]; // up to 512 mono samples -> stereo32
  bool playing = false;

  while (true) {
    int len = sock_read_line(sock, line, sizeof(line));
    if (len < 0) {
      ESP_LOGW(TAG, "no reply / timeout");
      break;
    }
    if (strstr(line, "audio-start")) {
      long rate = json_int_after(line, "\"rate\":");
      if (rate <= 0) rate = 22050;
      ESP_LOGI(TAG, "reply audio-start rate=%ld", rate);
      i2s_start_tx((int)rate);
      playing = true;
    } else if (strstr(line, "audio-chunk")) {
      long rem = json_int_after(line, "\"payload_length\":");
      if (rem <= 0) continue;
      while (rem > 0) {
        int want = rem < (long)sizeof(pcm) ? (int)rem : (int)sizeof(pcm);
        if (!sock_read_full(sock, pcm, want)) {
          ESP_LOGW(TAG, "reply chunk read failed");
          rem = 0;
          break;
        }
        if (playing && tx_chan) {
          const int16_t* s = reinterpret_cast<const int16_t*>(pcm);
          int ns = want / (int)sizeof(int16_t);
          for (int i = 0; i < ns; i++) {
            int32_t v = (int32_t)s[i] << 16;
            stereo[i * 2] = v;
            stereo[i * 2 + 1] = v;
          }
          size_t wrote = 0;
          i2s_channel_write(tx_chan, stereo, ns * 2 * sizeof(int32_t), &wrote, portMAX_DELAY);
        }
        rem -= want;
      }
    } else if (strstr(line, "audio-stop")) {
      ESP_LOGI(TAG, "reply done");
      break;
    }
  }
  i2s_stop_tx();
  i2s_init(); // restore RX @ 16 kHz for wake detection
}

// After wake detection, open a Wyoming TCP connection to niles and stream the
// spoken command as mono 16 kHz 16-bit PCM (raw downmix, no MIC_GAIN — cleaner
// for STT), ending on silence (energy VAD) or a hard cap. No playback yet.
static void stream_utterance() {
  struct sockaddr_in dest = {};
  dest.sin_family = AF_INET;
  dest.sin_port = htons(NILES_PORT);
  if (inet_pton(AF_INET, NILES_HOST, &dest.sin_addr) != 1) {
    ESP_LOGE(TAG, "bad NILES_HOST '%s'", NILES_HOST);
    return;
  }
  int sock = socket(AF_INET, SOCK_STREAM, IPPROTO_IP);
  if (sock < 0) {
    ESP_LOGE(TAG, "socket() failed");
    return;
  }
  if (connect(sock, reinterpret_cast<struct sockaddr*>(&dest), sizeof(dest)) != 0) {
    ESP_LOGE(TAG, "connect %s:%d failed (errno %d)", NILES_HOST, NILES_PORT, errno);
    close(sock);
    return;
  }

  const char* start =
      "{\"type\":\"audio-start\",\"data\":{\"rate\":16000,\"width\":2,\"channels\":1}}\n";
  if (!send_all(sock, start, strlen(start))) {
    close(sock);
    return;
  }

  // mean |sample| (raw, ungained) below this = silence. Ambient measures
  // ~15-24, real speech reaches thousands, so 60 separates them cleanly.
  // Lower it if quiet commands get cut off; raise if it never ends.
  static const int STOP_RMS = 60;
  static const int HANGOVER_FRAMES = 50; // ~500 ms of silence ends the utterance
  static const int MAX_FRAMES = 600;     // ~6 s hard cap

  int16_t slice[STRIDE_SAMPLES];
  char hdr[64];
  int silent = 0, total = 0;
  long emin = 1 << 30, emax = 0;
  while (silent < HANGOVER_FRAMES && total < MAX_FRAMES) {
    size_t got = 0;
    i2s_channel_read(rx_chan, i2s_buf, sizeof(i2s_buf), &got, portMAX_DELAY);
    int frames = got / (sizeof(int32_t) * 2);
    if (frames > STRIDE_SAMPLES) frames = STRIDE_SAMPLES;
    long sum = 0;
    for (int f = 0; f < frames; f++) {
      int16_t v = (int16_t)(i2s_buf[f * 2] >> 16); // raw mono, no gain
      slice[f] = v;
      sum += v < 0 ? -v : v;
    }
    for (int f = frames; f < STRIDE_SAMPLES; f++) slice[f] = 0;
    long energy = frames ? sum / frames : 0;
    if (energy < emin) emin = energy;
    if (energy > emax) emax = energy;

    int pb = STRIDE_SAMPLES * (int)sizeof(int16_t);
    int n = snprintf(hdr, sizeof(hdr), "{\"type\":\"audio-chunk\",\"payload_length\":%d}\n", pb);
    if (!send_all(sock, hdr, n) || !send_all(sock, slice, pb)) break;

    if (energy < STOP_RMS) silent++;
    else silent = 0;
    total++;
  }

  const char* stop = "{\"type\":\"audio-stop\"}\n";
  send_all(sock, stop, strlen(stop));
  ESP_LOGI(TAG, "utterance streamed (%d frames, ~%d ms) energy[min=%ld max=%ld] (STOP_RMS=%d)",
           total, total * 10, emin, emax, STOP_RMS);

  // Read + play niles' spoken reply on the same socket (Stage 3), then close.
  play_reply(sock);
  close(sock);
}

// ---- WiFi (station) ----
static EventGroupHandle_t s_wifi_events;
static constexpr int WIFI_CONNECTED_BIT = BIT0;

static void wifi_event_handler(void*, esp_event_base_t base, int32_t id, void* data) {
  if (base == WIFI_EVENT && id == WIFI_EVENT_STA_START) {
    esp_wifi_connect();
  } else if (base == WIFI_EVENT && id == WIFI_EVENT_STA_DISCONNECTED) {
    ESP_LOGW(TAG, "wifi disconnected — reconnecting");
    esp_wifi_connect();
  } else if (base == IP_EVENT && id == IP_EVENT_STA_GOT_IP) {
    auto* e = static_cast<ip_event_got_ip_t*>(data);
    ESP_LOGI(TAG, "wifi connected, IP=" IPSTR, IP2STR(&e->ip_info.ip));
    xEventGroupSetBits(s_wifi_events, WIFI_CONNECTED_BIT);
  }
}

static void wifi_init_sta() {
  s_wifi_events = xEventGroupCreate();
  ESP_ERROR_CHECK(esp_netif_init());
  ESP_ERROR_CHECK(esp_event_loop_create_default());
  esp_netif_create_default_wifi_sta();

  wifi_init_config_t cfg = WIFI_INIT_CONFIG_DEFAULT();
  ESP_ERROR_CHECK(esp_wifi_init(&cfg));
  ESP_ERROR_CHECK(esp_event_handler_instance_register(WIFI_EVENT, ESP_EVENT_ANY_ID,
                                                      &wifi_event_handler, nullptr, nullptr));
  ESP_ERROR_CHECK(esp_event_handler_instance_register(IP_EVENT, IP_EVENT_STA_GOT_IP,
                                                      &wifi_event_handler, nullptr, nullptr));
  wifi_config_t wc = {};
  strncpy(reinterpret_cast<char*>(wc.sta.ssid), WIFI_SSID, sizeof(wc.sta.ssid) - 1);
  strncpy(reinterpret_cast<char*>(wc.sta.password), WIFI_PASS, sizeof(wc.sta.password) - 1);
  ESP_ERROR_CHECK(esp_wifi_set_mode(WIFI_MODE_STA));
  ESP_ERROR_CHECK(esp_wifi_set_config(WIFI_IF_STA, &wc));
  ESP_ERROR_CHECK(esp_wifi_start());
  ESP_LOGI(TAG, "wifi connecting to '%s'...", WIFI_SSID);
  xEventGroupWaitBits(s_wifi_events, WIFI_CONNECTED_BIT, pdFALSE, pdTRUE, portMAX_DELAY);
}

extern "C" void app_main(void) {
  // NVS is required by WiFi.
  esp_err_t nvs = nvs_flash_init();
  if (nvs == ESP_ERR_NVS_NO_FREE_PAGES || nvs == ESP_ERR_NVS_NEW_VERSION_FOUND) {
    ESP_ERROR_CHECK(nvs_flash_erase());
    ESP_ERROR_CHECK(nvs_flash_init());
  }
  wifi_init_sta();

  tflite::InitializeTarget();
  if (InitializeMicroFeatures() != kTfLiteOk) {
    ESP_LOGE(TAG, "InitializeMicroFeatures failed");
    abort();
  }
  model_init();
  i2s_init();
  memset(window, 0, sizeof(window));
  ESP_LOGI(TAG, "listening — say 'nyles'");

  float ring[WINDOW_AVG] = {0};
  int idx = 0;
  int warmup = WINDOW_SAMPLES / STRIDE_SAMPLES; // fill the window first (~3 slices)
  int64_t last_fire_ms = 0;
  static Features features; // int8[kFeatureCount][kFeatureSize]; we use [0]

  int iter = 0;
  int32_t hb_peak = 0;     // max audio peak this heartbeat
  int hb_featmax = -128;   // max feature value this heartbeat
  float hb_maxprob = 0.0f; // max probability this heartbeat

  // The model takes 3 feature slices (3 * 40 = 120 int8) per inference and is
  // invoked every 30 ms (3 fresh slices). This cadence detects clearly better
  // than a 10 ms sliding window (which barely registered).
  static int8_t feat3[3 * kFeatureSize];
  int slot = 0;
  while (true) {
    push_slice();
    if (warmup > 0) { warmup--; continue; }

    // Audio level of the current window (peak |sample|) — confirms the mic
    // is actually capturing.
    int32_t peak = 0;
    for (int i = 0; i < WINDOW_SAMPLES; i++) {
      int v = window[i] < 0 ? -window[i] : window[i];
      if (v > peak) peak = v;
    }
    if (peak > hb_peak) hb_peak = peak;

    // One 40-feature slice for the current 30 ms window. Passing exactly
    // WINDOW_SAMPLES makes GenerateFeatures emit a single slice -> [0].
    if (GenerateFeatures(window, WINDOW_SAMPLES, &features) != kTfLiteOk) {
      ESP_LOGW(TAG, "GenerateFeatures failed");
      continue;
    }

    int featmax = -128;
    for (int i = 0; i < kFeatureSize; i++)
      if (features[0][i] > featmax) featmax = features[0][i];
    if (featmax > hb_featmax) hb_featmax = featmax;

    // Append this slice; invoke once we have 3 fresh slices (every 30 ms).
    memcpy(&feat3[slot * kFeatureSize], features[0], kFeatureSize * sizeof(int8_t));
    slot = (slot + 1) % 3;
    if (slot == 0) {
      for (int i = 0; i < 3 * kFeatureSize; i++) input->data.int8[i] = feat3[i];
      if (interpreter->Invoke() != kTfLiteOk) {
        ESP_LOGE(TAG, "wake Invoke failed");
      } else {
        float prob =
            (output->data.int8[0] - output->params.zero_point) * output->params.scale;
        if (prob > hb_maxprob) hb_maxprob = prob;
        int64_t now_ms = esp_log_timestamp();
        if (prob >= PROB_CUTOFF && now_ms - last_fire_ms > 1500) {
          last_fire_ms = now_ms;
          ESP_LOGI(TAG, ">>> WAKE WORD DETECTED (prob=%.3f) — streaming command <<<",
                   (double)prob);
          stream_utterance();
          // Reset wake state so stale slices don't immediately re-fire.
          slot = 0;
          last_fire_ms = esp_log_timestamp();
        }
      }
    }

    // ~1 s heartbeat (each iter is 10 ms): the MAX mic peak, MAX feature, and
    // MAX probability seen this second. featmax jumps when you speak; maxprob
    // jumps when you say "nyles". Use this to set PROB_CUTOFF (see above).
    if (++iter % 100 == 0) {
      ESP_LOGI(TAG, "peak=%ld featmax=%d maxprob=%.3f", (long)hb_peak, hb_featmax,
               (double)hb_maxprob);
      hb_peak = 0;
      hb_featmax = -128;
      hb_maxprob = 0.0f;
    }
  }
}
