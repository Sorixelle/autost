use std::{
    fs::{copy, create_dir_all, read_dir, File},
    io::{Read, Write},
    path::Path,
};

use jane_eyre::eyre::{self, bail, OptionExt};
use reqwest::redirect::Policy;
use sha2::{digest::generic_array::functional::FunctionalSequence, Digest, Sha256};
use tracing::{debug, trace, warn};
use uuid::Uuid;

use crate::{
    cohost::{attachment_id_to_url, Cacheable},
    path::AttachmentsPath,
};

pub trait AttachmentsContext {
    fn store(&self, input_path: &Path) -> eyre::Result<AttachmentsPath>;
    fn cache_imported(&self, url: &str, post_basename: &str) -> eyre::Result<AttachmentsPath>;
    fn cache_cohost_resource(&self, cacheable: &Cacheable) -> eyre::Result<AttachmentsPath>;
    fn cache_cohost_thumb(&self, id: &str) -> eyre::Result<AttachmentsPath>;
}

pub struct RealAttachmentsContext;
impl AttachmentsContext for RealAttachmentsContext {
    #[tracing::instrument(skip(self))]
    fn store(&self, input_path: &Path) -> eyre::Result<AttachmentsPath> {
        let dir = AttachmentsPath::ROOT.join(&Uuid::new_v4().to_string())?;
        create_dir_all(&dir)?;
        let filename = input_path.file_name().ok_or_eyre("no filename")?;
        let filename = filename.to_str().ok_or_eyre("unsupported filename")?;
        let path = dir.join(filename)?;
        copy(input_path, &path)?;

        Ok(path)
    }

    #[tracing::instrument(skip(self))]
    fn cache_imported(&self, url: &str, post_basename: &str) -> eyre::Result<AttachmentsPath> {
        let mut hash = Sha256::new();
        hash.update(url);
        let hash = hash.finalize().map(|o| format!("{o:02x}")).join("");
        let path = AttachmentsPath::ROOT.join(&format!("imported-{post_basename}-{hash}"))?;
        trace!(?path);
        create_dir_all(&path)?;

        cache_imported_attachment(url, &path)
    }

    #[tracing::instrument(skip(self))]
    fn cache_cohost_resource(&self, cacheable: &Cacheable) -> eyre::Result<AttachmentsPath> {
        match cacheable {
            Cacheable::Attachment { id } => {
                let url = attachment_id_to_url(id);
                let dir = &*AttachmentsPath::ROOT;
                let path = dir.join(id)?;
                create_dir_all(&path)?;
                cache_cohost_attachment(&url, &path, None)?;

                cached_attachment_url(id, dir)
            }

            Cacheable::Static { filename, url } => {
                let dir = &*AttachmentsPath::COHOST_STATIC;
                create_dir_all(dir)?;
                let path = dir.join(filename)?;
                trace!(?path);

                cache_other_cohost_resource(url, &path)
            }

            Cacheable::Avatar { filename, url } => {
                let dir = &*AttachmentsPath::COHOST_AVATAR;
                create_dir_all(dir)?;
                let path = dir.join(filename)?;
                trace!(?path);

                cache_other_cohost_resource(url, &path)
            }

            Cacheable::Header { filename, url } => {
                let dir = &*AttachmentsPath::COHOST_HEADER;
                create_dir_all(dir)?;
                let path = dir.join(filename)?;
                trace!(?path);

                cache_other_cohost_resource(url, &path)
            }
        }
    }

    #[tracing::instrument(skip(self))]
    fn cache_cohost_thumb(&self, id: &str) -> eyre::Result<AttachmentsPath> {
        fn thumb(url: &str) -> String {
            format!("{url}?width=675")
        }

        let url = attachment_id_to_url(id);
        let dir = &*AttachmentsPath::THUMBS;
        let path = dir.join(id)?;
        create_dir_all(&path)?;
        cache_cohost_attachment(&url, &path, Some(thumb))?;

        cached_attachment_url(id, dir)
    }
}

fn cached_attachment_url(id: &str, dir: &AttachmentsPath) -> eyre::Result<AttachmentsPath> {
    let path = dir.join(id)?;
    let mut entries = read_dir(&path)?;
    let Some(entry) = entries.next() else {
        bail!("directory is empty: {path:?}");
    };

    Ok(path.join_dir_entry(&entry?)?)
}

fn cache_imported_attachment(url: &str, path: &AttachmentsPath) -> eyre::Result<AttachmentsPath> {
    // if the attachment id directory exists...
    if let Ok(mut entries) = read_dir(&path) {
        // and the directory contains a file...
        if let Some(entry) = entries.next() {
            // and we can open the file...
            // TODO: move this logic into path module
            let path = path.join_dir_entry(&entry?)?;
            if let Ok(mut file) = File::open(&path) {
                trace!("cache hit: {url}");
                // check if we can read the file.
                let mut result = Vec::default();
                file.read_to_end(&mut result)?;
                return Ok(path);
            }
        }
    }

    trace!("cache miss");
    debug!("downloading attachment");

    let response = reqwest::blocking::get(url)?;
    let extension = match response.headers().get("Content-Type") {
        Some(x) if x == "image/gif" => "gif",
        Some(x) if x == "image/jpeg" => "jpg",
        Some(x) if x == "image/png" => "png",
        Some(x) if x == "image/svg+xml" => "svg",
        Some(x) if x == "image/webp" => "webp",
        other => {
            warn!("unknown attachment mime type: {other:?}");
            "bin"
        }
    };
    let path = path.join(&format!("file.{extension}"))?;
    debug!(?path);

    let result = response.bytes()?.to_vec();
    File::create(&path)?.write_all(&result)?;

    Ok(path)
}

/// given a cohost attachment redirect (`url`) and path to a uuid dir (`path`),
/// return the cached attachment path (`path/original-filename.ext`).
///
/// on cache miss, download the attachment from `url`, after first resolving the
/// redirect and transforming the resultant url (`transform_redirect_target`).
fn cache_cohost_attachment(
    url: &str,
    path: &AttachmentsPath,
    transform_redirect_target: Option<fn(&str) -> String>,
) -> eyre::Result<AttachmentsPath> {
    // if the attachment id directory exists...
    if let Ok(mut entries) = read_dir(path) {
        // and the directory contains a file...
        if let Some(entry) = entries.next() {
            // and we can open the file...
            // TODO: move this logic into path module
            let path = path.join_dir_entry(&entry?)?;
            if let Ok(mut file) = File::open(&path) {
                trace!("cache hit: {url}");
                // check if we can read the file.
                let mut result = Vec::default();
                file.read_to_end(&mut result)?;
                return Ok(path);
            }
        }
    }

    trace!("cache miss: {url}");
    debug!("downloading attachment");

    let client = reqwest::blocking::Client::builder()
        .redirect(Policy::none())
        .build()?;

    // attachment redirect endpoint occasionally returns 406 Not Acceptable
    let mut retries = 2;
    let mut redirect;
    let url = loop {
        redirect = client.head(url).send()?;
        let Some(url) = redirect.headers().get("location") else {
            if retries == 0 {
                bail!("expected redirect but got {}: {url}", redirect.status());
            } else {
                retries -= 1;
                continue;
            }
        };
        break url.to_str()?;
    };

    let Some((_, original_filename)) = url.rsplit_once("/") else {
        bail!("redirect target has no slashes: {url}");
    };
    let original_filename = urlencoding::decode(original_filename)?;
    trace!("original filename: {original_filename}");

    // cohost attachment redirects don’t preserve query params, so if we want to add any,
    // we need to add them to the destination of the redirect.
    // FIXME: this will silently misbehave if the endpoint introduces a second redirect!
    let url = if let Some(transform) = transform_redirect_target {
        let transformed_url = transform(url);
        trace!("transformed redirect target: {transformed_url}");
        transformed_url
    } else {
        url.to_owned()
    };

    let path = path.join(original_filename.as_ref())?;
    let result = reqwest::blocking::get(url)?.bytes()?.to_vec();
    File::create(&path)?.write_all(&result)?;

    Ok(path)
}

fn cache_other_cohost_resource(url: &str, path: &AttachmentsPath) -> eyre::Result<AttachmentsPath> {
    // if we can open the cached file...
    if let Ok(mut file) = File::open(path) {
        trace!("cache hit: {url}");
        // check if we can read the file.
        let mut result = Vec::default();
        file.read_to_end(&mut result)?;
        return Ok(path.clone());
    }

    trace!("cache miss");
    debug!("downloading resource");

    let response = reqwest::blocking::get(url)?;
    let result = response.bytes()?.to_vec();
    File::create(path)?.write_all(&result)?;

    Ok(path.clone())
}
