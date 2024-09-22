use std::{
    collections::BTreeMap,
    fs::{create_dir_all, read_dir, File},
    io::Write,
    path::{Path, PathBuf},
};

use askama::Template;
use autost::{
    AtomFeedTemplate, TemplatedPost, Thread, ThreadsContentTemplate, ThreadsTemplate, SETTINGS,
};
use chrono::{SecondsFormat, Utc};
use jane_eyre::eyre::{self, OptionExt};
use tracing::{debug, info, trace};

pub fn main(mut args: impl Iterator<Item = String>) -> eyre::Result<()> {
    let output_path = args.next().unwrap();
    let output_path = Path::new(&output_path);
    let mut args = args.peekable();

    if args.peek().is_some() {
        render(output_path, args)
    } else {
        render_all(output_path)
    }
}

pub fn render_all(output_path: &Path) -> eyre::Result<()> {
    let posts_path = PathBuf::from("posts");
    let mut post_paths = vec![];

    for entry in read_dir(posts_path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        // cohost2autost creates directories for chost thread ancestors.
        if metadata.is_dir() {
            continue;
        }

        let path = entry.path();
        let path = path.to_str().ok_or_eyre("unsupported path")?;
        post_paths.push(path.to_owned());
    }

    render(output_path, post_paths.into_iter())
}

pub fn render<'posts>(
    output_path: &Path,
    post_paths: impl Iterator<Item = String>,
) -> eyre::Result<()> {
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let mut collections = Collections::new([
        ("index", Collection::new(Some("index.feed.xml"), "posts")),
        ("all", Collection::new(None, "all posts")),
        (
            "untagged_interesting",
            Collection::new(None, "untagged interesting posts"),
        ),
        (
            "excluded",
            Collection::new(None, "archived posts that were marked excluded"),
        ),
        (
            "marked_interesting",
            Collection::new(None, "archived posts that were marked interesting"),
        ),
        (
            "skipped_own",
            Collection::new(None, "own skipped archived posts"),
        ),
        (
            "skipped_other",
            Collection::new(None, "others’ skipped archived posts"),
        ),
    ]);
    let mut threads_by_interesting_tag = BTreeMap::default();
    let mut tags = BTreeMap::default();

    for path in post_paths {
        let path = Path::new(&path);

        let mut post = TemplatedPost::load(&path)?;
        let extra_tags = SETTINGS
            .extra_archived_thread_tags(&post)
            .into_iter()
            .filter(|tag| !post.meta.tags.contains(tag))
            .map(|tag| tag.to_owned())
            .collect::<Vec<_>>();
        let combined_tags = extra_tags
            .into_iter()
            .chain(post.meta.tags.into_iter())
            .collect();
        let resolved_tags = SETTINGS.resolve_tags(combined_tags);
        post.meta.tags = resolved_tags;

        let filename = post.filename.clone();
        let meta = post.meta.clone();

        let mut posts = post
            .meta
            .references
            .iter()
            .flat_map(|filename| path.parent().map(|path| path.join(filename)))
            .map(|path| TemplatedPost::load(&path))
            .collect::<Result<Vec<_>, _>>()?;
        posts.push(post);

        // TODO: skip threads with other authors?
        // TODO: skip threads with private or logged-in-only authors?
        // TODO: gate sensitive posts behind an interaction?

        let overall_title = posts
            .iter()
            .rev()
            .find(|post| !post.meta.is_transparent_share)
            .and_then(|post| post.meta.title.clone())
            .unwrap_or("".to_owned());

        let thread = Thread {
            href: filename.clone(),
            posts,
            meta,
            overall_title: overall_title.clone(),
        };

        for tag in thread.meta.tags.iter() {
            *tags.entry(tag.clone()).or_insert(0usize) += 1;
        }
        collections.push("all", thread.clone());
        let mut was_interesting = false;
        if SETTINGS.thread_is_on_excluded_archived_list(&thread) {
            collections.push("excluded", thread.clone());
        } else if SETTINGS.thread_is_on_interesting_archived_list(&thread) {
            collections.push("marked_interesting", thread.clone());
            was_interesting = true;
        } else {
            for tag in thread.meta.tags.iter() {
                if SETTINGS.tag_is_interesting(tag) {
                    was_interesting = true;
                    break;
                }
            }
        }
        if was_interesting {
            collections.push("index", thread.clone());
            for tag in thread.meta.tags.iter() {
                if SETTINGS.tag_is_interesting(tag) {
                    threads_by_interesting_tag
                        .entry(tag.clone())
                        .or_insert(vec![])
                        .push(thread.clone());
                }
            }
            if thread.meta.tags.is_empty() {
                collections.push("untagged_interesting", thread.clone());
            }
        } else {
            // if the thread had some input from us at publish time, that is, if the last post was
            // authored by us with content and/or tags...
            if thread.posts.last().is_some_and(|post| {
                (!post.meta.is_transparent_share || !post.meta.tags.is_empty())
                    && post
                        .meta
                        .author
                        .as_ref()
                        .is_some_and(|author| SETTINGS.other_self_authors.contains(&author.href))
            }) {
                collections.push("skipped_own", thread.clone());
            } else {
                collections.push("skipped_other", thread.clone());
            }
        }

        // reader step: generate post page.
        let template = ThreadsContentTemplate {
            threads: vec![thread.clone()],
        };
        let content = template.render()?;
        let template = ThreadsTemplate {
            content,
            page_title: format!("{overall_title} — {}", SETTINGS.site_title),
            feed_href: None,
        };
        let path = output_path.join(filename);
        debug!("writing post page: {path:?}");
        writeln!(File::create(path)?, "{}", template.render()?)?;
    }

    for (_, threads) in threads_by_interesting_tag.iter_mut() {
        threads.sort_by(Thread::reverse_chronological);
    }
    trace!("threads by tag: {threads_by_interesting_tag:#?}");
    let tagged_path = output_path.join("tagged");
    create_dir_all(&tagged_path)?;

    // author step: generate atom feeds.
    let template = AtomFeedTemplate {
        threads: collections.threads("index").to_vec(),
        feed_title: SETTINGS.site_title.clone(),
        updated: now.clone(),
    };
    let atom_feed_path = output_path.join("index.feed.xml");
    writeln!(File::create(atom_feed_path)?, "{}", template.render()?)?;
    for (tag, threads) in threads_by_interesting_tag.clone().into_iter() {
        let template = AtomFeedTemplate {
            threads,
            feed_title: format!("{} — {tag}", SETTINGS.site_title),
            updated: now.clone(),
        };
        let atom_feed_path = tagged_path.join(format!("{tag}.feed.xml"));
        writeln!(File::create(atom_feed_path)?, "{}", template.render()?)?;
    }

    let mut tags = tags.into_iter().collect::<Vec<_>>();
    tags.sort_by(|p, q| p.1.cmp(&q.1).reverse().then(p.0.cmp(&q.0)));
    info!("all tags: {tags:?}");
    info!(
        "interesting tags: {:?}",
        tags.iter()
            .filter(|(tag, _)| SETTINGS.tag_is_interesting(tag))
            .collect::<Vec<_>>()
    );

    let interesting_tags_filenames = SETTINGS.interesting_tags_iter().flat_map(|tag| {
        [
            format!("tagged/{tag}.feed.xml"),
            format!("tagged/{tag}.html"),
        ]
    });
    let interesting_tags_posts_filenames = collections
        .threads("index")
        .iter()
        .map(|thread| thread.href.clone());
    let interesting_filenames = vec!["index.html".to_owned(), "index.feed.xml".to_owned()]
        .into_iter()
        .chain(interesting_tags_filenames)
        .chain(interesting_tags_posts_filenames)
        .map(|filename| format!("{}\n", filename))
        .collect::<Vec<_>>()
        .join("");
    if let Some(path) = &SETTINGS.interesting_output_filenames_list_path {
        File::create(path)?.write_all(interesting_filenames.as_bytes())?;
    }

    // reader step: generate posts pages.
    for key in collections.keys() {
        info!(
            "writing threads page for collection {key:?} ({} threads)",
            collections.threads(key).len()
        );
        collections.write_threads_page(key, output_path)?;
    }
    for (tag, threads) in threads_by_interesting_tag.into_iter() {
        let template = ThreadsContentTemplate { threads };
        let content = template.render()?;
        let template = ThreadsTemplate {
            content,
            page_title: format!("#{tag} — {}", SETTINGS.site_title),
            feed_href: Some(format!("tagged/{tag}.feed.xml")),
        };
        let posts_page_path = tagged_path.join(format!("{tag}.html"));
        writeln!(File::create(posts_page_path)?, "{}", template.render()?)?;
    }

    Ok(())
}

struct Collections {
    inner: BTreeMap<&'static str, Collection>,
}

struct Collection {
    feed_href: Option<String>,
    title: String,
    threads: Vec<Thread>,
}

impl Collections {
    fn new(collections: impl IntoIterator<Item = (&'static str, Collection)>) -> Self {
        Self {
            inner: collections.into_iter().collect(),
        }
    }

    fn keys(&self) -> impl Iterator<Item = &str> {
        self.inner.keys().map(|key| *key)
    }

    fn threads(&self, key: &str) -> &[Thread] {
        &self.inner[key].threads
    }

    fn push(&mut self, key: &str, thread: Thread) {
        self.inner
            .get_mut(key)
            .expect("BUG: unknown collection!")
            .threads
            .push(thread);
    }

    fn write_threads_page(&self, key: &str, output_path: &Path) -> eyre::Result<()> {
        self.inner[key].write_threads_page(&output_path.join(format!("{key}.html")))
    }
}

impl Collection {
    fn new(feed_href: Option<&str>, title: &str) -> Self {
        Self {
            feed_href: feed_href.map(|href| href.to_owned()),
            title: title.to_owned(),
            threads: vec![],
        }
    }

    fn write_threads_page(&self, posts_page_path: &Path) -> eyre::Result<()> {
        let mut threads = self.threads.clone();
        threads.sort_by(Thread::reverse_chronological);
        let template = ThreadsContentTemplate { threads };
        let content = template.render()?;
        let template = ThreadsTemplate {
            content,
            page_title: format!("{} — {}", self.title, SETTINGS.site_title),
            feed_href: self.feed_href.clone(),
        };
        writeln!(File::create(posts_page_path)?, "{}", template.render()?)?;

        Ok(())
    }
}
