//! Pictures for artifacts: found with the house's own search, kept by
//! Niles as its own copies. Why copies is in `niles_artifacts::images`.
//!
//! The assistant finds a picture with `image_search` (SearXNG's image
//! engines), and keeps it with `artifact_add_image`: Niles fetches it
//! from the public internet only ([`niles_websearch::PublicFetcher`]),
//! writes it again ([`images::prepare`]) and keeps it with the artifact,
//! which answers its name, `niles-image:12`. Content and pages name it
//! by that, never by its address, so nothing the person looks at
//! reaches out to anyone.

use super::{Artifacts, asker, id_arg, invalid, refused, str_arg};
use async_trait::async_trait;
use niles_artifacts::images::{self, NewImage};
use niles_artifacts::{Image, Operation, kinds};
use niles_tools::{Result, Tool, ToolDescriptor, ToolRegistry};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

/// What a picture may be asked for as: the four kinds Niles keeps.
const ACCEPT: &str = "image/jpeg,image/png,image/webp,image/gif;q=0.9,*/*;q=0.1";

/// From the first request to the last byte, redirects and all.
pub(crate) const DOWNLOAD_WITHIN: Duration = Duration::from_secs(30);

/// How many results `image_search` gives unless asked for more: enough
/// to choose from, few enough to read.
const RESULTS: u8 = 6;

/// Gets the bytes at an address the assistant chose. Niles's fetcher,
/// which goes only to the public internet; a test's own pictures.
#[async_trait]
pub(crate) trait Download: Send + Sync {
    async fn download(&self, url: &str) -> std::result::Result<Vec<u8>, String>;
}

#[async_trait]
impl Download for niles_websearch::PublicFetcher {
    async fn download(&self, url: &str) -> std::result::Result<Vec<u8>, String> {
        self.fetch(url, ACCEPT)
            .await
            .map(|f| f.bytes)
            .map_err(|e| e.to_string())
    }
}

/// Where pictures are found and how they are fetched.
pub(crate) struct Pictures {
    /// The house's search: `image_search` is offered while it has a
    /// SearXNG, and only then. Without one a picture can still be kept
    /// from an address the person gave or a page turned up.
    pub(crate) search: niles_tools::Search,
    pub(crate) download: Arc<dyn Download>,
}

/// The picture tools, for `only` artifact in a job, or in a turn any
/// the asker may change: theirs, or shared with them to edit. Nothing
/// without [`Pictures`].
pub(super) fn register(artifacts: &Arc<Artifacts>, reg: &mut ToolRegistry, only: Option<i64>) {
    let Some(pictures) = &artifacts.pictures else {
        return;
    };
    reg.register(Box::new(ImageSearch(pictures.search.clone())));
    reg.register(Box::new(AddImage {
        artifacts: artifacts.clone(),
        only,
    }));
    reg.register(Box::new(RemoveImage {
        artifacts: artifacts.clone(),
        only,
    }));
}

/// A kept picture as the assistant is told of it.
pub(super) fn view(image: &Image) -> Value {
    json!({
        "name": image.reference(),
        "alt": image.alt,
        "credit": image.credit,
        "source": image.source,
        "width": image.width,
        "height": image.height,
    })
}

/// What to say about the pictures `content` names that its artifact
/// does not keep: they show as nothing.
pub(super) fn missing(content: &Value, kept: &[Image]) -> Option<String> {
    let kept: BTreeSet<i64> = kept.iter().map(|i| i.id).collect();
    let gone: Vec<String> = images::referenced(content)
        .into_iter()
        .filter(|id| !kept.contains(id))
        .map(|id| format!("{}{id}", images::SCHEME))
        .collect();
    (!gone.is_empty()).then(|| {
        format!(
            "Saved, but this artifact keeps no picture {}, so nothing shows there. Keep one \
             with artifact_add_image and use the name it answers, or take the name out.",
            gone.join(", ")
        )
    })
}

fn only_this(tool: &str, only: Option<i64>, id: i64) -> Result<()> {
    match only {
        Some(only) if only != id => Err(invalid(
            tool,
            format!("this job keeps pictures for artifact {only}, and only that one"),
        )),
        _ => Ok(()),
    }
}

struct ImageSearch(niles_tools::Search);

#[async_trait]
impl Tool for ImageSearch {
    fn offered(&self) -> bool {
        self.0.configured()
    }

    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "image_search".into(),
            description: "Find pictures on the web for an artifact: a place, a dish, a \
                hotel. Each result is the picture's url, a thumbnail, the page it is on \
                (source), its size and who made it when known. Nothing is kept until \
                artifact_add_image keeps a copy; choose by the title, the source and the size \
                (a large one from the place's own site, a museum or Wikimedia Commons is \
                usually best)."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "What the picture is of: \"Grand Palace Bangkok\"" },
                    "num_results": { "type": "integer", "minimum": 1, "maximum": 20 }
                },
                "required": ["query"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        const TOOL: &str = "image_search";
        let query = str_arg(&args, "query")
            .ok_or_else(|| invalid(TOOL, "query is what the picture is of"))?;
        let n = args
            .get("num_results")
            .and_then(Value::as_u64)
            .map_or(RESULTS, |n| n.clamp(1, 20) as u8);
        let client = self.0.client().ok_or_else(|| {
            invalid(
                TOOL,
                "there is no image search any more: the SearXNG add-on was removed; do \
                 without pictures this time",
            )
        })?;
        let found = client.search_images(query, n).await.map_err(|e| {
            invalid(
                TOOL,
                format!("the image search did not answer ({e}); do without pictures this time"),
            )
        })?;
        let mut out = json!({
            "query": query,
            "images": found.iter().map(|i| json!({
                "title": i.title,
                "url": i.image_url,
                "thumbnail": i.thumbnail_url,
                "source": i.source,
                "size": i.size,
                "author": i.author,
            })).collect::<Vec<_>>(),
        });
        if found.is_empty() {
            out["note"] = json!(
                "No pictures were found. Try other words, once; if image_search finds none \
                 again, this Niles's search has no image engines on, so do without pictures."
            );
        }
        Ok(out)
    }
}

struct AddImage {
    artifacts: Arc<Artifacts>,
    /// In a job, the one artifact it may keep pictures for.
    only: Option<i64>,
}

#[async_trait]
impl Tool for AddImage {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "artifact_add_image".into(),
            description: format!(
                "Keep a copy of a picture with one of the asking person's artifacts: Niles \
                 fetches it from url (an https address of the picture itself: an \
                 image_search result's url, or one the person gave) and keeps its own copy, made smaller and without \
                 its metadata. Answers its name, niles-image:ID, to use in the content (a \
                 document's ![alt](niles-image:ID), a trip's image) or a page \
                 (niles.image(name)). alt says what it shows, for someone who cannot see it; \
                 credit who took or owns it; source the page it is from. An artifact keeps \
                 at most {} pictures. {}",
                images::MAX_PER_ARTIFACT,
                images::HELP
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "description": "The artifact's id" },
                    "url": { "type": "string" },
                    "alt": { "type": "string" },
                    "credit": { "type": "string" },
                    "source": { "type": "string" }
                },
                "required": ["id", "url", "alt"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        const TOOL: &str = "artifact_add_image";
        let asker = asker(TOOL)?;
        let id = id_arg(TOOL, &args)?;
        only_this(TOOL, self.only, id)?;
        let url = str_arg(&args, "url")
            .ok_or_else(|| invalid(TOOL, "url is the https address of the picture"))?;
        let alt = str_arg(&args, "alt").ok_or_else(|| {
            invalid(
                TOOL,
                "alt is what the picture shows, for someone who cannot see it",
            )
        })?;
        let Some(pictures) = &self.artifacts.pictures else {
            return Err(invalid(TOOL, "this Niles keeps no pictures"));
        };
        let store = &self.artifacts.store;
        let opened = self
            .artifacts
            .opened(TOOL, &asker.person, id, Operation::Edit)
            .await?;
        let (artifact, owner) = (&opened.artifact, opened.owner());
        // Room is asked before the download, so a full artifact costs no
        // fetch; the store asks again as it keeps it.
        let kept = store
            .images(owner, id)
            .await
            .map_err(|e| refused(TOOL, e))?;
        images::check_room(kept.len(), kept.iter().map(|i| i.size).sum(), 0)
            .map_err(|e| refused(TOOL, e))?;

        let raw = pictures.download.download(url).await.map_err(|why| {
            invalid(
                TOOL,
                format!("Niles could not fetch it: {why}. Choose another"),
            )
        })?;
        let picture = tokio::task::spawn_blocking(move || images::prepare(&raw))
            .await
            .map_err(|e| niles_tools::Error::Internal(e.to_string()))?
            .map_err(|e| refused(TOOL, e))?;
        let new = NewImage::new(
            picture,
            alt,
            str_arg(&args, "credit"),
            str_arg(&args, "source"),
        )
        .map_err(|e| refused(TOOL, e))?;
        let image = store
            .add_image(owner, id, new)
            .await
            .map_err(|e| refused(TOOL, e))?
            .ok_or_else(|| invalid(TOOL, format!("there is no artifact {id}")))?;
        tracing::info!(
            "[artifacts] {}: kept picture {} ({}×{}) for {:?}",
            asker.person,
            image.id,
            image.width,
            image.height,
            artifact.title
        );
        let mut out = view(&image);
        out["note"] = json!(format!(
            "Kept. Name it {} in the content{}, and save the content to show it.",
            image.reference(),
            match kinds::kind(&artifact.kind).map(|k| k.id) {
                Some("document") =>
                    format!(" on a line of its own: ![{alt}]({})", image.reference()),
                Some("trip") => " as the trip's, a day's or an item's image".to_string(),
                _ => String::new(),
            }
        ));
        Ok(out)
    }
}

struct RemoveImage {
    artifacts: Arc<Artifacts>,
    only: Option<i64>,
}

#[async_trait]
impl Tool for RemoveImage {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "artifact_remove_image".into(),
            description: "Take a picture away from one of the asking person's artifacts, by \
                its name (niles-image:ID). Where the content still names it, nothing shows: \
                take the name out of the content too."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "description": "The artifact's id" },
                    "image": { "type": "string", "description": "niles-image:ID" }
                },
                "required": ["id", "image"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        const TOOL: &str = "artifact_remove_image";
        let asker = asker(TOOL)?;
        let id = id_arg(TOOL, &args)?;
        only_this(TOOL, self.only, id)?;
        let image = match args.get("image") {
            Some(Value::Number(n)) => n.as_i64(),
            Some(Value::String(s)) => images::parse_reference(s).or_else(|| s.trim().parse().ok()),
            _ => None,
        }
        .ok_or_else(|| invalid(TOOL, "image is the picture's name, niles-image:ID"))?;
        let opened = self
            .artifacts
            .opened(TOOL, &asker.person, id, Operation::Edit)
            .await?;
        let gone = self
            .artifacts
            .store
            .remove_image(opened.owner(), id, image)
            .await
            .map_err(|e| refused(TOOL, e))?;
        if !gone {
            return Err(invalid(
                TOOL,
                format!("artifact {id} keeps no picture {}{image}", images::SCHEME),
            ));
        }
        Ok(json!({ "removed": format!("{}{image}", images::SCHEME) }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approvals::{self, Asker};
    use crate::artifacts::{Nobody, Runner};
    use niles_artifacts::{ArtifactStore, MemoryArtifacts, NewArtifact, NewRevision, Status};
    use niles_core::EventBus;
    use std::sync::Mutex;

    const MARK: &str = "person:mark";

    struct Idle;

    #[async_trait]
    impl Runner for Idle {
        fn available(&self) -> Option<&'static str> {
            None
        }

        async fn run(
            &self,
            _asker: Asker,
            _system: &str,
            _prompt: &str,
            _tools: ToolRegistry,
        ) -> std::result::Result<(&'static str, String), String> {
            Err("idle".into())
        }
    }

    /// Hands back the bytes it was given for an address, and remembers
    /// what it was asked for.
    #[derive(Default)]
    struct Files {
        at: Mutex<Vec<(String, Vec<u8>)>>,
        asked: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl Download for Files {
        async fn download(&self, url: &str) -> std::result::Result<Vec<u8>, String> {
            self.asked.lock().unwrap().push(url.to_string());
            self.at
                .lock()
                .unwrap()
                .iter()
                .find(|(u, _)| u == url)
                .map(|(_, b)| b.clone())
                .ok_or_else(|| "refused: not on the public internet".to_string())
        }
    }

    fn jpeg(width: u32, height: u32) -> Vec<u8> {
        let picture = ::image::RgbImage::from_pixel(width, height, ::image::Rgb([20, 120, 200]));
        let mut out = std::io::Cursor::new(Vec::new());
        picture
            .write_to(&mut out, ::image::ImageFormat::Jpeg)
            .unwrap();
        out.into_inner()
    }

    struct Fixture {
        artifacts: Arc<Artifacts>,
        store: Arc<MemoryArtifacts>,
        files: Arc<Files>,
    }

    fn fixture(search: Option<Arc<niles_websearch::SearXngClient>>) -> Fixture {
        let store = Arc::new(MemoryArtifacts::new());
        let files = Arc::new(Files::default());
        files.at.lock().unwrap().extend([
            (
                "https://upload.wikimedia.org/palace.jpg".to_string(),
                jpeg(2400, 1600),
            ),
            (
                "https://x.example/page".to_string(),
                b"<!doctype html><p>hi".to_vec(),
            ),
        ]);
        let artifacts = Arc::new(
            Artifacts::new(
                store.clone(),
                EventBus::default(),
                Arc::new(Idle),
                Arc::new(Nobody),
                None,
                Box::new(|_| {}),
            )
            .with_pictures(Pictures {
                search: niles_tools::Search::fixed(search, 5),
                download: files.clone(),
            }),
        );
        Fixture {
            artifacts,
            store,
            files,
        }
    }

    async fn document(store: &MemoryArtifacts, person: &str) -> i64 {
        store
            .create(
                person,
                NewArtifact {
                    kind: "document".into(),
                    title: "Bangkok guide".into(),
                    summary: String::new(),
                    notify: true,
                    status: Status::Ready,
                    status_text: None,
                    job_id: None,
                    brief: None,
                    conversation_id: None,
                    first: Some(NewRevision {
                        content: json!({ "markdown": "# Bangkok" }),
                        reason: "First draft".into(),
                        author: niles_artifacts::Author::Claude,
                        by: None,
                        summary: None,
                        page: niles_artifacts::PageChange::Keep,
                    }),
                },
            )
            .await
            .unwrap()
            .id
    }

    async fn call(tools: &ToolRegistry, person: &str, name: &str, args: Value) -> Result<Value> {
        approvals::ASKER
            .scope(
                Asker {
                    person: person.into(),
                    slug: None,
                    satellite: None,
                    conversation: None,
                    in_chat: false,
                },
                tools.execute(&niles_llm::ToolCall {
                    id: String::new(),
                    name: name.into(),
                    arguments: args,
                }),
            )
            .await
    }

    fn add(id: i64, url: &str) -> Value {
        json!({
            "id": id,
            "url": url,
            "alt": "The Grand Palace from the river",
            "credit": "Photo: J. Doe, CC BY-SA",
            "source": "https://commons.wikimedia.org/wiki/File:Palace.jpg",
        })
    }

    #[tokio::test]
    async fn a_picture_is_kept_as_niles_s_own_copy_and_named() {
        let f = fixture(None);
        let id = document(&f.store, MARK).await;
        let tools = f.artifacts.tools();
        let kept = call(
            &tools,
            MARK,
            "artifact_add_image",
            add(id, "https://upload.wikimedia.org/palace.jpg"),
        )
        .await
        .unwrap();
        let name = kept["name"].as_str().unwrap();
        assert!(images::parse_reference(name).is_some(), "{kept}");
        assert_eq!(
            (kept["width"].as_u64(), kept["height"].as_u64()),
            (Some(1600), Some(1067))
        );
        assert!(
            kept["note"]
                .as_str()
                .unwrap()
                .contains(&format!("![The Grand Palace from the river]({name})"))
        );
        let listed = f.store.images(MARK, id).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].credit.as_deref(), Some("Photo: J. Doe, CC BY-SA"));

        let got = call(&tools, MARK, "artifact_get", json!({ "id": id }))
            .await
            .unwrap();
        assert_eq!(got["images"][0]["name"], name);
        assert_eq!(got["images"][0]["alt"], "The Grand Palace from the river");
    }

    #[tokio::test]
    async fn what_is_not_a_picture_or_not_reachable_is_refused_saying_why() {
        let f = fixture(None);
        let id = document(&f.store, MARK).await;
        let tools = f.artifacts.tools();
        let e = call(
            &tools,
            MARK,
            "artifact_add_image",
            add(id, "https://x.example/page"),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(e.contains("web page"), "{e}");
        let e = call(
            &tools,
            MARK,
            "artifact_add_image",
            add(id, "https://10.0.0.1/a.jpg"),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(e.contains("could not fetch"), "{e}");
        let mut no_alt = add(id, "https://upload.wikimedia.org/palace.jpg");
        no_alt["alt"] = json!(" ");
        assert!(
            call(&tools, MARK, "artifact_add_image", no_alt)
                .await
                .is_err()
        );
        assert!(f.store.images(MARK, id).await.unwrap().is_empty());
        assert_eq!(
            f.files.asked.lock().unwrap().len(),
            2,
            "nothing fetched without alt"
        );
    }

    #[tokio::test]
    async fn a_picture_is_kept_only_for_the_asker_and_in_a_job_only_for_its_artifact() {
        let f = fixture(None);
        let hers = document(&f.store, "person:majse").await;
        let tools = f.artifacts.tools();
        let url = "https://upload.wikimedia.org/palace.jpg";
        let e = call(&tools, MARK, "artifact_add_image", add(hers, url))
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("no artifact"), "{e}");
        assert!(
            f.files.asked.lock().unwrap().is_empty(),
            "nothing fetched for her artifact"
        );

        let mine = document(&f.store, MARK).await;
        let job = f.artifacts.job_tools(mine, "document", None);
        let e = call(&job, MARK, "artifact_add_image", add(mine + 1, url))
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("only that one"), "{e}");
        assert!(
            call(&job, MARK, "artifact_add_image", add(mine, url))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn a_picture_is_kept_on_hers_shared_to_edit_and_never_on_one_shared_to_read() {
        use niles_artifacts::Level;
        let f = fixture(None);
        let majse = "person:majse";
        let to_edit = document(&f.store, majse).await;
        let to_read = document(&f.store, majse).await;
        f.store
            .set_share(majse, to_edit, MARK, Some(Level::Edit))
            .await
            .unwrap();
        f.store
            .set_share(majse, to_read, MARK, Some(Level::View))
            .await
            .unwrap();
        let tools = f.artifacts.tools();
        let url = "https://upload.wikimedia.org/palace.jpg";
        let kept = call(&tools, MARK, "artifact_add_image", add(to_edit, url))
            .await
            .unwrap();
        assert_eq!(f.store.images(majse, to_edit).await.unwrap().len(), 1);

        let e = call(&tools, MARK, "artifact_add_image", add(to_read, url))
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("to read only"), "{e}");
        assert_eq!(
            f.files.asked.lock().unwrap().len(),
            1,
            "nothing fetched for one he may only read"
        );
        f.store
            .set_share(majse, to_edit, MARK, Some(Level::View))
            .await
            .unwrap();
        let e = call(
            &tools,
            MARK,
            "artifact_remove_image",
            json!({ "id": to_edit, "image": kept["name"] }),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(e.contains("to read only"), "{e}");
        assert_eq!(f.store.images(majse, to_edit).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_full_artifact_fetches_nothing_more() {
        let f = fixture(None);
        let id = document(&f.store, MARK).await;
        let picture = images::prepare(&jpeg(4, 4)).unwrap();
        for i in 0..images::MAX_PER_ARTIFACT {
            let new = NewImage::new(picture.clone(), &format!("{i}"), None, None).unwrap();
            f.store.add_image(MARK, id, new).await.unwrap();
        }
        let e = call(
            &f.artifacts.tools(),
            MARK,
            "artifact_add_image",
            add(id, "https://upload.wikimedia.org/palace.jpg"),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(e.contains("artifact_remove_image"), "{e}");
        assert!(f.files.asked.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_picture_is_taken_away_by_its_name_and_a_save_says_what_shows_nothing() {
        let f = fixture(None);
        let id = document(&f.store, MARK).await;
        let tools = f.artifacts.tools();
        let kept = call(
            &tools,
            MARK,
            "artifact_add_image",
            add(id, "https://upload.wikimedia.org/palace.jpg"),
        )
        .await
        .unwrap();
        let name = kept["name"].as_str().unwrap().to_string();
        let saved = call(
            &tools,
            MARK,
            "artifact_update",
            json!({ "id": id, "content": { "markdown": format!("![Palace]({name})") }, "reason": "A picture" }),
        )
        .await
        .unwrap();
        assert!(saved.get("note").is_none(), "{saved}");

        let gone = call(
            &tools,
            MARK,
            "artifact_remove_image",
            json!({ "id": id, "image": name }),
        )
        .await
        .unwrap();
        assert_eq!(gone["removed"], name.as_str());
        assert!(
            call(
                &tools,
                MARK,
                "artifact_remove_image",
                json!({ "id": id, "image": name })
            )
            .await
            .is_err()
        );
        let saved = call(
            &tools,
            MARK,
            "artifact_update",
            json!({ "id": id, "content": { "markdown": format!("![Palace]({name})") }, "reason": "Again" }),
        )
        .await
        .unwrap();
        assert!(saved["note"].as_str().unwrap().contains(&name), "{saved}");
    }

    struct Answer(String);

    #[async_trait]
    impl niles_websearch::SearXngTransport for Answer {
        async fn get(&self, _url: &str) -> niles_websearch::Result<String> {
            Ok(self.0.clone())
        }
    }

    fn searching(body: &str) -> Arc<niles_websearch::SearXngClient> {
        Arc::new(niles_websearch::SearXngClient::with_transport(
            Arc::new(Answer(body.into())),
            niles_websearch::SearXngConfig {
                base_url: "https://search.example/search".into(),
                request_timeout: Duration::from_secs(5),
                user_agent: "test".into(),
            },
        ))
    }

    #[tokio::test]
    async fn image_search_offers_pictures_and_says_when_there_are_none() {
        let f = fixture(Some(searching(
            r#"{"results": [{"title": "Grand Palace", "url": "https://commons.wikimedia.org/wiki/File:P.jpg",
                "img_src": "https://upload.wikimedia.org/palace.jpg", "resolution": "2400 x 1600"}]}"#,
        )));
        let found = call(
            &f.artifacts.tools(),
            MARK,
            "image_search",
            json!({ "query": "Grand Palace Bangkok" }),
        )
        .await
        .unwrap();
        assert_eq!(
            found["images"][0]["url"],
            "https://upload.wikimedia.org/palace.jpg"
        );
        assert_eq!(found["images"][0]["size"], "2400 x 1600");
        assert!(found.get("note").is_none());

        let f = fixture(Some(searching(r#"{"results": []}"#)));
        let none = call(
            &f.artifacts.tools(),
            MARK,
            "image_search",
            json!({ "query": "x" }),
        )
        .await
        .unwrap();
        assert!(
            none["note"].as_str().unwrap().contains("no image engines"),
            "{none}"
        );

        let f = fixture(Some(searching("<html>502</html>")));
        let e = call(
            &f.artifacts.tools(),
            MARK,
            "image_search",
            json!({ "query": "x" }),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(e.contains("do without pictures"), "{e}");
    }

    #[tokio::test]
    async fn documents_trips_and_pages_have_pictures_and_a_training_plan_does_not() {
        let f = fixture(Some(searching(r#"{"results": []}"#)));
        for kind in ["document", "trip"] {
            let tools = f.artifacts.job_tools(1, kind, None);
            for tool in [
                "image_search",
                "artifact_add_image",
                "artifact_remove_image",
            ] {
                assert!(tools.has(tool), "{kind}: {tool}");
            }
        }
        let plan = f.artifacts.job_tools(1, "training_plan", None);
        assert!(!plan.has("artifact_add_image"));
        let page = f.artifacts.page_tools(1);
        assert!(page.has("image_search") && page.has("artifact_add_image"));

        let without = fixture(None);
        let trip = without.artifacts.job_tools(1, "trip", None);
        assert!(!trip.has("image_search"));
        assert!(
            !trip.llm_tools().iter().any(|t| t.name == "image_search"),
            "not offered without SearXNG"
        );
        assert!(
            without
                .artifacts
                .job_tools(1, "trip", None)
                .has("artifact_add_image")
        );
    }

    #[test]
    fn a_name_the_artifact_does_not_keep_is_pointed_out() {
        assert_eq!(missing(&json!({ "markdown": "no pictures" }), &[]), None);
        let note = missing(&json!({ "image": "niles-image:9" }), &[]).unwrap();
        assert!(note.contains("niles-image:9"), "{note}");
    }
}
