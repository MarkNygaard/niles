//! The command line: what `niles` takes and the arguments of each subcommand.

use crate::manifest::GenerateManifestArgs;
use crate::plugins;
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "niles", about = "AI-first home automation system", version)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Commands,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    /// Run the full Niles stack in one process: Z2M source, HTTP API,
    /// voice dispatch (Wyoming + STT + intent), and the ambient-lighting
    /// curve, all sharing one device registry and one manual-mode tracker.
    Serve(ServeArgs),
    /// Configuration utilities.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Connect to MQTT and print received messages (for development).
    MqttTap(MqttTapArgs),
    /// Discover devices via Z2M and print add/remove/state-change events.
    Discover(DiscoverArgs),
    /// Send a Z2M set-command to a device (dev tool).
    Set(SetArgs),
    /// Run the Z2M source + HTTP API together. Devices land in the
    /// registry as Z2M reports them, and `curl http://<bind>/devices`
    /// returns the current snapshot.
    Api(ApiArgs),
    /// Run the Wyoming server and log every incoming event from
    /// connected satellites (dev tool — no STT / TTS yet).
    WyomingTap(WyomingTapArgs),
    /// Transcribe an audio file via the configured STT provider
    /// (ElevenLabs Scribe by default). The file is uploaded as-is —
    /// supported formats are WAV, MP3, FLAC, OGG, M4A, etc.
    Transcribe(TranscribeArgs),
    /// Synthesize speech via the configured TTS provider (Piper).
    /// Writes the resulting WAV to the path given by `--out`.
    Synthesize(SynthesizeArgs),
    /// One-shot chat with the configured Tier 1 LLM (Cerebras
    /// gpt-oss-120b by default). Sends `prompt` as a single user message with no tools
    /// and prints the model's reply to stdout. Manual-verification
    /// path — voice loop integration comes in a later PR.
    Chat(ChatArgs),
    /// Run the Wyoming server, accumulate each satellite utterance
    /// between `audio-start` and `audio-stop`, send it to the STT
    /// provider, and print the transcript. (Dev tool — no intent
    /// dispatch or TTS yet.)
    VoiceTap(VoiceTapArgs),
    /// Same audio pipeline as `voice-tap`, plus parse each
    /// transcript through the Tier 0 intent router and publish MQTT
    /// `set` commands for matched intents. Connects to MQTT and
    /// populates the device registry from Z2M, so room references
    /// like "kitchen" resolve to real devices. Pass `--dry-run` to
    /// match and log without publishing.
    VoiceDispatch(VoiceDispatchArgs),
    /// Enroll a speaker: extract a voice embedding from a 16 kHz mono
    /// 16-bit WAV and store it for per-utterance speaker identification.
    Enroll(EnrollArgs),
    /// Run the ambient-lighting curve driver: every minute, compute
    /// the curve's brightness + color-temperature target and publish
    /// it to every currently-on light. Per the architecture the curve
    /// never turns lights on or off — it only adjusts ones already
    /// on. In standalone mode no flags are set, so the curve runs for
    /// all on-lights; use `niles serve` for manual-mode integration.
    Lighting(LightingArgs),
    /// Regenerate MANIFEST.md from features.toml. Use --check to
    /// verify the committed MANIFEST.md is in sync (CI mode).
    GenerateManifest(GenerateManifestArgs),
    /// Third-party plugins: check one, or run one against a stand-in
    /// house while writing it.
    Plugin {
        #[command(subcommand)]
        action: plugins::PluginAction,
    },
}

#[derive(Subcommand)]
pub(crate) enum ConfigAction {
    /// Validate the Niles config file.
    Validate(ConfigValidateArgs),
}

#[derive(Args)]
pub(crate) struct ConfigValidateArgs {
    /// Path to the Niles config file.
    #[arg(short, long, default_value = "niles.toml")]
    pub(crate) config: PathBuf,
}

#[derive(Args)]
pub(crate) struct MqttTapArgs {
    /// Path to the Niles config file.
    #[arg(short, long, default_value = "niles.toml")]
    pub(crate) config: PathBuf,
    /// MQTT topic to subscribe to (wildcards `+` and `#` supported).
    #[arg(short, long, default_value = "zigbee2mqtt/#")]
    pub(crate) topic: String,
}

#[derive(Args)]
pub(crate) struct DiscoverArgs {
    /// Path to the Niles config file.
    #[arg(short, long, default_value = "niles.toml")]
    pub(crate) config: PathBuf,
}

#[derive(Args)]
pub(crate) struct SetArgs {
    /// Path to the Niles config file.
    #[arg(short, long, default_value = "niles.toml")]
    pub(crate) config: PathBuf,
    /// Target device as "<room>/<device>" (source prefix `z2m:` is added).
    pub(crate) device: String,
    /// Turn on (`--on true`), off (`--on false`), or leave unchanged.
    #[arg(long)]
    pub(crate) on: Option<bool>,
    /// Brightness percent (0–100).
    #[arg(long, value_parser = clap::value_parser!(u8).range(0..=100))]
    pub(crate) brightness: Option<u8>,
    /// Color temperature in Kelvin (typical lighting range 1000–20000).
    #[arg(long, value_parser = clap::value_parser!(u16).range(1000..=20000))]
    pub(crate) kelvin: Option<u16>,
    /// Print the message that would be published without actually sending.
    #[arg(long)]
    pub(crate) dry_run: bool,
}

#[derive(Args)]
pub(crate) struct ApiArgs {
    /// Path to the Niles config file.
    #[arg(short, long, default_value = "niles.toml")]
    pub(crate) config: PathBuf,
}

#[derive(Args)]
pub(crate) struct WyomingTapArgs {
    /// Path to the Niles config file.
    #[arg(short, long, default_value = "niles.toml")]
    pub(crate) config: PathBuf,
}

#[derive(Args)]
pub(crate) struct ChatArgs {
    /// Path to the Niles config file.
    #[arg(short, long, default_value = "niles.toml")]
    pub(crate) config: PathBuf,
    /// Prompt to send as a single user message.
    pub(crate) prompt: String,
}

#[derive(Args)]
pub(crate) struct TranscribeArgs {
    /// Path to the Niles config file.
    #[arg(short, long, default_value = "niles.toml")]
    pub(crate) config: PathBuf,
    /// Audio file to transcribe (WAV, MP3, FLAC, OGG, M4A, ...).
    pub(crate) audio: PathBuf,
}

#[derive(Args)]
pub(crate) struct SynthesizeArgs {
    /// Path to the Niles config file.
    #[arg(short, long, default_value = "niles.toml")]
    pub(crate) config: PathBuf,
    /// Text to synthesize.
    pub(crate) text: String,
    /// Path to write the resulting WAV file.
    #[arg(short, long)]
    pub(crate) out: PathBuf,
    /// Override the default voice from the [tts] config.
    #[arg(long)]
    pub(crate) voice: Option<String>,
}

#[derive(Args)]
pub(crate) struct VoiceTapArgs {
    /// Path to the Niles config file.
    #[arg(short, long, default_value = "niles.toml")]
    pub(crate) config: PathBuf,
}

#[derive(Args)]
pub(crate) struct VoiceDispatchArgs {
    /// Path to the Niles config file.
    #[arg(short, long, default_value = "niles.toml")]
    pub(crate) config: PathBuf,
    /// Match and log intents but skip the actual MQTT publish.
    /// Useful for confirming the audio + intent pipeline against a
    /// real broker-populated registry without affecting devices.
    #[arg(long)]
    pub(crate) dry_run: bool,
}

#[derive(Args)]
pub(crate) struct LightingArgs {
    /// Path to the Niles config file.
    #[arg(short, long, default_value = "niles.toml")]
    pub(crate) config: PathBuf,
    /// Compute and log the curve target each tick, but don't publish.
    #[arg(long)]
    pub(crate) dry_run: bool,
    /// Override the default 60-second tick interval. Mostly useful
    /// for development — the curve is integer-minute discretized,
    /// so polling faster than once a minute doesn't change values.
    #[arg(long, default_value_t = 60)]
    pub(crate) tick_seconds: u64,
}

#[derive(Args)]
pub(crate) struct EnrollArgs {
    /// Path to the Niles config file.
    #[arg(short, long, default_value = "niles.toml")]
    pub(crate) config: PathBuf,
    /// Speaker slug (a-z, 0-9, hyphens), e.g. `mark`.
    #[arg(long)]
    pub(crate) name: String,
    /// WAV file (16 kHz, mono, 16-bit) of the speaker talking.
    #[arg(long)]
    pub(crate) audio: PathBuf,
}

#[derive(Args)]
pub(crate) struct ServeArgs {
    /// A config file to start from. Optional: without one Niles runs on
    /// its defaults and what the app has saved in the database, which is
    /// everything a household sets. A file named here must exist.
    #[arg(short, long)]
    pub(crate) config: Option<PathBuf>,
    /// Match and log intents but skip the actual MQTT publish.
    #[arg(long)]
    pub(crate) dry_run: bool,
    /// Override the default 60-second curve tick interval.
    #[arg(long, default_value_t = 60)]
    pub(crate) tick_seconds: u64,
}
