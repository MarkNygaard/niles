// niles voice satellite (XIAO ESP32-S3) — ESP-IDF firmware.
//
// STAGE 1 (this file): microWakeWord "Hey Jarvis" detection only.
// Reads the XVF3800 I2S mic, runs the microWakeWord pipeline, and prints
// the detection probability so we can confirm + tune before building
// streaming / barge-in around it.
//
// Pipeline (verified against current microWakeWord / esp-tflite-micro):
//   I2S mic 16 kHz mono -> 30 ms sliding window (slid 10 ms) -> the
//   AUDIO PREPROCESSOR model (audio_preprocessor_int8_model_data.h, via
//   micro_features_generator) emits 40 int8 spectrogram features per slice
//   -> streaming wake-word model (hey_jarvis.tflite, internal state) ->
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
//   2) Random prob / never rises on "Hey Jarvis" -> a feature-quantization
//      mismatch between the preprocessor output and the wake-word model
//      input (requantize using their scale/zero_point), or the I2S slot
//      format (Philips vs MSB).
//   3) Audio garbage -> I2S slot/bit-width/pin config below.

#include <cstdio>
#include <cstring>

#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "driver/i2s_std.h"
#include "esp_log.h"

#include "tensorflow/lite/micro/micro_interpreter.h"
#include "tensorflow/lite/micro/micro_mutable_op_resolver.h"
#include "tensorflow/lite/micro/micro_resource_variable.h"
#include "tensorflow/lite/micro/system_setup.h"
#include "tensorflow/lite/schema/schema_generated.h"

#include "micro_features_generator.h"  // GenerateFeatures, InitializeMicroFeatures, Features
#include "micro_model_settings.h"      // kFeatureSize, kAudioSampleFrequency, kFeatureDurationMs

// Embedded wake-word model (EMBED_FILES "hey_jarvis.tflite").
extern const uint8_t g_model_start[] asm("_binary_hey_jarvis_tflite_start");

static const char* TAG = "niles-ww";

// ---- audio / windowing ----
static constexpr int SAMPLE_RATE = kAudioSampleFrequency;                 // 16000
static constexpr int WINDOW_SAMPLES = kFeatureDurationMs * SAMPLE_RATE / 1000; // 480 (30 ms)
static constexpr int STRIDE_SAMPLES = 10 * SAMPLE_RATE / 1000;            // 160 (10 ms)

// XVF3800 I2S pins (match the proven VAD wiring): BCLK 8, WS 7, DIN 43.
static constexpr gpio_num_t PIN_BCLK = GPIO_NUM_8;
static constexpr gpio_num_t PIN_WS = GPIO_NUM_7;
static constexpr gpio_num_t PIN_DIN = GPIO_NUM_43;

// ---- microWakeWord "Hey Jarvis" v2 (from hey_jarvis.json) ----
static constexpr float PROB_CUTOFF = 0.30f; // tuning; wake word peaks ~0.45, noise <0.1
static constexpr int WINDOW_AVG = 5;

// The XVF3800 mono downmix is low-level (~6.6% of full scale on loud
// speech); the wake-word preprocessor expects normal-level PCM, so we
// amplify before feature extraction. TUNE THIS: raise it until the
// heartbeat's featmax clearly rises above the quiet floor when you speak
// (try 8 / 16 / 32). Too high = clipping distortion.
static constexpr int MIC_GAIN = 8;

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

static void model_init() {
  const tflite::Model* model = tflite::GetModel(g_model_start);
  if (model->version() != TFLITE_SCHEMA_VERSION) {
    ESP_LOGE(TAG, "wake model schema %lu != %d", (unsigned long)model->version(),
             TFLITE_SCHEMA_VERSION);
    abort();
  }
  // Streaming microWakeWord op set. Add any op Invoke() reports missing.
  static tflite::MicroMutableOpResolver<20> resolver;
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

extern "C" void app_main(void) {
  tflite::InitializeTarget();
  if (InitializeMicroFeatures() != kTfLiteOk) {
    ESP_LOGE(TAG, "InitializeMicroFeatures failed");
    abort();
  }
  model_init();
  i2s_init();
  memset(window, 0, sizeof(window));
  ESP_LOGI(TAG, "listening — say 'Hey Jarvis'");

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
          ESP_LOGI(TAG, ">>> WAKE WORD DETECTED: Hey Jarvis <<< (prob=%.3f)", (double)prob);
        }
      }
    }

    // ~1 s heartbeat (each iter is 10 ms): the MAX mic peak, MAX feature, and
    // MAX probability seen this second. featmax jumps when you speak; maxprob
    // jumps when you say "Hey Jarvis".
    if (++iter % 100 == 0) {
      ESP_LOGI(TAG, "peak=%ld featmax=%d maxprob=%.3f", (long)hb_peak, hb_featmax,
               (double)hb_maxprob);
      hb_peak = 0;
      hb_featmax = -128;
      hb_maxprob = 0.0f;
    }
  }
}
