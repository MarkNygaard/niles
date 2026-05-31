use anyhow::Context;
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct Catalog {
    #[serde(default)]
    pub preamble: Preamble,
    #[serde(default, rename = "feature")]
    pub features: Vec<Feature>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Preamble {
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Feature {
    pub id: String,
    pub category: String,
    pub summary: String,
    #[serde(default)]
    pub phrasings: Vec<String>,
    pub since_pr: u32,
    #[serde(default)]
    pub hardware_required: bool,
}

impl Catalog {
    pub fn from_toml(raw: &str) -> anyhow::Result<Self> {
        toml::from_str(raw).context("parsing features.toml")
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        let mut errors: Vec<String> = Vec::new();
        let mut seen_ids: HashSet<&str> = HashSet::new();

        for (idx, feature) in self.features.iter().enumerate() {
            let label = if feature.id.is_empty() || !is_valid_id(&feature.id) {
                format!("feature[{idx}]")
            } else {
                format!("`{}`", feature.id)
            };

            if feature.id.is_empty() {
                errors.push(format!("{label}: id is empty"));
            } else if !is_valid_id(&feature.id) {
                errors.push(format!(
                    "{label}: id `{}` is invalid (must match ^[a-z0-9][a-z0-9-]{{0,63}}$)",
                    feature.id
                ));
            } else if !seen_ids.insert(&feature.id) {
                errors.push(format!("{label}: duplicate id `{}`", feature.id));
            }

            if feature.category.is_empty() {
                errors.push(format!("{label}: category is empty"));
            } else if !is_valid_category(&feature.category) {
                errors.push(format!(
                    "{label}: category `{}` is invalid (must match ^[a-z]+(\\.[a-z][a-z-]*)*$)",
                    feature.category
                ));
            }

            if feature.summary.is_empty() {
                errors.push(format!("{label}: summary is empty"));
            } else {
                let summary_len = feature.summary.chars().count();
                if summary_len > 120 {
                    errors.push(format!("{label}: summary is {summary_len} chars (max 120)"));
                }
                if feature.summary.contains('\n') || feature.summary.contains('\r') {
                    errors.push(format!("{label}: summary contains newline"));
                }
                if feature.summary.contains('|') {
                    errors.push(format!("{label}: summary contains '|'"));
                }
            }

            for (pidx, phrasing) in feature.phrasings.iter().enumerate() {
                if phrasing.is_empty() {
                    errors.push(format!("{label}: phrasings[{pidx}] is empty"));
                }
            }

            if feature.since_pr < 1 {
                errors.push(format!("{label}: since_pr must be >= 1"));
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            let n = errors.len();
            let joined = errors.join("\n  - ");
            Err(anyhow::anyhow!(
                "features.toml has {n} validation error(s):\n  - {joined}"
            ))
        }
    }
}

/// Hand-rolled validation: `^[a-z0-9][a-z0-9-]{0,63}$`
fn is_valid_id(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return false;
    }
    let first = bytes[0];
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }
    for &b in &bytes[1..] {
        if b != b'-' && !b.is_ascii_lowercase() && !b.is_ascii_digit() {
            return false;
        }
    }
    true
}

/// Hand-rolled validation: `^[a-z]+(\.[a-z-]+)*$`
fn is_valid_category(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return false;
    }

    let mut i = 0;
    // first segment: [a-z]+
    if i >= bytes.len() || !bytes[i].is_ascii_lowercase() {
        return false;
    }
    i += 1;
    while i < bytes.len() && bytes[i].is_ascii_lowercase() {
        i += 1;
    }

    // remaining segments: (\.[a-z-]+)*
    while i < bytes.len() {
        if bytes[i] != b'.' {
            return false;
        }
        i += 1;
        if i >= bytes.len() || !bytes[i].is_ascii_lowercase() {
            return false;
        }
        i += 1;
        while i < bytes.len() && (bytes[i].is_ascii_lowercase() || bytes[i] == b'-') {
            i += 1;
        }
    }

    true
}

pub fn render(catalog: &Catalog) -> String {
    let mut out = String::new();

    out.push_str("<!-- DO NOT EDIT BY HAND. Generated from features.toml — run `cargo run -p niles-bin -- generate-manifest`. -->\n\n");
    out.push_str("# niles — feature manifest\n\n");

    if !catalog.preamble.text.trim().is_empty() {
        out.push_str(catalog.preamble.text.trim());
        out.push_str("\n\n");
    }

    // Group by top-level category (BTreeMap keeps keys sorted)
    let mut map: BTreeMap<String, Vec<&Feature>> = BTreeMap::new();
    for f in &catalog.features {
        let top = f
            .category
            .split('.')
            .next()
            .unwrap_or(&f.category)
            .to_string();
        map.entry(top).or_default().push(f);
    }
    let mut groups: Vec<(String, Vec<&Feature>)> = map.into_iter().collect();
    for (_, features) in &mut groups {
        features.sort_by_key(|f| (&f.category, &f.id));
    }

    for (top_level, features) in &groups {
        out.push_str(&format!("## {top_level}\n\n"));

        let has_hardware = features.iter().any(|f| f.hardware_required);

        if has_hardware {
            out.push_str("| Feature | Summary | Since | Hardware |\n");
            out.push_str("| --- | --- | --- | --- |\n");
        } else {
            out.push_str("| Feature | Summary | Since |\n");
            out.push_str("| --- | --- | --- |\n");
        }

        for f in features.iter() {
            let since = format!(
                "[#{}](https://github.com/MarkNygaard/niles/pull/{})",
                f.since_pr, f.since_pr
            );
            if has_hardware {
                let hw = if f.hardware_required {
                    "⚠️ hardware"
                } else {
                    "—"
                };
                out.push_str(&format!(
                    "| `{id}` | {summary} | {since} | {hw} |\n",
                    id = f.id,
                    summary = f.summary,
                ));
            } else {
                out.push_str(&format!(
                    "| `{id}` | {summary} | {since} |\n",
                    id = f.id,
                    summary = f.summary,
                ));
            }
        }

        out.push('\n');

        for f in features.iter() {
            if !f.phrasings.is_empty() {
                out.push_str(&format!("### `{id}` — example phrasings\n\n", id = f.id));
                for phrasing in &f.phrasings {
                    out.push_str(&format!("- {phrasing}\n"));
                }
                out.push('\n');
            }
        }
    }

    // Ensure exactly one trailing newline
    while out.ends_with('\n') {
        out.pop();
    }
    out.push('\n');

    out
}

#[derive(clap::Args)]
pub struct GenerateManifestArgs {
    /// Path to features.toml.
    #[arg(long, default_value = "features.toml")]
    pub catalog: PathBuf,
    /// Path to write the generated MANIFEST.md.
    #[arg(long, default_value = "MANIFEST.md")]
    pub output: PathBuf,
    /// Check that MANIFEST.md is in sync with features.toml instead of writing.
    #[arg(long)]
    pub check: bool,
}

pub fn generate_manifest(args: GenerateManifestArgs) -> anyhow::Result<()> {
    let catalog_raw = std::fs::read_to_string(&args.catalog)
        .with_context(|| format!("reading {}", args.catalog.display()))?;
    let catalog = Catalog::from_toml(&catalog_raw)?;
    catalog.validate()?;
    let rendered = render(&catalog);

    if args.check {
        let existing = std::fs::read_to_string(&args.output)
            .with_context(|| format!("reading {}", args.output.display()))?;
        if existing != rendered {
            eprintln!("MANIFEST.md is out of date with features.toml.");
            eprintln!("Run `cargo run -p niles-bin -- generate-manifest` to update.");
            std::process::exit(1);
        }
        println!("MANIFEST.md is up to date.");
    } else {
        std::fs::write(&args.output, &rendered)
            .with_context(|| format!("writing {}", args.output.display()))?;
        println!(
            "Wrote {} ({} bytes).",
            args.output.display(),
            rendered.len()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parser tests ───────────────────────────────────────────────

    #[test]
    fn parse_minimal_catalog() {
        let raw = r#"
[[feature]]
id = "test-feature"
category = "voice.lighting"
summary = "Turn a light on or off"
since_pr = 3
"#;
        let catalog = Catalog::from_toml(raw).unwrap();
        assert_eq!(catalog.features.len(), 1);
        assert_eq!(catalog.features[0].id, "test-feature");
        assert!(catalog.features[0].phrasings.is_empty());
        assert!(!catalog.features[0].hardware_required);
    }

    #[test]
    fn parse_rejects_unknown_field() {
        let raw = r#"
[[feature]]
id = "test"
category = "voice.lighting"
summary = "x"
since_pr = 1
foo = "bar"
"#;
        assert!(Catalog::from_toml(raw).is_err());
    }

    #[test]
    fn parse_empty_phrasings_defaulted() {
        let raw = r#"
[[feature]]
id = "test"
category = "voice.lighting"
summary = "x"
since_pr = 1
"#;
        let catalog = Catalog::from_toml(raw).unwrap();
        assert!(catalog.features[0].phrasings.is_empty());
    }

    // ── validation tests ───────────────────────────────────────────

    #[test]
    fn validate_rejects_empty_id() {
        let catalog = Catalog {
            preamble: Preamble::default(),
            features: vec![Feature {
                id: "".into(),
                category: "voice.lighting".into(),
                summary: "x".into(),
                phrasings: vec![],
                since_pr: 1,
                hardware_required: false,
            }],
        };
        let err = catalog.validate().unwrap_err().to_string();
        assert!(err.contains("id is empty"));
    }

    #[test]
    fn validate_rejects_bad_id_chars() {
        for bad in ["Bad", "bad_id", "-bad", "a".repeat(65).as_str()] {
            let catalog = Catalog {
                preamble: Preamble::default(),
                features: vec![Feature {
                    id: bad.into(),
                    category: "voice.lighting".into(),
                    summary: "x".into(),
                    phrasings: vec![],
                    since_pr: 1,
                    hardware_required: false,
                }],
            };
            assert!(catalog.validate().is_err(), "expected error for id={bad:?}");
        }
    }

    #[test]
    fn validate_rejects_duplicate_id() {
        let catalog = Catalog {
            preamble: Preamble::default(),
            features: vec![
                Feature {
                    id: "dup".into(),
                    category: "a.b".into(),
                    summary: "x".into(),
                    phrasings: vec![],
                    since_pr: 1,
                    hardware_required: false,
                },
                Feature {
                    id: "dup".into(),
                    category: "c.d".into(),
                    summary: "y".into(),
                    phrasings: vec![],
                    since_pr: 2,
                    hardware_required: false,
                },
            ],
        };
        let err = catalog.validate().unwrap_err().to_string();
        assert!(err.contains("duplicate id"));
    }

    #[test]
    fn validate_rejects_malformed_category() {
        for bad in ["Voice.Lighting", "voice..lighting", ".voice", "voice."] {
            let catalog = Catalog {
                preamble: Preamble::default(),
                features: vec![Feature {
                    id: "test".into(),
                    category: bad.into(),
                    summary: "x".into(),
                    phrasings: vec![],
                    since_pr: 1,
                    hardware_required: false,
                }],
            };
            assert!(
                catalog.validate().is_err(),
                "expected error for category={bad:?}"
            );
        }
    }

    #[test]
    fn validate_rejects_long_summary() {
        let catalog = Catalog {
            preamble: Preamble::default(),
            features: vec![Feature {
                id: "test".into(),
                category: "a.b".into(),
                summary: "x".repeat(121),
                phrasings: vec![],
                since_pr: 1,
                hardware_required: false,
            }],
        };
        let err = catalog.validate().unwrap_err().to_string();
        assert!(err.contains("summary is 121 chars"));
    }

    #[test]
    fn validate_rejects_empty_summary() {
        let catalog = Catalog {
            preamble: Preamble::default(),
            features: vec![Feature {
                id: "test".into(),
                category: "a.b".into(),
                summary: "".into(),
                phrasings: vec![],
                since_pr: 1,
                hardware_required: false,
            }],
        };
        let err = catalog.validate().unwrap_err().to_string();
        assert!(err.contains("summary is empty"));
    }

    #[test]
    fn validate_rejects_pipe_in_summary() {
        let catalog = Catalog {
            preamble: Preamble::default(),
            features: vec![Feature {
                id: "test".into(),
                category: "a.b".into(),
                summary: "has | pipe".into(),
                phrasings: vec![],
                since_pr: 1,
                hardware_required: false,
            }],
        };
        let err = catalog.validate().unwrap_err().to_string();
        assert!(err.contains("contains '|'"));
    }

    #[test]
    fn validate_rejects_empty_phrasing() {
        let catalog = Catalog {
            preamble: Preamble::default(),
            features: vec![Feature {
                id: "test".into(),
                category: "a.b".into(),
                summary: "x".into(),
                phrasings: vec!["".into()],
                since_pr: 1,
                hardware_required: false,
            }],
        };
        let err = catalog.validate().unwrap_err().to_string();
        assert!(err.contains("phrasings[0] is empty"));
    }

    #[test]
    fn validate_rejects_since_pr_zero() {
        let catalog = Catalog {
            preamble: Preamble::default(),
            features: vec![Feature {
                id: "test".into(),
                category: "a.b".into(),
                summary: "x".into(),
                phrasings: vec![],
                since_pr: 0,
                hardware_required: false,
            }],
        };
        let err = catalog.validate().unwrap_err().to_string();
        assert!(err.contains("since_pr must be >= 1"));
    }

    #[test]
    fn validate_reports_all_errors() {
        let catalog = Catalog {
            preamble: Preamble::default(),
            features: vec![
                Feature {
                    id: "".into(),
                    category: "a.b".into(),
                    summary: "x".into(),
                    phrasings: vec![],
                    since_pr: 0,
                    hardware_required: false,
                },
                Feature {
                    id: "dup".into(),
                    category: "BAD".into(),
                    summary: "y".into(),
                    phrasings: vec![],
                    since_pr: 1,
                    hardware_required: false,
                },
            ],
        };
        let err = catalog.validate().unwrap_err().to_string();
        assert!(err.contains("id is empty"));
        assert!(err.contains("since_pr must be >= 1"));
        assert!(err.contains("category"));
    }

    #[test]
    fn validate_exactly_120_char_summary_passes() {
        let catalog = Catalog {
            preamble: Preamble::default(),
            features: vec![Feature {
                id: "test".into(),
                category: "a.b".into(),
                summary: "x".repeat(120),
                phrasings: vec![],
                since_pr: 1,
                hardware_required: false,
            }],
        };
        catalog.validate().unwrap();
    }

    #[test]
    fn validate_64_char_id_passes() {
        let catalog = Catalog {
            preamble: Preamble::default(),
            features: vec![Feature {
                id: "a".repeat(64),
                category: "a.b".into(),
                summary: "x".into(),
                phrasings: vec![],
                since_pr: 1,
                hardware_required: false,
            }],
        };
        catalog.validate().unwrap();
    }

    #[test]
    fn validate_valid_multi_segment_category_passes() {
        for good in [
            "a",
            "a.b",
            "a.b.c",
            "voice.lighting",
            "api.read",
            "ambient.manual-mode",
        ] {
            let catalog = Catalog {
                preamble: Preamble::default(),
                features: vec![Feature {
                    id: "test".into(),
                    category: good.into(),
                    summary: "x".into(),
                    phrasings: vec![],
                    since_pr: 1,
                    hardware_required: false,
                }],
            };
            catalog.validate().unwrap();
        }
    }

    #[test]
    fn validate_rejects_newline_in_summary() {
        for bad in ["has \n newline", "has \r carriage return", "has \r\n crlf"] {
            let catalog = Catalog {
                preamble: Preamble::default(),
                features: vec![Feature {
                    id: "test".into(),
                    category: "a.b".into(),
                    summary: bad.into(),
                    phrasings: vec![],
                    since_pr: 1,
                    hardware_required: false,
                }],
            };
            let err = catalog.validate().unwrap_err().to_string();
            assert!(
                err.contains("summary contains newline"),
                "expected newline error for {bad:?}"
            );
        }
    }

    // ── renderer tests ─────────────────────────────────────────────

    #[test]
    fn render_groups_by_top_level_category() {
        let catalog = Catalog {
            preamble: Preamble::default(),
            features: vec![
                Feature {
                    id: "light-on".into(),
                    category: "voice.lighting".into(),
                    summary: "Turn on".into(),
                    phrasings: vec![],
                    since_pr: 3,
                    hardware_required: false,
                },
                Feature {
                    id: "weather".into(),
                    category: "llm.tools".into(),
                    summary: "Get weather".into(),
                    phrasings: vec![],
                    since_pr: 81,
                    hardware_required: false,
                },
            ],
        };
        let out = render(&catalog);
        let voice_pos = out.find("## voice").unwrap();
        let llm_pos = out.find("## llm").unwrap();
        assert!(llm_pos < voice_pos);
    }

    #[test]
    fn render_sorts_within_group() {
        let catalog = Catalog {
            preamble: Preamble::default(),
            features: vec![
                Feature {
                    id: "b".into(),
                    category: "voice.media".into(),
                    summary: "B".into(),
                    phrasings: vec![],
                    since_pr: 1,
                    hardware_required: false,
                },
                Feature {
                    id: "a".into(),
                    category: "voice.lighting".into(),
                    summary: "A".into(),
                    phrasings: vec![],
                    since_pr: 1,
                    hardware_required: false,
                },
            ],
        };
        let out = render(&catalog);
        let a_pos = out.find("`a`").unwrap();
        let b_pos = out.find("`b`").unwrap();
        assert!(a_pos < b_pos);
    }

    #[test]
    fn render_hardware_column_only_when_needed() {
        let no_hw = Catalog {
            preamble: Preamble::default(),
            features: vec![Feature {
                id: "x".into(),
                category: "a.b".into(),
                summary: "S".into(),
                phrasings: vec![],
                since_pr: 1,
                hardware_required: false,
            }],
        };
        let out_no = render(&no_hw);
        assert!(!out_no.contains("Hardware"));
        assert!(!out_no.contains("⚠️ hardware"));

        let with_hw = Catalog {
            preamble: Preamble::default(),
            features: vec![
                Feature {
                    id: "x".into(),
                    category: "a.b".into(),
                    summary: "S".into(),
                    phrasings: vec![],
                    since_pr: 1,
                    hardware_required: false,
                },
                Feature {
                    id: "y".into(),
                    category: "a.c".into(),
                    summary: "T".into(),
                    phrasings: vec![],
                    since_pr: 2,
                    hardware_required: true,
                },
            ],
        };
        let out_yes = render(&with_hw);
        assert!(out_yes.contains("Hardware"));
        assert!(out_yes.contains("⚠️ hardware"));
        assert!(out_yes.contains("—"));
    }

    #[test]
    fn render_pull_request_link_format() {
        let catalog = Catalog {
            preamble: Preamble::default(),
            features: vec![Feature {
                id: "x".into(),
                category: "a.b".into(),
                summary: "S".into(),
                phrasings: vec![],
                since_pr: 42,
                hardware_required: false,
            }],
        };
        let out = render(&catalog);
        assert!(out.contains("[#42](https://github.com/MarkNygaard/niles/pull/42)"));
    }

    #[test]
    fn render_phrasings_block_skipped_when_empty() {
        let catalog = Catalog {
            preamble: Preamble::default(),
            features: vec![Feature {
                id: "x".into(),
                category: "a.b".into(),
                summary: "S".into(),
                phrasings: vec![],
                since_pr: 1,
                hardware_required: false,
            }],
        };
        let out = render(&catalog);
        assert!(!out.contains("example phrasings"));
    }

    #[test]
    fn render_emits_do_not_edit_banner() {
        let catalog = Catalog {
            preamble: Preamble::default(),
            features: vec![],
        };
        let out = render(&catalog);
        assert!(out.starts_with("<!-- DO NOT EDIT BY HAND"));
    }

    #[test]
    fn render_preamble_emitted_when_present() {
        let catalog = Catalog {
            preamble: Preamble {
                text: "Hello world.".into(),
            },
            features: vec![],
        };
        let out = render(&catalog);
        assert!(out.contains("Hello world."));
    }

    #[test]
    fn render_preamble_skipped_when_empty() {
        let catalog = Catalog {
            preamble: Preamble { text: "".into() },
            features: vec![],
        };
        let out = render(&catalog);
        // Should go straight from manifest title to end (no "##" sections because no features)
        let title_end = out.find("# niles — feature manifest\n").unwrap()
            + "# niles — feature manifest\n".len();
        let rest = &out[title_end..];
        // rest should be empty (exactly one trailing newline is part of the title match)
        assert_eq!(rest, "");
    }

    #[test]
    fn render_ends_with_exactly_one_newline() {
        let catalog = Catalog {
            preamble: Preamble::default(),
            features: vec![Feature {
                id: "x".into(),
                category: "a.b".into(),
                summary: "S".into(),
                phrasings: vec!["phrase".into()],
                since_pr: 1,
                hardware_required: false,
            }],
        };
        let out = render(&catalog);
        assert!(out.ends_with('\n'));
        assert!(!out.ends_with("\n\n"));
    }

    #[test]
    fn render_three_level_category_groups_by_top_level() {
        let catalog = Catalog {
            preamble: Preamble::default(),
            features: vec![Feature {
                id: "x".into(),
                category: "voice.lighting.warm-white".into(),
                summary: "S".into(),
                phrasings: vec![],
                since_pr: 1,
                hardware_required: false,
            }],
        };
        let out = render(&catalog);
        assert!(out.contains("## voice"));
        assert!(!out.contains("## voice.lighting"));
    }

    // ── integration test ───────────────────────────────────────────

    #[test]
    fn committed_manifest_matches_features_toml() {
        let catalog_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../features.toml");
        let manifest_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../MANIFEST.md");

        // Only run when both files exist (they may not during very early bootstrap).
        if !catalog_path.exists() || !manifest_path.exists() {
            return;
        }

        let raw = std::fs::read_to_string(&catalog_path).unwrap();
        let catalog = Catalog::from_toml(&raw).unwrap();
        catalog.validate().unwrap();
        let rendered = render(&catalog);
        let committed = std::fs::read_to_string(&manifest_path).unwrap();
        assert_eq!(
            rendered,
            committed,
            "rendered MANIFEST.md does not match committed {}. \
             Run `cargo run -p niles-bin -- generate-manifest` to update.",
            manifest_path.display()
        );
    }
}
