//! What Niles has been taught to talk to.
//!
//! Not a config section — a list of facts about the outside world that
//! belong to the code rather than to anybody's file. Whether Niles can
//! use Groq is decided by the client it ships with; where Groq's API
//! lives has exactly one answer; and which models it serves is
//! something a person should pick from rather than be asked to
//! remember.
//!
//! Keeping the list here rather than in the API crate means the shipped
//! defaults can read from it. `[stt]` with nothing in it has to name a
//! model, and that model and the one offered in the dropdown must be
//! the same string — which they now are by construction rather than by
//! somebody noticing.

use crate::providers::Role;

/// How a thing is set up, which decides what its card looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// An inference account: one endpoint, one key, and the roles it
    /// can serve. Adding it appends to `[[providers]]`.
    Provider,
    /// Anything with its own wiring — an OAuth flow, a webhook, a
    /// broker.
    Service,
}

/// Where it sits on the Integrations page.
///
/// By what it does for the house rather than by how it is set up: a
/// newcomer looks for "music", not for "OAuth service".
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    Essentials,
    Music,
    Presence,
    Calendars,
    Shopping,
    Assistants,
}

/// How much a house misses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Importance {
    /// Niles does not do its job without it — or, for a provider,
    /// without one of the providers that serve the same role.
    Required,
    /// Noticeably better with it, for a house that has the thing.
    Recommended,
    Optional,
}

/// A group as the page heads it.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct GroupInfo {
    pub id: Group,
    pub label: &'static str,
    pub blurb: &'static str,
    /// What the group as a whole has to have, when that is not "each of
    /// these". The speech and AI providers are "one of", and a Required
    /// badge on each of them alone would say "all three".
    pub requirement: Option<&'static str>,
}

/// The groups, in the order the page shows them. Essentials first, so
/// a new install can work down the page.
pub const GROUPS: &[GroupInfo] = &[
    GroupInfo {
        id: Group::Essentials,
        label: "Essentials",
        blurb: "What Niles needs to reach your lights, hear what you say and answer.",
        requirement: Some(
            "Niles needs one speech-to-text provider and one AI provider. Groq does both on one account.",
        ),
    },
    GroupInfo {
        id: Group::Music,
        label: "Music & TV",
        blurb: "Speakers, music and the television, by voice.",
        requirement: None,
    },
    GroupInfo {
        id: Group::Presence,
        label: "Presence & climate",
        blurb: "Who is home, and the heating.",
        requirement: None,
    },
    GroupInfo {
        id: Group::Calendars,
        label: "Calendars",
        blurb: "Each person's calendar, read out for whoever asks. iCloud needs nothing here: each person connects it from their own profile.",
        requirement: None,
    },
    GroupInfo {
        id: Group::Shopping,
        label: "Shopping",
        blurb: "Groceries, from the shopping list to the door.",
        requirement: None,
    },
    GroupInfo {
        id: Group::Assistants,
        label: "Assistants & tools",
        blurb: "Claude or ChatGPT in the chat, on your own subscription, and work handed to Niles from Linear.",
        requirement: None,
    },
];

impl Group {
    pub fn info(self) -> &'static GroupInfo {
        GROUPS
            .iter()
            .find(|g| g.id == self)
            .expect("every group is in GROUPS")
    }

    /// Where it comes on the page.
    pub fn position(self) -> usize {
        GROUPS
            .iter()
            .position(|g| g.id == self)
            .expect("every group is in GROUPS")
    }
}

/// Which API a provider speaks.
///
/// Every provider here but one answers the OpenAI shape at its base URL,
/// which is why a provider has always been a URL and a key. ElevenLabs
/// does not, so the catalogue has to say which client to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Api {
    OpenAi,
    ElevenLabs,
}

pub struct Known {
    pub id: &'static str,
    pub label: &'static str,
    /// What it does for the house, in a sentence or two a newcomer can
    /// follow. Not what the API is.
    pub blurb: &'static str,
    /// What somebody needs before they start: an account, a key, a
    /// device on the network, a subscription.
    pub needs: &'static str,
    pub kind: Kind,
    pub group: Group,
    pub importance: Importance,
    /// A Lucide icon name. Generic on purpose: the card says what kind
    /// of thing it is, and the row beside it carries the brand.
    pub icon: &'static str,
    /// Every credential it keeps. Removing it deletes the stored ones,
    /// since nothing else shows them once it is gone.
    pub secrets: &'static [&'static str],
    /// Providers only. The one address its API lives at, so nobody has
    /// to look it up or mistype it.
    pub base_url: Option<&'static str>,
    pub serves: &'static [Role],
    /// Services have none and say `OpenAi`, which nothing reads.
    pub api: Api,
    /// What it can be asked for, per role. **The first entry for a role
    /// is what Niles uses when nothing is written down.**
    ///
    /// A list rather than a free-text box because nobody remembers
    /// `distil-whisper-large-v3-en`, and a box you have to guess at is
    /// a box that gets a wrong answer typed into it. It will go stale —
    /// providers add models — so the page keeps whatever is already
    /// configured even when it is not on this list, and offers a way to
    /// type one that is not.
    pub models: &'static [(Role, &'static str)],
}

/// The provider the shipped defaults point at.
///
/// `[stt]` and `[llm]` have to default to *something*, and this is what
/// they have always defaulted to.
pub const DEFAULT_PROVIDER: &str = "groq";

/// In the order the page shows them: by group, and within a group the
/// way a new install should work down it.
pub const KNOWN: &[Known] = &[
    Known {
        id: "mqtt",
        label: "Zigbee & MQTT",
        blurb: "Switch and dim your Zigbee lights and WLED strips.",
        needs: "An MQTT broker on your network, with Zigbee2MQTT connected to it. Every light reaches Niles through it.",
        kind: Kind::Service,
        group: Group::Essentials,
        importance: Importance::Required,
        icon: "network",
        secrets: &["mqtt.username", "mqtt.password"],
        base_url: None,
        serves: &[],
        api: Api::OpenAi,
        models: &[],
    },
    Known {
        id: "groq",
        label: "Groq",
        blurb: "Hears what you say and answers, in one account.",
        needs: "A Groq account (there is a free tier) and an API key from console.groq.com. It both listens and answers, fast enough to reply while you wait.",
        kind: Kind::Provider,
        group: Group::Essentials,
        importance: Importance::Required,
        icon: "sparkles",
        secrets: &["provider.groq.api_key"],
        base_url: Some("https://api.groq.com/openai/v1"),
        serves: &[Role::Stt, Role::Llm],
        api: Api::OpenAi,
        models: &[
            (Role::Stt, "whisper-large-v3-turbo"),
            (Role::Stt, "whisper-large-v3"),
            (Role::Stt, "distil-whisper-large-v3-en"),
            (Role::Llm, "openai/gpt-oss-20b"),
            (Role::Llm, "openai/gpt-oss-120b"),
            (Role::Llm, "llama-3.3-70b-versatile"),
            (Role::Llm, "llama-3.1-8b-instant"),
        ],
    },
    Known {
        id: "elevenlabs",
        label: "ElevenLabs",
        blurb: "Hears what you say, and is best at hearing Niles's name.",
        needs: "An ElevenLabs account and an API key. It only listens, so pair it with Groq or Cerebras to answer.",
        kind: Kind::Provider,
        group: Group::Essentials,
        importance: Importance::Required,
        icon: "audio-lines",
        secrets: &["provider.elevenlabs.api_key"],
        base_url: Some("https://api.elevenlabs.io/v1"),
        serves: &[Role::Stt],
        api: Api::ElevenLabs,
        models: &[(Role::Stt, "scribe_v2")],
    },
    Known {
        id: "cerebras",
        label: "Cerebras",
        blurb: "The fastest answers, said out loud while you wait.",
        needs: "A Cerebras account and an API key. It does not listen, so pair it with Groq or ElevenLabs.",
        kind: Kind::Provider,
        group: Group::Essentials,
        importance: Importance::Required,
        icon: "brain",
        secrets: &["provider.cerebras.api_key"],
        base_url: Some("https://api.cerebras.ai/v1"),
        // Language only. There is no speech endpoint here, and
        // offering one would fail as a 404 from somebody else's
        // server rather than as something this page could explain.
        serves: &[Role::Llm],
        api: Api::OpenAi,
        models: &[(Role::Llm, "gpt-oss-120b"), (Role::Llm, "qwen-3.8-27b")],
    },
    Known {
        id: "sonos",
        label: "Sonos",
        blurb: "Music and radio in each room, turned down while Niles speaks.",
        needs: "Sonos speakers on the same network as Niles.",
        kind: Kind::Service,
        group: Group::Music,
        importance: Importance::Optional,
        icon: "speaker",
        secrets: &[],
        base_url: None,
        serves: &[],
        api: Api::OpenAi,
        models: &[],
    },
    Known {
        id: "spotify",
        label: "Spotify",
        blurb: "Ask for any artist, song or album and hear it on Sonos.",
        needs: "Sonos, a Spotify Premium account linked in the Sonos app, and a free Spotify developer app for its two keys.",
        kind: Kind::Service,
        group: Group::Music,
        importance: Importance::Optional,
        icon: "music",
        secrets: &[
            "integrations.spotify.client_id",
            "integrations.spotify.client_secret",
        ],
        base_url: None,
        serves: &[],
        api: Api::OpenAi,
        models: &[],
    },
    Known {
        id: "lg_tv",
        label: "LG TV",
        blurb: "Turn it on and off, open apps and switch inputs by voice.",
        needs: "An LG TV on the same network. No LG account; you accept a prompt on the TV once.",
        kind: Kind::Service,
        group: Group::Music,
        importance: Importance::Optional,
        icon: "tv",
        // Issued by one TV when it is paired, so a TV added again pairs
        // again rather than keeping a key for the last one.
        secrets: &["tv.client_key"],
        base_url: None,
        serves: &[],
        api: Api::OpenAi,
        models: &[],
    },
    Known {
        id: "unifi",
        label: "UniFi",
        blurb: "Knows who is home from their phone, so lights can follow.",
        needs: "A UniFi network with its console on your network, and an API key made in UniFi Network.",
        kind: Kind::Service,
        group: Group::Presence,
        importance: Importance::Recommended,
        icon: "wifi",
        secrets: &["presence.unifi.api_key"],
        base_url: None,
        serves: &[],
        api: Api::OpenAi,
        models: &[],
    },
    Known {
        id: "tado",
        label: "tado°",
        blurb: "Set the heating in each room by voice.",
        needs: "tado° thermostats and the tado° account they are set up with.",
        kind: Kind::Service,
        group: Group::Presence,
        importance: Importance::Optional,
        icon: "thermometer",
        // tado keeps tokens rather than a key, and fetches them itself.
        secrets: &[],
        base_url: None,
        serves: &[],
        api: Api::OpenAi,
        models: &[],
    },
    Known {
        id: "microsoft",
        label: "Microsoft calendars",
        blurb: "Hear what is on your Outlook and work calendars.",
        needs: "A free app registration in Microsoft Entra, made once for the house. Then each person signs in from their profile.",
        kind: Kind::Service,
        group: Group::Calendars,
        importance: Importance::Optional,
        icon: "calendar",
        secrets: &[
            "integrations.microsoft.client_id",
            "integrations.microsoft.client_secret",
        ],
        base_url: None,
        serves: &[],
        api: Api::OpenAi,
        models: &[],
    },
    Known {
        id: "google",
        label: "Google calendars",
        blurb: "Hear what is on your Google calendar.",
        needs: "A free Google Cloud project with the Calendar API, made once for the house. Then each person signs in from their profile.",
        kind: Kind::Service,
        group: Group::Calendars,
        importance: Importance::Optional,
        icon: "calendar-days",
        secrets: &[
            "integrations.google.client_id",
            "integrations.google.client_secret",
        ],
        base_url: None,
        serves: &[],
        api: Api::OpenAi,
        models: &[],
    },
    Known {
        id: "nemlig",
        label: "nemlig.com",
        blurb: "Turn the shopping list into a filled basket.",
        needs: "A nemlig.com account, with its email and password.",
        kind: Kind::Service,
        group: Group::Shopping,
        importance: Importance::Optional,
        icon: "shopping-basket",
        secrets: &[
            "integrations.nemlig.username",
            "integrations.nemlig.password",
        ],
        base_url: None,
        serves: &[],
        api: Api::OpenAi,
        models: &[],
    },
    Known {
        id: "claude_code",
        label: "Claude Code",
        blurb: "Claude answers the chat and handles new rules.",
        needs: "A paid Claude subscription, and a token from running `claude setup-token` on a computer.",
        kind: Kind::Service,
        group: Group::Assistants,
        importance: Importance::Optional,
        icon: "bot",
        secrets: &["integrations.claude_code.oauth_token"],
        base_url: None,
        serves: &[],
        api: Api::OpenAi,
        models: &[],
    },
    Known {
        id: "chatgpt",
        label: "ChatGPT",
        blurb: "ChatGPT answers the chat and handles new rules.",
        needs: "A paid ChatGPT subscription (Plus or Pro). You sign in to ChatGPT from here; there is no key to copy.",
        kind: Kind::Service,
        group: Group::Assistants,
        importance: Importance::Optional,
        icon: "bot",
        // The sign-in, kept by Niles and renewed by it alone.
        secrets: &[
            "integrations.chatgpt.access_token",
            "integrations.chatgpt.refresh_token",
            "integrations.chatgpt.account_id",
            "integrations.chatgpt.expires_at",
        ],
        base_url: None,
        serves: &[],
        api: Api::OpenAi,
        models: &[],
    },
    Known {
        id: "linear",
        label: "Linear",
        blurb: "Turn an issue into work Niles picks up.",
        needs: "A Linear workspace, an API key, and the team whose issues Niles should watch.",
        kind: Kind::Service,
        group: Group::Assistants,
        importance: Importance::Optional,
        icon: "list-todo",
        secrets: &["integrations.linear.api_key"],
        base_url: None,
        serves: &[],
        api: Api::OpenAi,
        models: &[],
    },
];

pub fn find(id: &str) -> Option<&'static Known> {
    KNOWN.iter().find(|known| known.id == id)
}

impl Known {
    /// Everything it offers for a role, in the order it is offered.
    pub fn models_for(&self, role: Role) -> impl Iterator<Item = &'static str> {
        self.models
            .iter()
            .filter(move |(r, _)| *r == role)
            .map(|(_, model)| *model)
    }

    /// Where its key is kept.
    pub fn secret_key(&self) -> Option<String> {
        match self.kind {
            Kind::Provider => Some(format!("provider.{}.api_key", self.id)),
            // tado has tokens rather than a key, and fetches them itself.
            Kind::Service if self.id == "linear" => Some("integrations.linear.api_key".into()),
            Kind::Service if self.id == "claude_code" => {
                Some("integrations.claude_code.oauth_token".into())
            }
            // Not one typed in: its absence says nobody has signed in.
            Kind::Service if self.id == "chatgpt" => {
                Some("integrations.chatgpt.access_token".into())
            }
            // It takes two — the email as well — and its card asks for
            // both; this is the one whose absence says "not finished".
            Kind::Service if self.id == "nemlig" => Some("integrations.nemlig.password".into()),
            Kind::Service if self.id == "unifi" => Some("presence.unifi.api_key".into()),
            // Two again, like nemlig; the secret is the one that says.
            Kind::Service if self.id == "spotify" => {
                Some("integrations.spotify.client_secret".into())
            }
            Kind::Service if self.id == "microsoft" => {
                Some("integrations.microsoft.client_secret".into())
            }
            Kind::Service if self.id == "google" => {
                Some("integrations.google.client_secret".into())
            }
            Kind::Service => None,
        }
    }
}

/// The model the shipped config uses for a role.
///
/// Panics only if the catalogue is inconsistent with itself — the
/// default provider missing, or serving a role it lists no model for —
/// which the tests below rule out at build time rather than leaving to
/// a first start.
pub fn default_model(role: Role) -> &'static str {
    find(DEFAULT_PROVIDER)
        .and_then(|known| known.models_for(role).next())
        .expect("the default provider serves every role the shipped config has")
}

/// The endpoint the shipped config points a role at.
pub fn default_base_url() -> &'static str {
    find(DEFAULT_PROVIDER)
        .and_then(|known| known.base_url)
        .expect("the default provider is a provider, so it has an endpoint")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elevenlabs_is_the_one_that_does_not_speak_openai() {
        // The client Niles builds for speech-to-text is chosen from this;
        // a provider marked wrongly gets requests it cannot parse.
        for known in KNOWN.iter().filter(|k| k.kind == Kind::Provider) {
            let expected = if known.id == "elevenlabs" {
                Api::ElevenLabs
            } else {
                Api::OpenAi
            };
            assert_eq!(known.api, expected, "{}", known.id);
        }
        assert_eq!(
            find("elevenlabs").unwrap().models_for(Role::Stt).next(),
            Some("scribe_v2")
        );
    }

    #[test]
    fn the_default_provider_exists_and_is_one() {
        let known = find(DEFAULT_PROVIDER).expect("in the catalogue");
        assert_eq!(known.kind, Kind::Provider);
        assert!(known.base_url.is_some());
    }

    #[test]
    fn everything_a_provider_serves_it_offers_a_model_for() {
        // A provider that serves a role with no model would leave the
        // dropdown empty and the shipped default unresolvable.
        for known in KNOWN {
            for role in known.serves {
                assert!(
                    known.models_for(*role).next().is_some(),
                    "{} serves {role:?} and offers no model for it",
                    known.id
                );
            }
        }
    }

    #[test]
    fn a_service_offers_no_models_and_no_endpoint() {
        for known in KNOWN.iter().filter(|k| k.kind != Kind::Provider) {
            assert!(known.base_url.is_none(), "{}", known.id);
            assert!(known.models.is_empty(), "{}", known.id);
            assert!(known.serves.is_empty(), "{}", known.id);
        }
    }

    #[test]
    fn the_required_set_is_the_broker_and_the_voice_providers() {
        // Required means Niles does not do its job without it. The
        // broker is how it reaches any light at all; the providers are
        // "one of" for each role, which the group says rather than each
        // card. Anything else marked required would send a new install
        // off registering apps it does not need.
        let required: Vec<&str> = KNOWN
            .iter()
            .filter(|k| k.importance == Importance::Required)
            .map(|k| k.id)
            .collect();
        assert_eq!(required, ["mqtt", "groq", "elevenlabs", "cerebras"]);
        for known in KNOWN.iter().filter(|k| k.kind == Kind::Provider) {
            assert_eq!(known.importance, Importance::Required, "{}", known.id);
            assert_eq!(known.group, Group::Essentials, "{}", known.id);
        }
    }

    #[test]
    fn a_group_of_required_providers_says_one_of_each_is_enough() {
        let requirement = Group::Essentials.info().requirement.expect("said");
        assert!(requirement.contains("one speech-to-text"));
        assert!(requirement.contains("one AI provider"));
    }

    #[test]
    fn between_them_the_required_providers_serve_every_role() {
        for role in [Role::Stt, Role::Llm] {
            assert!(
                KNOWN.iter().any(|k| k.kind == Kind::Provider
                    && k.importance == Importance::Required
                    && k.serves.contains(&role)),
                "nothing required serves {role:?}"
            );
        }
    }

    #[test]
    fn every_entry_has_card_text_and_an_icon() {
        for known in KNOWN {
            assert!(!known.blurb.trim().is_empty(), "{}", known.id);
            assert!(!known.needs.trim().is_empty(), "{}", known.id);
            assert!(!known.icon.trim().is_empty(), "{}", known.id);
            assert!(!known.group.info().label.is_empty(), "{}", known.id);
        }
    }

    #[test]
    fn every_group_is_listed_once_and_used() {
        for (i, group) in GROUPS.iter().enumerate() {
            assert_eq!(group.id.position(), i, "{:?} listed twice", group.id);
            assert!(
                KNOWN.iter().any(|k| k.group == group.id),
                "{:?} has nothing in it",
                group.id
            );
        }
    }

    #[test]
    fn the_list_is_in_the_order_the_page_shows_it() {
        // The page heads its groups in the order they first appear, so
        // the catalogue's order is the page's order.
        let positions: Vec<usize> = KNOWN.iter().map(|k| k.group.position()).collect();
        assert!(positions.is_sorted(), "{positions:?}");
        assert_eq!(KNOWN[0].group, Group::Essentials);
    }

    #[test]
    fn the_key_that_says_finished_is_one_it_owns() {
        // Removing an integration deletes what it owns; a key it reads
        // but does not own would be left behind with nothing showing it.
        for known in KNOWN {
            if let Some(key) = known.secret_key() {
                assert!(known.secrets.contains(&key.as_str()), "{}", known.id);
            }
        }
    }

    #[test]
    fn what_each_integration_owns() {
        let owns = |id: &str| find(id).expect("in the catalogue").secrets.to_vec();
        assert_eq!(
            owns("nemlig"),
            [
                "integrations.nemlig.username",
                "integrations.nemlig.password"
            ]
        );
        for id in ["spotify", "microsoft", "google"] {
            assert_eq!(
                owns(id),
                [
                    format!("integrations.{id}.client_id"),
                    format!("integrations.{id}.client_secret")
                ]
            );
        }
        assert_eq!(owns("lg_tv"), ["tv.client_key"]);
        assert_eq!(owns("unifi"), ["presence.unifi.api_key"]);
        assert_eq!(owns("linear"), ["integrations.linear.api_key"]);
        assert_eq!(
            owns("claude_code"),
            ["integrations.claude_code.oauth_token"]
        );
        // All four of the sign-in, so removing it signs out for good.
        let mut chatgpt = owns("chatgpt");
        chatgpt.sort();
        let mut sign_in = crate::ChatGptSignIn::KEYS.to_vec();
        sign_in.sort();
        assert_eq!(chatgpt, sign_in);
        assert_eq!(owns("mqtt"), ["mqtt.username", "mqtt.password"]);
    }

    #[test]
    fn the_first_model_for_a_role_is_the_default() {
        assert_eq!(default_model(Role::Stt), "whisper-large-v3-turbo");
        assert_eq!(default_model(Role::Llm), "openai/gpt-oss-20b");
    }

    #[test]
    fn a_provider_is_only_offered_for_what_it_serves() {
        // What keeps Cerebras out of the speech dropdown. It has no
        // transcription endpoint, and the settings page filters on
        // exactly this: offering it there would fail as a 404 from
        // somebody else's server.
        let cerebras = find("cerebras").expect("in the catalogue");
        assert_eq!(cerebras.serves, &[Role::Llm]);
        assert!(cerebras.models_for(Role::Stt).next().is_none());
        assert_eq!(cerebras.models_for(Role::Llm).next(), Some("gpt-oss-120b"));
    }

    #[test]
    fn every_provider_has_somewhere_to_keep_a_key() {
        // The Integrations card shows a key field when there is a place
        // to put it, and the route that saves one accepts only keys
        // Niles reads. A provider without this is one the page offers
        // and nobody can finish setting up.
        for known in KNOWN.iter().filter(|k| k.kind == Kind::Provider) {
            let key = format!("provider.{}.api_key", known.id);
            assert_eq!(
                known.secret_key().as_deref(),
                Some(key.as_str()),
                "{}",
                known.id
            );
            assert_eq!(known.secrets, [key.as_str()], "{}", known.id);
        }
    }

    #[test]
    fn both_assistants_sit_together_before_linear() {
        let assistants: Vec<&str> = KNOWN
            .iter()
            .filter(|k| k.group == Group::Assistants)
            .map(|k| k.id)
            .collect();
        assert_eq!(assistants, ["claude_code", "chatgpt", "linear"]);
        let chatgpt = find("chatgpt").expect("in the catalogue");
        assert_eq!(chatgpt.kind, Kind::Service);
        assert_eq!(chatgpt.importance, Importance::Optional);
        assert!(chatgpt.needs.contains("ChatGPT subscription"));
    }

    #[test]
    fn no_two_entries_share_an_id() {
        let mut seen: Vec<&str> = Vec::new();
        for known in KNOWN {
            assert!(!seen.contains(&known.id), "two entries called {}", known.id);
            seen.push(known.id);
        }
    }
}
