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
#include "freertos/queue.h"
#include "freertos/semphr.h"
#include "driver/i2c_master.h"
#include "driver/i2s_std.h"
#include "esp_log.h"
#include "esp_wifi.h"
#include "esp_event.h"
#include "esp_netif.h"
#include "nvs_flash.h"
#include "lwip/sockets.h"
#include "lwip/inet.h"
#include <fcntl.h>

#include <atomic>

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
// Where niles reaches us to say something unprompted. Fixed rather
// than configured: it is the satellite's own listening port, and niles
// learns it from its `[satellites]` entry.
static constexpr int NILES_PUSH_PORT = 10301;

static constexpr gpio_num_t PIN_BCLK = GPIO_NUM_8;
static constexpr gpio_num_t PIN_WS = GPIO_NUM_7;
static constexpr gpio_num_t PIN_DIN = GPIO_NUM_43;
static constexpr gpio_num_t PIN_DOUT = GPIO_NUM_44;

// ---- microWakeWord "nyles" v2 (custom model, from nyles.json) ----
// On hardware, a real "nyles" peaks ~0.89-0.99 while ambient noise and
// look-alikes top out ~0.42. At 0.80, background speech occasionally crossed
// the bar and false-woke; real detections never drop below ~0.89, so 0.85
// leaves margin for true hits while rejecting more of the background.
// nyles.json suggests 0.98, but that assumes a 5-window average; we fire on the
// single-invoke probability. Lower toward 0.8 if real "nyles" gets missed;
// raise toward 0.9 (or add 5-frame averaging) if background still false-wakes.
//
// 2026-09-12: it did. Background conversation during washing-up woke it on
// "so", so 0.85 -> 0.90. That is close to the bottom of the real-detection
// range (~0.89), so the next false-wake is an argument for 5-frame averaging
// or more negative training data, not for raising this further — past ~0.92
// there is nothing left between the two distributions.
static constexpr float PROB_CUTOFF = 0.90f;
static constexpr int WINDOW_AVG = 5;

// 2026-09-23: the next false wake arrived, and then fifty-two more. One day
// of logs: 53 wakes, 43 of them answered into an empty room -- "Thank you."
// nine times, and television dialogue transcribed perfectly. PROB_CUTOFF was
// doing what it could; the problem is that it judges ONE inference.
//
// The detector runs every 30 ms, and a word that merely resembles "nyles"
// produces a single high slice. The wake word itself produces a run of them.
// So the ring declared below -- present and unused since this was written --
// is now filled, and the decision is made on the average of the last
// WINDOW_AVG inferences (~150 ms).
//
// That changes what the number means, so it gets its own. A real detection
// peaks near 1.0 across two or three slices of five and averages well below
// its peak; a one-slice accident averages to almost nothing. Both are logged
// per heartbeat as maxprob / maxavg -- tune AVG_CUTOFF from those, the way
// PROB_CUTOFF was tuned from maxprob.
//
// Measured the same evening, five real wakes against "nice" and "files"
// said deliberately at the same distance:
//
//   "nyles"          avg 0.573 0.576 0.633 0.670 0.849
//   nice / files     avg 0.355 0.264 0.160 0.121 0.109
//
// A gap of 0.218, which is the thing the single-frame check could not see:
// on maxprob those two sets overlap almost completely (a "nice" peaked at
// 0.945). 0.45 sits near the middle of the gap -- 0.123 below the weakest
// real wake and 0.095 above the loudest impostor. The first guess of 0.55
// cleared the weakest real wake by 0.023, which is not a margin.
//
// Except those impostors were the wrong ones. "nice" and "files" are single
// short words, which average low whatever they peak at; a television speaks
// continuously, and at 0.45 it woke the house nine times in twenty-six
// minutes. The negatives that matter are the ones actually in the room.
//
// Back to 0.55 while the real distribution is collected -- which is what the
// wake_avg reported below is for. A threshold argued from four minutes at a
// desk is how this went wrong twice.
static constexpr float AVG_CUTOFF = 0.55f;

// ~1 s of audio kept before the wake word, in PSRAM.
//
// The satellite used to start streaming at the moment of detection, so the
// sound that triggered it was the one sound never sent. That makes a false
// wake impossible to learn from: niles received the silence afterwards and
// Whisper turned it into "Thank you.". Retraining needs the trigger itself.
//
// It also un-clips real commands, which began a fraction of a second before
// the stream opened.
static constexpr int PREROLL_SAMPLES = SAMPLE_RATE; // 1.0 s at 16 kHz
// Up here rather than inside stream_utterance, because the pre-roll has to
// apply the same gain as the live stream or the seam is audible.
static constexpr int STREAM_GAIN = 6;

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
// Raw mono, pre-gain: the stream applies its own STREAM_GAIN, and storing
// the detector's MIC_GAIN copy would amplify it twice. Allocated from PSRAM
// at boot, because 32 KB of internal SRAM is worth more elsewhere.
static int16_t* preroll = nullptr;
static int preroll_pos = 0;  // next slot to write
static bool preroll_full = false;

// ---- wake-word model (the preprocessor model lives in micro_features_generator) ----
static constexpr int kArenaSize = 64 * 1024;
alignas(16) static uint8_t tensor_arena[kArenaSize];
static tflite::MicroInterpreter* interpreter = nullptr;
static TfLiteTensor* input = nullptr;
static TfLiteTensor* output = nullptr;
static constexpr int kNumResourceVars = 20; // streaming state vars

// ---- LED ring ----
//
// Left alone, the XVF3800 drives its own ring in direction-of-arrival
// mode: it lights towards whatever it hears, all day, including the
// television. That is motion in the corner of your eye reporting on
// sound nobody asked it about. The ring is far more useful saying what
// *niles* is doing, which is only ever one of four things.
//
// Control is I2C to the XMOS chip: [resource][command][byte count][data].
// Resource 20 is the GPO servicer; effect 0=off, 1=breathing, 2=rainbow,
// 3=solid, 4=DOA, 5=ring.
static constexpr uint8_t XMOS_I2C_ADDR = 0x2C;
static constexpr uint8_t XMOS_RES_GPO = 20;
static constexpr uint8_t XMOS_CMD_LED_EFFECT = 12;
static constexpr uint8_t XMOS_CMD_LED_BRIGHTNESS = 13;
static constexpr uint8_t XMOS_CMD_LED_COLOR = 16;

static constexpr uint8_t LED_EFFECT_OFF = 0;
static constexpr uint8_t LED_EFFECT_BREATHING = 1;
static constexpr uint8_t LED_EFFECT_SOLID = 3;

// XIAO ESP32-S3's I2C pins, which is where the XVF3800's control
// interface lands on this carrier.
static constexpr gpio_num_t PIN_SDA = GPIO_NUM_5;
static constexpr gpio_num_t PIN_SCL = GPIO_NUM_6;

static i2c_master_dev_handle_t xmos_dev = nullptr;
// Two tasks drive the ring now -- the wake loop and the speaker -- and a
// colour and an effect are two writes that must not interleave.
static SemaphoreHandle_t led_lock = nullptr;

static void xmos_write(uint8_t res, uint8_t cmd, const uint8_t* data, uint8_t n) {
  if (!xmos_dev) return;
  uint8_t buf[8];
  if (n > sizeof(buf) - 3) return;
  buf[0] = res;
  buf[1] = cmd;
  buf[2] = n;
  for (uint8_t i = 0; i < n; i++) buf[3 + i] = data[i];
  esp_err_t err = i2c_master_transmit(xmos_dev, buf, 3 + n, 100);
  if (err != ESP_OK) {
    // The ring is decoration. Losing it must never take the voice loop
    // down with it, so this is logged and dropped.
    ESP_LOGW(TAG, "LED write (cmd %u) failed: %s", cmd, esp_err_to_name(err));
  }
}

// What niles is doing, shown on the whole ring at once.
enum class Leds { Idle, Listening, Thinking, Speaking };

// LED_COLOR is a uint32, not three bytes — a three-byte write is
// rejected outright, which is how every state came out the same stock
// colour. The wire order is not documented; little-endian is the native
// order on both sides of this bus. If the colours come out mirrored
// (blue reading as orange), swap to big-endian here and nowhere else.
static void led_color(uint8_t r, uint8_t g, uint8_t b) {
  const uint32_t packed = (uint32_t)r << 16 | (uint32_t)g << 8 | (uint32_t)b;
  const uint8_t bytes[4] = {
      (uint8_t)(packed & 0xFF),
      (uint8_t)(packed >> 8 & 0xFF),
      (uint8_t)(packed >> 16 & 0xFF),
      (uint8_t)(packed >> 24 & 0xFF),
  };
  xmos_write(XMOS_RES_GPO, XMOS_CMD_LED_COLOR, bytes, sizeof(bytes));
}

static void leds_show(Leds state) {
  uint8_t rgb[3];
  uint8_t effect;
  switch (state) {
    case Leds::Listening:  // heard its name, capturing — steady white
      effect = LED_EFFECT_SOLID;
      rgb[0] = 255; rgb[1] = 255; rgb[2] = 255;
      break;
    case Leds::Thinking:  // waiting on niles — breathing, so the wait reads as work
      effect = LED_EFFECT_BREATHING;
      rgb[0] = 0; rgb[1] = 120; rgb[2] = 255;
      break;
    case Leds::Speaking:  // replying — steady, and a different colour from listening
      effect = LED_EFFECT_SOLID;
      rgb[0] = 0; rgb[1] = 180; rgb[2] = 90;
      break;
    case Leds::Idle:
    default:
      effect = LED_EFFECT_OFF;
      rgb[0] = 0; rgb[1] = 0; rgb[2] = 0;
      break;
  }
  // Colour first: setting the effect last means the ring never shows
  // the new effect in the old colour, however briefly.
  if (led_lock) xSemaphoreTake(led_lock, portMAX_DELAY);
  led_color(rgb[0], rgb[1], rgb[2]);
  xmos_write(XMOS_RES_GPO, XMOS_CMD_LED_EFFECT, &effect, 1);
  if (led_lock) xSemaphoreGive(led_lock);
}

static void leds_init() {
  led_lock = xSemaphoreCreateMutex();
  i2c_master_bus_config_t bus_cfg = {};
  bus_cfg.i2c_port = I2C_NUM_0;
  bus_cfg.sda_io_num = PIN_SDA;
  bus_cfg.scl_io_num = PIN_SCL;
  bus_cfg.clk_source = I2C_CLK_SRC_DEFAULT;
  bus_cfg.glitch_ignore_cnt = 7;
  bus_cfg.flags.enable_internal_pullup = true;

  i2c_master_bus_handle_t bus = nullptr;
  esp_err_t err = i2c_new_master_bus(&bus_cfg, &bus);
  if (err != ESP_OK) {
    ESP_LOGW(TAG, "I2C bus init failed (%s) — ring stays as it is", esp_err_to_name(err));
    return;
  }

  i2c_device_config_t dev_cfg = {};
  dev_cfg.dev_addr_length = I2C_ADDR_BIT_LEN_7;
  dev_cfg.device_address = XMOS_I2C_ADDR;
  dev_cfg.scl_speed_hz = 100000;
  err = i2c_master_bus_add_device(bus, &dev_cfg, &xmos_dev);
  if (err != ESP_OK) {
    ESP_LOGW(TAG, "XVF3800 not on I2C (%s) — ring stays as it is", esp_err_to_name(err));
    xmos_dev = nullptr;
    return;
  }

  uint8_t brightness = 40;  // full is glaring in a dark room
  xmos_write(XMOS_RES_GPO, XMOS_CMD_LED_BRIGHTNESS, &brightness, 1);
  leds_show(Leds::Idle);
  ESP_LOGI(TAG, "LED ring under our control (direction-of-arrival off)");
}

// ---- I2S: one port, both directions, always on ----
//
// The microphone and the speaker share this port. It used to carry one
// direction at a time -- tear the microphone down, play at the reply's
// rate, bring the microphone back -- so a satellite that was talking
// could not hear. A ringing timer could only be stopped in the gaps
// between chimes, and an answer could not be interrupted at all.
//
// Both directions on one port share one clock, so everything plays at the
// microphone's 16 kHz and play_stream() converts whatever arrives. The
// XVF3800 cancels its own speaker out of the microphone -- the audio sent
// to it here is its echo reference -- which is what lets the wake word be
// heard over the satellite's own voice.
static i2s_chan_handle_t tx_chan = nullptr;

static void i2s_init_duplex() {
  i2s_chan_config_t chan_cfg = I2S_CHANNEL_DEFAULT_CONFIG(I2S_NUM_0, I2S_ROLE_MASTER);
  // With nothing to play the port must send silence, not repeat the last
  // buffer it was given -- the tail of every reply, looping forever.
  chan_cfg.auto_clear_after_cb = true;
  ESP_ERROR_CHECK(i2s_new_channel(&chan_cfg, &tx_chan, &rx_chan));
  i2s_std_config_t std_cfg = {
      .clk_cfg = I2S_STD_CLK_DEFAULT_CONFIG(SAMPLE_RATE),
      .slot_cfg = I2S_STD_PHILIPS_SLOT_DEFAULT_CONFIG(I2S_DATA_BIT_WIDTH_32BIT,
                                                      I2S_SLOT_MODE_STEREO),
      .gpio_cfg = {
          .mclk = I2S_GPIO_UNUSED,
          .bclk = PIN_BCLK,
          .ws = PIN_WS,
          .dout = PIN_DOUT,
          .din = PIN_DIN,
          .invert_flags = {.mclk_inv = false, .bclk_inv = false, .ws_inv = false},
      },
  };
  ESP_ERROR_CHECK(i2s_channel_init_std_mode(tx_chan, &std_cfg));
  ESP_ERROR_CHECK(i2s_channel_init_std_mode(rx_chan, &std_cfg));
  ESP_ERROR_CHECK(i2s_channel_enable(tx_chan));
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
    int32_t raw = (int32_t)(i2s_buf[f * 2] >> 16);
    if (preroll) {
      preroll[preroll_pos] = (int16_t)raw;
      if (++preroll_pos >= PREROLL_SAMPLES) {
        preroll_pos = 0;
        preroll_full = true;
      }
    }
    int32_t s = raw * MIC_GAIN;
    if (s > 32767) s = 32767;
    else if (s < -32768) s = -32768;
    tail[f] = (int16_t)s;
  }
  for (int f = frames; f < STRIDE_SAMPLES; f++) tail[f] = 0;
}

// Defined below, with the rest of the socket helpers; declared here because
// the pre-roll belongs beside the ring it drains, not beside the socket.
static bool send_all(int sock, const void* buf, size_t len);

/// Send everything kept before the wake word, oldest first.
///
/// The ring is written continuously and never paused, so this runs before
/// the live loop starts reading I2S again; a slice arriving in between would
/// land in the ring and be sent twice.
static bool send_preroll(int sock) {
  if (!preroll) return true;
  int have = preroll_full ? PREROLL_SAMPLES : preroll_pos;
  int start = preroll_full ? preroll_pos : 0;
  char hdr[64];
  int16_t slice[STRIDE_SAMPLES];
  for (int sent = 0; sent < have; sent += STRIDE_SAMPLES) {
    int n = have - sent;
    if (n > STRIDE_SAMPLES) n = STRIDE_SAMPLES;
    for (int f = 0; f < n; f++) {
      int32_t g = preroll[(start + sent + f) % PREROLL_SAMPLES] * STREAM_GAIN;
      if (g > 32767) g = 32767;
      else if (g < -32768) g = -32768;
      slice[f] = (int16_t)g;
    }
    for (int f = n; f < STRIDE_SAMPLES; f++) slice[f] = 0;
    int pb = STRIDE_SAMPLES * (int)sizeof(int16_t);
    int hn = snprintf(hdr, sizeof(hdr),
                      "{\"type\":\"audio-chunk\",\"payload_length\":%d}\n", pb);
    if (!send_all(sock, hdr, hn) || !send_all(sock, slice, pb)) return false;
  }
  return true;
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

// Parse the integer right after `key` in a JSON header line (e.g. "\"rate\":").
static long json_int_after(const char* s, const char* key) {
  const char* p = strstr(s, key);
  if (!p) return -1;
  return atol(p + strlen(key));
}

// ---- Playback, on its own task ----
//
// Everything the satellite plays -- a reply, or something niles dialled in
// to say -- goes through the speaker task, so the wake loop never stops
// listening. Saying the wake word over it bumps play_gen, and whatever was
// playing gives way: that is barge-in, for a chime and an answer alike.
struct PlayJob {
  int sock;
  uint32_t gen;  // play_gen when queued; anything newer means "stop"
  bool reply;    // a reply waits for niles to think; a push does not
};
static QueueHandle_t play_q = nullptr;
static std::atomic<uint32_t> play_gen{0};
static std::atomic<bool> playing_now{false};

static bool superseded(const PlayJob& job) { return job.gen != play_gen.load(); }

// recv() that gives up when the job is superseded, rather than holding the
// speaker for the whole of niles's thinking time after somebody has already
// asked something else. Short socket timeouts, and an overall idle limit.
static int recv_or_abort(const PlayJob& job, void* buf, int n) {
  int idle_ms = 0;
  const int limit_ms = job.reply ? 10000 : 3000;  // niles needs time to think
  while (true) {
    int r = recv(job.sock, buf, n, 0);
    if (r > 0) return r;
    if (r == 0) return -1;
    if (errno != EAGAIN && errno != EWOULDBLOCK) return -1;
    if (superseded(job)) return -1;
    idle_ms += 100;
    if (idle_ms >= limit_ms) return -1;
  }
}

static int job_read_line(const PlayJob& job, char* buf, int max) {
  int idx = 0;
  while (idx < max - 1) {
    char c;
    if (recv_or_abort(job, &c, 1) < 0) return -1;
    if (c == '\n') break;
    buf[idx++] = c;
  }
  buf[idx] = 0;
  return idx;
}

static bool job_read_full(const PlayJob& job, uint8_t* buf, int n) {
  int got = 0;
  while (got < n) {
    int r = recv_or_abort(job, buf + got, n - got);
    if (r < 0) return false;
    got += r;
  }
  return true;
}

// Mono 16-bit in at any rate; stereo 32-bit out at SAMPLE_RATE, L=R.
// Linear interpolation, carried across chunks so the joins are seamless.
struct Resampler {
  float step = 1.0f;  // input samples per output sample
  float t = 0.0f;     // where the next output falls between prev and the next input
  int16_t prev = 0;
};
static constexpr int kTxFrames = 512;
static int32_t tx_buf[kTxFrames * 2];
static int tx_fill = 0;

static void tx_flush() {
  if (tx_fill == 0) return;
  size_t wrote = 0;
  i2s_channel_write(tx_chan, tx_buf, tx_fill * 2 * sizeof(int32_t), &wrote, portMAX_DELAY);
  tx_fill = 0;
}

static void tx_put(int16_t s) {
  int32_t v = (int32_t)s << 16;
  tx_buf[tx_fill * 2] = v;
  tx_buf[tx_fill * 2 + 1] = v;
  if (++tx_fill == kTxFrames) tx_flush();
}

static void resample_into_tx(Resampler& rs, const int16_t* in, int n) {
  for (int i = 0; i < n; i++) {
    const int16_t x = in[i];
    while (rs.t < 1.0f) {
      tx_put((int16_t)(rs.prev + (x - rs.prev) * rs.t));
      rs.t += rs.step;
    }
    rs.t -= 1.0f;
    rs.prev = x;
  }
}

// Play one Wyoming audio stream (audio-start{rate} / audio-chunk+PCM /
// audio-stop) from the job's socket. Returns false if it was cut off.
static bool play_stream(const PlayJob& job) {
  struct timeval tv = {.tv_sec = 0, .tv_usec = 100 * 1000};
  setsockopt(job.sock, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));

  char line[192];
  uint8_t pcm[1024];
  Resampler rs;
  bool started = false;

  while (!superseded(job)) {
    int len = job_read_line(job, line, sizeof(line));
    if (len < 0) {
      if (!superseded(job)) ESP_LOGW(TAG, "no reply / timeout");
      break;
    }
    if (strstr(line, "audio-start")) {
      long rate = json_int_after(line, "\"rate\":");
      if (rate <= 0) rate = 22050;
      rs = Resampler{};
      rs.step = (float)rate / (float)SAMPLE_RATE;
      ESP_LOGI(TAG, "playing, audio-start rate=%ld", rate);
      if (!superseded(job)) leds_show(Leds::Speaking);
      playing_now = true;
      started = true;
    } else if (strstr(line, "audio-chunk")) {
      long rem = json_int_after(line, "\"payload_length\":");
      if (rem <= 0) continue;
      while (rem > 0) {
        int want = rem < (long)sizeof(pcm) ? (int)rem : (int)sizeof(pcm);
        if (!job_read_full(job, pcm, want)) {
          rem = 0;
          break;
        }
        if (started && !superseded(job))
          resample_into_tx(rs, reinterpret_cast<const int16_t*>(pcm),
                           want / (int)sizeof(int16_t));
        rem -= want;
      }
    } else if (strstr(line, "audio-stop")) {
      break;
    }
  }
  const bool cut_off = superseded(job);
  if (!cut_off) tx_flush();
  tx_fill = 0;
  playing_now = false;
  if (started) ESP_LOGI(TAG, "%s", cut_off ? "playback cut off by the wake word" : "playback done");
  return !cut_off;
}

// After wake detection, open a Wyoming TCP connection to niles and stream the
// spoken command as mono 16 kHz 16-bit PCM (raw downmix, no MIC_GAIN — cleaner
// for STT), ending on silence (energy VAD) or a hard cap.
// ---- Niles calling us ----
//
// The satellite has only ever spoken first: connect, stream, hear the
// reply, hang up. So niles could never start a conversation — a timer
// could fire and there was nowhere to say so, because an idle
// satellite holds no connection and the peer index correctly reports
// that it has none.
//
// A listening socket is the smaller half of fixing that. The wake loop
// already runs every 10 ms, so a non-blocking accept costs a branch;
// the alternative — holding an outbound connection open — needs
// keepalives, reconnection, and stale-socket handling for the same
// result.
//
// Deliberately playback only. It does NOT capture afterwards: a
// microphone that opens because *niles* decided to talk is a
// microphone that opens without anyone saying the wake word, and an
// earlier capture of an unrelated television is exactly what that
// looks like when it goes wrong. To answer a ringing timer, say the
// wake word like anything else.
static int push_listener = -1;

static void push_listener_init() {
  push_listener = socket(AF_INET, SOCK_STREAM, IPPROTO_IP);
  if (push_listener < 0) {
    ESP_LOGE(TAG, "push listener socket() failed");
    return;
  }
  int yes = 1;
  setsockopt(push_listener, SOL_SOCKET, SO_REUSEADDR, &yes, sizeof(yes));

  struct sockaddr_in addr = {};
  addr.sin_family = AF_INET;
  addr.sin_addr.s_addr = htonl(INADDR_ANY);
  addr.sin_port = htons(NILES_PUSH_PORT);
  if (bind(push_listener, reinterpret_cast<struct sockaddr*>(&addr), sizeof(addr)) != 0) {
    ESP_LOGE(TAG, "push listener bind failed (errno %d)", errno);
    close(push_listener);
    push_listener = -1;
    return;
  }
  if (listen(push_listener, 1) != 0) {
    ESP_LOGE(TAG, "push listener listen failed (errno %d)", errno);
    close(push_listener);
    push_listener = -1;
    return;
  }
  // Non-blocking: the wake loop polls it and must never stall there,
  // or the satellite stops hearing its own name.
  fcntl(push_listener, F_SETFL, O_NONBLOCK);
  ESP_LOGI(TAG, "listening for niles pushes on :%d", NILES_PUSH_PORT);
}

// Everything that plays, one stream at a time: replies handed over by the
// wake loop, and anything niles dials in to say.
static void speaker_task(void*) {
  while (true) {
    PlayJob job;
    if (xQueueReceive(play_q, &job, pdMS_TO_TICKS(20)) != pdTRUE) {
      if (push_listener < 0) continue;
      int client = accept(push_listener, nullptr, nullptr);
      if (client < 0) continue;  // EWOULDBLOCK -- the normal case
      ESP_LOGI(TAG, ">>> niles is calling <<<");
      // The accepted socket inherits O_NONBLOCK on some stacks; the reads
      // rely on SO_RCVTIMEO instead.
      fcntl(client, F_SETFL, 0);
      job = PlayJob{client, play_gen.load(), false};
    }
    const bool finished = play_stream(job);
    close(job.sock);
    // Cut off means somebody is talking to it now, and the wake loop owns
    // the ring until they have finished.
    if (finished) leds_show(Leds::Idle);
  }
}

// Returns true if the socket went to the speaker task to await a reply.
static bool stream_utterance(float wake_avg, uint32_t gen) {
  struct sockaddr_in dest = {};
  dest.sin_family = AF_INET;
  dest.sin_port = htons(NILES_PORT);
  if (inet_pton(AF_INET, NILES_HOST, &dest.sin_addr) != 1) {
    ESP_LOGE(TAG, "bad NILES_HOST '%s'", NILES_HOST);
    return false;
  }
  int sock = socket(AF_INET, SOCK_STREAM, IPPROTO_IP);
  if (sock < 0) {
    ESP_LOGE(TAG, "socket() failed");
    return false;
  }
  if (connect(sock, reinterpret_cast<struct sockaddr*>(&dest), sizeof(dest)) != 0) {
    ESP_LOGE(TAG, "connect %s:%d failed (errno %d)", NILES_HOST, NILES_PORT, errno);
    close(sock);
    return false;
  }

  // The probability that fired rides along with the audio. It only
  // existed on the serial console before, so tuning the threshold meant
  // carrying the satellite to a desk and hoping the television performed.
  // niles logs it beside the transcript; a week of the real room then
  // says where the bar belongs.
  char start[192];
  int sn = snprintf(start, sizeof(start),
                    "{\"type\":\"audio-start\",\"data\":{\"rate\":16000,\"width\":2,"
                    "\"channels\":1,\"wake_avg\":%.3f}}\n",
                    (double)wake_avg);
  if (!send_all(sock, start, sn)) {
    close(sock);
    return false;
  }

  // Everything heard just before the wake word, before anything live:
  // the sound that triggered this is the one worth having.
  if (!send_preroll(sock)) {
    close(sock);
    return false;
  }

  // End-of-speech is measured RELATIVE to how loud this sentence is,
  // not against a fixed level.
  //
  // A constant cannot work in a room that is sometimes quiet and
  // sometimes has a television in it. At STOP_RMS = 60 with ambient
  // ~15-24 it was fine in a quiet room and impossible with the TV on:
  // the floor never fell below 60, the capture ran to MAX_FRAMES every
  // time, and Whisper was handed the sentence plus two and a half
  // seconds of room. That is what turned "what's the weather like" into
  // "what's the word I like".
  //
  // Speech runs into the thousands and background sits in the tens, so
  // the *ratio* is stable even though neither level is. Stop when the
  // level falls to a fraction of the loudest speech so far in this
  // utterance, clamped at both ends: a floor so a whispered sentence
  // doesn't set the bar at nothing, and a ceiling so a shouted one
  // doesn't set it so high that the trailing words are cut off.
  static const int STOP_DIVISOR = 16;
  static const int STOP_RMS_MIN = 60;  // the old fixed value, now the floor
  static const int STOP_RMS_MAX = 400;
  static const int HANGOVER_FRAMES = 35; // ~350 ms of trailing silence ends it
  static const int MAX_FRAMES = 400;     // ~4 s hard cap (was 6 s — too slow)

  // Speech-onset gate: don't start the end-of-utterance countdown until speech
  // actually BEGINS. Otherwise a slow start after the wake word ends the
  // capture on leading silence, and niles transcribes a near-empty clip (which
  // Whisper turns into hallucinated stock phrases). Wait up to
  // ONSET_TIMEOUT_FRAMES for energy to cross ONSET_RMS (well above ambient ~24,
  // below normal speech); if the user never speaks, abort without a real clip.
  static const int ONSET_RMS = 120;
  static const int ONSET_TIMEOUT_FRAMES = 250; // ~2.5 s to start talking

  // The XVF3800 downmix is low-level: raw command speech peaks only ~700-4600
  // (~2-14% of full scale). Whisper HALLUCINATES on near-silent audio (it
  // invents stock phrases — e.g. Korean news intros). Amplify the outgoing
  // PCM so it lands at a healthy level; speech ~3000 raw * 6 ~= 55% FS, and
  // the loudest observed (~4600) stays just under clipping. VAD still measures
  // the RAW level so STOP_RMS tuning is unaffected.

  int16_t slice[STRIDE_SAMPLES];
  char hdr[64];
  int silent = 0, total = 0, lead = 0;
  bool started = false;
  long emin = 1 << 30, emax = 0;
  // The loudest speech seen since onset, which sets the bar for what
  // counts as silence afterwards.
  long speech_peak = 0;
  int stop_rms = STOP_RMS_MIN;
  while (total < MAX_FRAMES) {
    size_t got = 0;
    i2s_channel_read(rx_chan, i2s_buf, sizeof(i2s_buf), &got, portMAX_DELAY);
    int frames = got / (sizeof(int32_t) * 2);
    if (frames > STRIDE_SAMPLES) frames = STRIDE_SAMPLES;
    long sum = 0;
    for (int f = 0; f < frames; f++) {
      int32_t raw = (int32_t)(i2s_buf[f * 2] >> 16); // raw mono
      sum += raw < 0 ? -raw : raw;                   // VAD measures raw level
      int32_t g = raw * STREAM_GAIN;                 // amplify for STT
      if (g > 32767) g = 32767;
      else if (g < -32768) g = -32768;
      slice[f] = (int16_t)g;
    }
    for (int f = frames; f < STRIDE_SAMPLES; f++) slice[f] = 0;
    long energy = frames ? sum / frames : 0;
    if (energy < emin) emin = energy;
    if (energy > emax) emax = energy;

    // Stream from the start so we keep a little pre-roll (no clipped first
    // word), but only the post-onset silence counts toward the endpoint.
    int pb = STRIDE_SAMPLES * (int)sizeof(int16_t);
    int n = snprintf(hdr, sizeof(hdr), "{\"type\":\"audio-chunk\",\"payload_length\":%d}\n", pb);
    if (!send_all(sock, hdr, n) || !send_all(sock, slice, pb)) break;
    total++;

    if (!started) {
      if (energy >= ONSET_RMS) {
        started = true;
      } else if (++lead >= ONSET_TIMEOUT_FRAMES) {
        ESP_LOGW(TAG, "no speech after wake — aborting capture");
        break;
      }
    } else {
      // Recompute the bar from the loudest speech so far: how loud this
      // person is right now is the only thing that says what silence
      // sounds like afterwards.
      if (energy > speech_peak) {
        speech_peak = energy;
        long scaled = speech_peak / STOP_DIVISOR;
        if (scaled < STOP_RMS_MIN) scaled = STOP_RMS_MIN;
        if (scaled > STOP_RMS_MAX) scaled = STOP_RMS_MAX;
        stop_rms = (int)scaled;
      }
      if (energy < stop_rms) {
        if (++silent >= HANGOVER_FRAMES) break;
      } else {
        silent = 0;
      }
    }
  }

  const char* stop = "{\"type\":\"audio-stop\"}\n";
  send_all(sock, stop, strlen(stop));
  // Nothing more to say; from here the wait is niles's.
  leds_show(Leds::Thinking);
  ESP_LOGI(TAG,
           "utterance streamed (%d frames, ~%d ms, %s%s) energy[min=%ld max=%ld] "
           "speech_peak=%ld stop_rms=%d",
           total, total * 10, started ? "spoke" : "no-speech",
           total >= MAX_FRAMES ? ", HIT CAP" : "", emin, emax, speech_peak, stop_rms);

  // The reply comes back on this socket. The speaker task plays it, so the
  // wake loop is listening again while niles thinks and while it talks.
  PlayJob job{sock, gen, true};
  if (xQueueSend(play_q, &job, 0) != pdTRUE) {
    close(sock);
    return false;
  }
  return true;
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
  // PSRAM: 32 KB of internal SRAM is worth more elsewhere, and this is
  // written once per 10 ms frame, nowhere near a bottleneck.
  preroll = (int16_t*)heap_caps_malloc(PREROLL_SAMPLES * sizeof(int16_t), MALLOC_CAP_SPIRAM);
  if (!preroll) {
    ESP_LOGW(TAG, "no PSRAM for the pre-roll; wakes will be sent without it");
  }
  i2s_init_duplex();
  leds_init();
  push_listener_init();
  play_q = xQueueCreate(2, sizeof(PlayJob));
  // The other core: the wake loop keeps this one busy, and playback must
  // never make it late for a frame.
  xTaskCreatePinnedToCore(speaker_task, "speaker", 6144, nullptr, 5, nullptr, 1);
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
  float hb_maxprob = 0.0f; // max single-inference probability this heartbeat
  float hb_maxavg = 0.0f;  // max averaged probability -- what now decides
  bool hb_played = false;  // whether the speaker played during this heartbeat

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
        // microWakeWord's probability output is an UNSIGNED byte [0,255]
        // (255 ~= 1.0), but TFLM types the tensor int8. Reading it signed
        // wraps high-confidence detections (byte > 127) to negative, hiding
        // every strong hit and capping the visible prob at ~0.496 (=127/256).
        // Read it unsigned.
        float prob = output->data.uint8[0] * output->params.scale;
        if (prob > hb_maxprob) hb_maxprob = prob;

        // The decision is the average of the last WINDOW_AVG inferences. A
        // single loud coincidence cannot carry it; the wake word, which
        // stays high across several slices, can.
        ring[idx] = prob;
        idx = (idx + 1) % WINDOW_AVG;
        float avg = 0.0f;
        for (int i = 0; i < WINDOW_AVG; i++) avg += ring[i];
        avg /= WINDOW_AVG;
        if (avg > hb_maxavg) hb_maxavg = avg;

        int64_t now_ms = esp_log_timestamp();
        if (avg >= AVG_CUTOFF && now_ms - last_fire_ms > 1500) {
          last_fire_ms = now_ms;
          ESP_LOGI(TAG,
                   ">>> WAKE WORD DETECTED (avg=%.3f, last=%.3f) — streaming command <<<",
                   (double)avg, (double)prob);
          // Before the socket: the ring is the acknowledgement that it
          // heard its name, and it has to arrive while you are still
          // speaking, not after the network has had its turn.
          // Whatever is playing gives way: a chime, or an answer being
          // talked over.
          const uint32_t gen = ++play_gen;
          leds_show(Leds::Listening);
          if (!stream_utterance(avg, gen)) leds_show(Leds::Idle);
          // Reset wake state so stale slices don't immediately re-fire.
          // The ring included: the average that just fired would otherwise
          // still be most of the way to firing again.
          slot = 0;
          memset(ring, 0, sizeof(ring));
          idx = 0;
          last_fire_ms = esp_log_timestamp();
        }
      }
    }

    // ~1 s heartbeat (each iter is 10 ms): the MAX mic peak, MAX feature, and
    // MAX probability seen this second. featmax jumps when you speak; maxprob
    // jumps when you say "nyles". Use this to set PROB_CUTOFF (see above).
    // play=1 marks a second in which the speaker was playing: how much of
    // the satellite's own voice survives the XVF3800's echo cancellation is
    // read off maxavg on those lines.
    if (playing_now) hb_played = true;
    if (++iter % 100 == 0) {
      ESP_LOGI(TAG, "peak=%ld featmax=%d maxprob=%.3f maxavg=%.3f play=%d", (long)hb_peak,
               hb_featmax, (double)hb_maxprob, (double)hb_maxavg, hb_played ? 1 : 0);
      hb_played = false;
      hb_peak = 0;
      hb_maxavg = 0.0f;
      hb_featmax = -128;
      hb_maxprob = 0.0f;
    }
  }
}
