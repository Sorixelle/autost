use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

use crate::Author;

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct PostsResponse {
    pub nItems: usize,
    pub nPages: usize,
    pub items: Vec<Value>,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct Post {
    pub postId: usize,
    pub transparentShareOfPostId: Option<usize>,
    pub shareOfPostId: Option<usize>,
    pub filename: String,
    pub publishedAt: String,
    pub headline: String,
    pub tags: Vec<String>,
    pub postingProject: PostingProject,
    pub shareTree: Vec<Post>,

    /// markdown source only, without attachments or asks.
    pub plainTextBody: String,

    /// post body (markdown), attachments, and asks (markdown).
    pub blocks: Vec<Block>,

    /// fully rendered versions of markdown blocks.
    pub astMap: AstMap,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct PostingProject {
    pub handle: String,
    pub displayName: String,
    pub privacy: String,
    pub loggedOutPostVisibility: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(non_snake_case)]
pub enum Block {
    #[serde(rename = "markdown")]
    Markdown { markdown: Markdown },

    #[serde(rename = "attachment")]
    Attachment { attachment: Attachment },

    #[serde(rename = "attachment-row")]
    AttachmentRow { attachments: Vec<Block> },

    #[serde(rename = "ask")]
    Ask { ask: Ask },

    #[serde(untagged)]
    Unknown {
        #[serde(flatten)]
        fields: HashMap<String, Value>,
    },
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct Markdown {
    pub content: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind")]
#[allow(non_snake_case)]
pub enum Attachment {
    #[serde(rename = "image")]
    Image {
        attachmentId: String,
        altText: Option<String>,
        width: Option<usize>,
        height: Option<usize>,
    },

    #[serde(rename = "audio")]
    Audio {
        attachmentId: String,
        artist: String,
        title: String,
    },

    #[serde(untagged)]
    Unknown {
        #[serde(flatten)]
        fields: HashMap<String, Value>,
    },
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct Ask {
    pub content: String,
    pub askingProject: Option<AskingProject>,
    pub anon: bool,
    pub loggedIn: bool,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct AskingProject {
    pub handle: String,
    pub displayName: String,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct AstMap {
    pub spans: Vec<Span>,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct Span {
    pub ast: String,
    pub startIndex: usize,
    pub endIndex: usize,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct TrpcResponse<T> {
    pub result: TrpcResult<T>,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct TrpcResult<T> {
    pub data: T,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct ListEditedProjectsResponse {
    pub projects: Vec<EditedProject>,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct EditedProject {
    pub projectId: usize,
    pub handle: String,
    pub displayName: String,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct LoggedInResponse {
    pub projectId: usize,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(non_snake_case)]
pub enum Ast {
    #[serde(rename = "root")]
    Root { children: Vec<Ast> },

    #[serde(rename = "element")]
    Element {
        tagName: String,
        properties: HashMap<String, Value>,
        children: Vec<Ast>,
    },

    #[serde(rename = "text")]
    Text { value: String },
}

pub fn attachment_id_to_url(id: &str) -> String {
    format!("https://cohost.org/rc/attachment-redirect/{id}")
}

pub fn attachment_url_to_id(url: &str) -> Option<&str> {
    url.strip_prefix("https://cohost.org/rc/attachment-redirect/")
        .or_else(|| url.strip_prefix("https://cohost.org/api/v1/attachments/"))
        .or_else(|| url.strip_prefix("https://staging.cohostcdn.org/attachment/"))
        .filter(|id_plus| id_plus.len() >= 36)
        .map(|id_plus| &id_plus[..36])
}

pub fn custom_emoji_url_to_id(url: &str) -> Option<&str> {
    url.strip_prefix("https://cohost.org/static/")
        .and_then(|basename| basename.rsplit_once("."))
        .map(|(id, _extension)| id)
}

#[test]
fn test_attachment_url_to_id() {
    assert_eq!(
        attachment_url_to_id(
            "https://cohost.org/rc/attachment-redirect/44444444-4444-4444-4444-444444444444?query"
        ),
        Some("44444444-4444-4444-4444-444444444444")
    );
    assert_eq!(
        attachment_url_to_id(
            "https://cohost.org/api/v1/attachments/44444444-4444-4444-4444-444444444444?query"
        ),
        Some("44444444-4444-4444-4444-444444444444")
    );
    assert_eq!(attachment_url_to_id("https://staging.cohostcdn.org/attachment/44444444-4444-4444-4444-444444444444/file.jpg?query"), Some("44444444-4444-4444-4444-444444444444"));
}

impl From<&PostingProject> for Author {
    fn from(project: &PostingProject) -> Self {
        Self {
            href: format!("https://cohost.org/{}", project.handle),
            name: if project.displayName.is_empty() {
                format!("@{}", project.handle)
            } else {
                format!("{} (@{})", project.displayName, project.handle)
            },
            display_name: project.displayName.clone(),
            display_handle: format!("@{}", project.handle),
        }
    }
}

#[test]
fn test_author_from_posting_project() {
    assert_eq!(
        Author::from(&PostingProject {
            handle: "staff".to_owned(),
            displayName: "cohost dot org".to_owned(),
            privacy: "[any value]".to_owned(),
            loggedOutPostVisibility: "[any value]".to_owned(),
        }),
        Author {
            href: "https://cohost.org/staff".to_owned(),
            name: "cohost dot org (@staff)".to_owned(),
            display_name: "cohost dot org".to_owned(),
            display_handle: "@staff".to_owned(),
        }
    );
    assert_eq!(
        Author::from(&PostingProject {
            handle: "VinDuv".to_owned(),
            displayName: "".to_owned(),
            privacy: "[any value]".to_owned(),
            loggedOutPostVisibility: "[any value]".to_owned(),
        }),
        Author {
            href: "https://cohost.org/VinDuv".to_owned(),
            name: "@VinDuv".to_owned(),
            display_name: "".to_owned(),
            display_handle: "@VinDuv".to_owned(),
        }
    );
}
