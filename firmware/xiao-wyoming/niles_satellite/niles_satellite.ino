// niles voice satellite — reSpeaker XVF3800 + XIAO ESP32-S3
//
// Energy-VAD "always listen" firmware: no wake word, no push-to-talk.
// Continuously reads I2S from the XVF3800, and when room audio is loud
// enough for long enough, opens a Wyoming TCP connection to niles,
// streams the utterance as mono 16 kHz 16-bit PCM, then plays niles'
// spoken reply back out over the same connection (the XVF3800 handles
// acoustic echo cancellation so the reply doesn't retrigger capture).
//
// The XVF3800's side button is a hardware MUTE (privacy): muting kills
// the mic, so energy drops to ~0 and VAD never triggers — it is NOT a
// talk button. See firmware/xiao-wyoming/README.md for the full
// architecture, the wire format, and bring-up notes.
//
// Prereqs (Arduino IDE):
//   - Board: XIAO_ESP32S3   USB CDC On Boot: Enabled   PSRAM: OPI PSRAM
//   - Library: arduino-audio-tools (pschatzmann) — install via
//     Sketch > Include Library > Add .ZIP Library (NOT in Library Mgr)
//   - XVF3800 flashed with Seeed's stock I2S DFU firmware (2ch/32-bit)
//   - Flash via the XIAO module's OWN USB-C port, not the carrier board's

#include "AudioTools.h"
#include <WiFi.h>

// ---- EDIT THESE ----
const char* WIFI_SSID = "YOUR_WIFI_SSID";
const char* WIFI_PASS = "YOUR_WIFI_PASSWORD";
const char* NILES_HOST = "192.168.10.173";   // the host running `niles serve`
const uint16_t NILES_PORT = 10300;           // [wyoming] bind port
// --------------------

// ---- VAD tuning: watch the "idle energy=" / "speech energy=" prints and adjust ----
// These are per-unit/per-room. On the first satellite: ambient idle ~7,
// so START=40 / STOP=20 with hysteresis worked. If nothing ever triggers
// they're too high; if room noise triggers it they're too low. Reject a
// transient click (e.g. the mute button) with START_FRAMES, not a ceiling
// — a ceiling would also drop loud speech.
long START_RMS = 40;         // mean |sample| to START capturing
long STOP_RMS  = 20;         // below this = silence (hysteresis)
const int START_FRAMES   = 3;    // ~96 ms of speech to trigger
const int HANGOVER_FRAMES= 25;   // ~800 ms of silence to end
const int PREROLL_FRAMES = 8;    // ~256 ms kept before onset (no clipped first word)
const int MAX_FRAMES     = 600;  // ~19 s hard cap per utterance
// ----------------------------------------------------------------------------------

I2SStream i2s;
WiFiClient client;
static const int MONO_SAMPLES = 512;
int32_t i2sBuf[MONO_SAMPLES * 2];
int16_t monoBuf[MONO_SAMPLES];
int16_t preroll[PREROLL_FRAMES][MONO_SAMPLES];
int prerollHead = 0, prerollCount = 0;
int currentRate = 0;

void i2sStart(int rate) {
  if (currentRate == rate) return;
  if (currentRate != 0) i2s.end();
  auto cfg = i2s.defaultConfig(RXTX_MODE);
  AudioInfo info(rate, 2, 32);
  cfg.copyFrom(info);
  cfg.pin_bck = 8; cfg.pin_ws = 7; cfg.pin_data = 44; cfg.pin_data_rx = 43;
  cfg.is_master = true;
  i2s.begin(cfg);
  currentRate = rate;
}
int readLine(char* b, int maxLen, unsigned long to) {
  int idx = 0; unsigned long s = millis();
  while (millis() - s < to) {
    if (client.available()) { char c = client.read(); if (c=='\n'){b[idx]=0;return idx;} if(idx<maxLen-1)b[idx++]=c; }
    else if (!client.connected()) return -1; else delay(1);
  } return -1;
}
bool readFully(uint8_t* b, int n, unsigned long to) {
  int got=0; unsigned long s=millis();
  while (got<n && millis()-s<to) {
    if (client.available()){int r=client.read(b+got,n-got); if(r>0){got+=r;s=millis();}}
    else if(!client.connected()) return false; else delay(1);
  } return got==n;
}
long parseIntAfter(const char* l, const char* k){const char* p=strstr(l,k); if(!p)return -1; return atol(p+strlen(k));}
void ensureWifi(){
  if (WiFi.status()==WL_CONNECTED) return;
  WiFi.begin(WIFI_SSID, WIFI_PASS);
  Serial.print("WiFi"); while(WiFi.status()!=WL_CONNECTED){delay(500);Serial.print(".");}
  Serial.printf(" OK %s\n", WiFi.localIP().toString().c_str());
}
long readFrame() {
  size_t got = i2s.readBytes((uint8_t*)i2sBuf, sizeof(i2sBuf));
  int frames = got / (sizeof(int32_t)*2);
  long sum = 0;
  for (int f=0; f<frames; f++){ int16_t s=(int16_t)(i2sBuf[f*2]>>16); monoBuf[f]=s; sum += s<0?-s:s; }
  for (int f=frames; f<MONO_SAMPLES; f++) monoBuf[f]=0;
  return frames>0 ? sum/frames : 0;
}
void sendChunk(const int16_t* buf, int samples) {
  int pb = samples*sizeof(int16_t);
  char hdr[64]; snprintf(hdr,sizeof(hdr),"{\"type\":\"audio-chunk\",\"payload_length\":%d}\n",pb);
  client.print(hdr); client.write((const uint8_t*)buf, pb);
}
void playReply() {
  char line[128]; uint8_t pcm[2048]; int32_t frame[2];
  while (true) {
    int len = readLine(line,sizeof(line),15000);
    if (len<0){Serial.println("No reply / timeout.");return;}
    if (strstr(line,"audio-start")){ long r=parseIntAfter(line,"\"rate\":"); if(r<=0)r=22050; Serial.printf("Reply rate=%ld\n",r); i2sStart(r); }
    else if (strstr(line,"audio-chunk")){
      long rem=parseIntAfter(line,"\"payload_length\":"); if(rem<=0)return;
      while(rem>0){ int want=rem<(long)sizeof(pcm)?(int)rem:(int)sizeof(pcm);
        if(!readFully(pcm,want,5000)){Serial.println("chunk timeout");return;}
        int n=want/2; int16_t* s=(int16_t*)pcm;
        for(int i=0;i<n;i++){int32_t v=((int32_t)s[i])<<16;frame[0]=v;frame[1]=v;i2s.write((uint8_t*)frame,sizeof(frame));}
        rem-=want; }
    } else if (strstr(line,"audio-stop")){Serial.println("Reply done.");return;}
  }
}
void setup() {
  Serial.begin(115200);
  i2sStart(16000);
  ensureWifi();
  Serial.println("Always-listening (VAD). Watch energy= to tune. Just speak a command.");
}
void loop() {
  long energy = readFrame();
  memcpy(preroll[prerollHead], monoBuf, sizeof(monoBuf));
  prerollHead = (prerollHead+1)%PREROLL_FRAMES;
  if (prerollCount<PREROLL_FRAMES) prerollCount++;

  static int aboveCount = 0; static unsigned long lastPrint = 0;
  if (millis()-lastPrint > 500) { Serial.printf("idle energy=%ld\n", energy); lastPrint=millis(); }
  if (energy > START_RMS) aboveCount++; else aboveCount = 0;
  if (aboveCount < START_FRAMES) return;
  aboveCount = 0;

  ensureWifi(); i2sStart(16000);
  if (!client.connect(NILES_HOST, NILES_PORT)) { Serial.println("TCP FAILED"); return; }
  Serial.printf(">>> speech energy=%ld, streaming <<<\n", energy);
  client.print("{\"type\":\"audio-start\",\"data\":{\"rate\":16000,\"width\":2,\"channels\":1}}\n");
  for (int i=0;i<prerollCount;i++){ int idx=(prerollHead-prerollCount+i+PREROLL_FRAMES*2)%PREROLL_FRAMES; sendChunk(preroll[idx],MONO_SAMPLES); }

  int silence = 0, total = prerollCount;
  while (silence < HANGOVER_FRAMES && total < MAX_FRAMES) {
    long e = readFrame(); sendChunk(monoBuf, MONO_SAMPLES); total++;
    if (e < STOP_RMS) silence++; else silence = 0;
  }
  client.print("{\"type\":\"audio-stop\"}\n"); client.flush();
  Serial.println("end of speech, waiting for niles...");
  playReply();
  client.stop(); i2sStart(16000);
  prerollHead = 0; prerollCount = 0;
  Serial.println("ready.");
}
