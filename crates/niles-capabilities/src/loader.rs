//! Directory scanner and capability indexer.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use tracing::warn;

use crate::error::{Error, Result};
use crate::skill::{Capability, CapabilityMetadata};

/// In-memory index of capabilities loaded from a directory tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityLoader {
    by_name: BTreeMap<String, Capability>,
}

impl CapabilityLoader {
    /// Scan `root` one level deep for subdirectories containing `SKILL.md`.
    ///
    /// Each subdirectory is expected to contain a `SKILL.md` file with YAML
    /// frontmatter delimited by `---` and a markdown body.
    pub fn load_from_dir(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        let mut by_name: BTreeMap<String, Capability> = BTreeMap::new();

        // Sort entries so duplicate detection's `first`/`second` labels and
        // the order capabilities are encountered are reproducible across
        // filesystems (ext4's read_dir order is not lexicographic).
        let mut entries: Vec<_> = fs::read_dir(root)?.collect::<std::io::Result<_>>()?;
        entries.sort_by_key(|e| e.path());

        for entry in entries {
            let meta = entry.metadata()?;
            if !meta.is_dir() {
                continue;
            }

            let dir = entry.path();
            let skill_path = dir.join("SKILL.md");
            if !skill_path.is_file() {
                return Err(Error::MissingSkillFile { dir });
            }

            let raw = fs::read_to_string(&skill_path)?;
            let (metadata, body) = parse_skill_md(&raw, &dir)?;

            if let Some(existing) = by_name.get(&metadata.name) {
                return Err(Error::DuplicateName {
                    name: metadata.name.clone(),
                    first: existing.dir.clone(),
                    second: dir.clone(),
                });
            }

            let dir_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if dir_name != metadata.name {
                warn!(
                    dir = %dir.display(),
                    metadata_name = %metadata.name,
                    "capability directory name does not match metadata name"
                );
            }

            by_name.insert(
                metadata.name.clone(),
                Capability {
                    metadata,
                    body,
                    dir,
                },
            );
        }

        Ok(CapabilityLoader { by_name })
    }

    /// Return all registered capability names in alphabetical order.
    pub fn names(&self) -> Vec<&str> {
        self.by_name.keys().map(|s| s.as_str()).collect()
    }

    /// Look up a capability by its metadata name.
    pub fn get(&self, name: &str) -> Option<&Capability> {
        self.by_name.get(name)
    }

    /// Iterate capabilities in alphabetical order by name.
    pub fn iter(&self) -> impl Iterator<Item = &Capability> {
        self.by_name.values()
    }

    /// Number of loaded capabilities.
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    /// Whether the loader is empty.
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}

/// Split a `SKILL.md` string into frontmatter and body.
///
/// Expects the first line to be exactly `---`, a YAML block, a line that is
/// exactly `---`, then the markdown body. The closing delimiter must be on
/// its own line — a `---` substring inside a YAML scalar must not terminate
/// the frontmatter.
fn parse_skill_md(raw: &str, dir: &Path) -> Result<(CapabilityMetadata, String)> {
    let trimmed = raw.trim_start();

    // First line must be exactly `---` (allowing for `\r\n` line endings).
    let first_newline = trimmed.find('\n');
    let first_line = match first_newline {
        Some(idx) => trimmed[..idx].trim_end_matches('\r'),
        None => trimmed.trim_end_matches('\r'),
    };
    if first_line != "---" {
        return Err(Error::Frontmatter {
            dir: dir.to_path_buf(),
            reason: "first line must be `---` delimiter".into(),
        });
    }

    let Some(open_end) = first_newline else {
        return Err(Error::Frontmatter {
            dir: dir.to_path_buf(),
            reason: "missing closing `---` delimiter".into(),
        });
    };
    let after_open = &trimmed[open_end + 1..];

    // Walk lines until we hit one that is exactly `---`.
    let mut search = 0;
    let close = loop {
        let line_end = after_open[search..]
            .find('\n')
            .map(|i| search + i)
            .unwrap_or(after_open.len());
        let line = after_open[search..line_end].trim_end_matches('\r');
        if line == "---" {
            break Some((search, line_end));
        }
        if line_end >= after_open.len() {
            break None;
        }
        search = line_end + 1;
    };

    let Some((close_start, close_end)) = close else {
        return Err(Error::Frontmatter {
            dir: dir.to_path_buf(),
            reason: "missing closing `---` delimiter".into(),
        });
    };

    let yaml = &after_open[..close_start];
    let body_start = (close_end + 1).min(after_open.len());
    let body = after_open[body_start..].trim_start().to_string();

    if body.is_empty() {
        return Err(Error::BodyMissing {
            dir: dir.to_path_buf(),
        });
    }

    let metadata: CapabilityMetadata =
        serde_yaml::from_str(yaml).map_err(|e| Error::Frontmatter {
            dir: dir.to_path_buf(),
            reason: e.to_string(),
        })?;

    Ok((metadata, body))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn write_skill(dir: &std::path::Path, content: &str) {
        fs::write(dir.join("SKILL.md"), content).unwrap();
    }

    #[test]
    fn empty_directory_is_empty() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        assert!(loader.is_empty());
        assert_eq!(loader.len(), 0);
        assert!(loader.names().is_empty());
    }

    #[test]
    fn one_valid_capability_loads_correctly() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("lights");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\nname: lights\ndescription: Control smart lights\nversion: 1.0.0\n---\n# Lights\n\nTurn on/off lights.\n",
        );

        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        assert_eq!(loader.len(), 1);
        let cap = loader.get("lights").unwrap();
        assert_eq!(cap.metadata.name, "lights");
        assert_eq!(cap.metadata.description, "Control smart lights");
        assert_eq!(cap.metadata.version, "1.0.0");
        assert!(cap.metadata.prerequisites.is_empty());
        assert_eq!(cap.body, "# Lights\n\nTurn on/off lights.\n");
        assert_eq!(cap.dir, cap_dir);
    }

    #[test]
    fn missing_body_errors() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("empty-body");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\nname: empty-body\ndescription: No body\nversion: 0.1.0\n---",
        );

        let err = CapabilityLoader::load_from_dir(tmp.path()).unwrap_err();
        assert!(matches!(err, Error::BodyMissing { .. }));
    }

    #[test]
    fn no_frontmatter_errors() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("no-fm");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(&cap_dir, "# Just markdown\n\nNo frontmatter here.\n");

        let err = CapabilityLoader::load_from_dir(tmp.path()).unwrap_err();
        assert!(matches!(err, Error::Frontmatter { .. }));
    }

    #[test]
    fn missing_required_field_in_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("bad-fm");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\nname: bad-fm\nversion: 0.1.0\n---\nBody here.\n",
        );

        let err = CapabilityLoader::load_from_dir(tmp.path()).unwrap_err();
        match err {
            Error::Frontmatter { reason, .. } => {
                assert!(
                    reason.contains("description"),
                    "reason should mention missing field: {reason}"
                );
            }
            other => panic!("expected Frontmatter error, got {other:?}"),
        }
    }

    #[test]
    fn two_capabilities_sorted_alphabetically() {
        let tmp = TempDir::new().unwrap();
        let zebra_dir = tmp.path().join("zebra");
        fs::create_dir(&zebra_dir).unwrap();
        write_skill(
            &zebra_dir,
            "---\nname: zebra\ndescription: Zebra capability\nversion: 1.0.0\n---\nZ.\n",
        );

        let alpha_dir = tmp.path().join("alpha");
        fs::create_dir(&alpha_dir).unwrap();
        write_skill(
            &alpha_dir,
            "---\nname: alpha\ndescription: Alpha capability\nversion: 1.0.0\n---\nA.\n",
        );

        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        assert_eq!(loader.len(), 2);
        let names = loader.names();
        assert_eq!(names, vec!["alpha", "zebra"]);

        let collected: Vec<_> = loader.iter().map(|c| &c.metadata.name).collect();
        assert_eq!(collected, vec!["alpha", "zebra"]);
    }

    #[test]
    fn duplicate_name_errors() {
        let tmp = TempDir::new().unwrap();
        let a_dir = tmp.path().join("cap-a");
        fs::create_dir(&a_dir).unwrap();
        write_skill(
            &a_dir,
            "---\nname: shared\ndescription: First\nversion: 1.0.0\n---\nA.\n",
        );

        let b_dir = tmp.path().join("cap-b");
        fs::create_dir(&b_dir).unwrap();
        write_skill(
            &b_dir,
            "---\nname: shared\ndescription: Second\nversion: 2.0.0\n---\nB.\n",
        );

        let err = CapabilityLoader::load_from_dir(tmp.path()).unwrap_err();
        match err {
            Error::DuplicateName {
                name,
                first,
                second,
            } => {
                assert_eq!(name, "shared");
                assert_eq!(first, a_dir);
                assert_eq!(second, b_dir);
            }
            other => panic!("expected DuplicateName error, got {other:?}"),
        }
    }

    #[test]
    fn capability_with_nested_subdir_loads_fine() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("my-cap");
        fs::create_dir(&cap_dir).unwrap();
        fs::create_dir(cap_dir.join("references")).unwrap();
        fs::write(cap_dir.join("references").join("foo.md"), "# Ref\n").unwrap();
        write_skill(
            &cap_dir,
            "---\nname: my-cap\ndescription: With refs\nversion: 1.0.0\n---\nSee [refs](references/foo.md).\n",
        );

        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        assert_eq!(loader.len(), 1);
        assert!(loader.get("my-cap").is_some());
    }

    #[test]
    fn dir_name_vs_metadata_name_mismatch_loads_with_warning() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("dir-name");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\nname: meta-name\ndescription: Mismatch\nversion: 1.0.0\n---\nBody.\n",
        );

        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        assert_eq!(loader.len(), 1);
        let cap = loader.get("meta-name").unwrap();
        assert_eq!(cap.metadata.name, "meta-name");
        assert_eq!(cap.dir.file_name().unwrap(), "dir-name");
    }

    #[test]
    fn prerequisites_parse_into_vec() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("prereq");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\nname: prereq\ndescription: Needs deps\nversion: 1.0.0\nprerequisites:\n  - foo\n  - bar\n---\nBody.\n",
        );

        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let cap = loader.get("prereq").unwrap();
        assert_eq!(cap.metadata.prerequisites, vec!["foo", "bar"]);
    }

    #[test]
    fn yaml_scalar_containing_triple_dash_is_not_a_delimiter() {
        // The closing `---` delimiter must be on its own line; a `---`
        // substring inside a YAML scalar must not terminate frontmatter.
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("triple");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\nname: triple\ndescription: \"a --- b\"\nversion: 1.0.0\n---\nBody.\n",
        );

        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let cap = loader.get("triple").unwrap();
        assert_eq!(cap.metadata.description, "a --- b");
        assert_eq!(cap.body, "Body.\n");
    }

    #[test]
    fn subdir_without_skill_md_errors() {
        // Loader enforces the strict policy that every subdirectory under
        // `root` is a capability and must contain SKILL.md. A stray
        // directory (without SKILL.md) surfaces as a `MissingSkillFile`
        // error rather than being silently skipped.
        let tmp = TempDir::new().unwrap();
        let stray = tmp.path().join("not-a-capability");
        fs::create_dir(&stray).unwrap();

        let err = CapabilityLoader::load_from_dir(tmp.path()).unwrap_err();
        match err {
            Error::MissingSkillFile { dir } => assert_eq!(dir, stray),
            other => panic!("expected MissingSkillFile, got {other:?}"),
        }
    }

    #[test]
    fn crlf_line_endings_parse_correctly() {
        // SKILL.md files authored on Windows can have `\r\n` line endings.
        // The parser must still detect `---` delimiters and not include the
        // trailing `\r` in YAML keys.
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("crlf");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\r\nname: crlf\r\ndescription: Windows line endings\r\nversion: 1.0.0\r\n---\r\nBody line one.\r\n",
        );

        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let cap = loader.get("crlf").unwrap();
        assert_eq!(cap.metadata.description, "Windows line endings");
    }

    #[test]
    fn get_returns_none_for_unknown() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        assert!(loader.get("nonexistent").is_none());
    }
}
