// SPDX-License-Identifier: AGPL-3.0-or-later
//! A creative's own People page design (DESIGN §15.1), from their portfolio repository.
//!
//! Beside `.slop/portfolio.json`, a creative can add `.slop/page/index.html` (up to 64 KB) and
//! `.slop/page/style.css` (up to 32 KB). Slop reads them at the portfolio's commit, with it, so a
//! design is reviewed and approved together with the works: what was reviewed is what shows, and
//! a changed design is a new portfolio set.
//!
//! Neither file is used as written:
//!
//! - The HTML is parsed as a browser would (html5ever) and rebuilt as a tree of allowed elements
//!   and attributes ([`Node`]). The website draws that tree itself, element by element, never as
//!   HTML, so nothing in it can run. `<slop-…>` tags stand for Slop's own parts (a work, the works,
//!   a YouTube video, a track, the services), which the website draws in their place.
//! - The CSS is parsed (cssparser) and rebuilt from allowed rules, selectors, properties and
//!   values. Pictures in `url()` and `<img>` come from the repository at that commit, checked like
//!   the works.
//!
//! The website shows the design in a shadow root inside a box that contains its layout and paint
//! (People.tsx), under Slop's own header and above Slop's reviews. What isn't allowed is dropped
//! and said in a warning; what can't work (a missing picture, a file over its limit, a
//! `<slop-work>` that isn't a work) is a problem, like a broken portfolio.json.

use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, HashMap},
    rc::{Rc, Weak},
};

use cssparser::{
    AtRuleParser, BasicParseErrorKind, CowRcStr, DeclarationParser, ParseError, ParseErrorKind,
    Parser, ParserInput, ParserState, QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser,
    StyleSheetParser, ToCss, Token, TokenSerializationType, serialize_identifier, serialize_string,
};
use html5ever::{
    Attribute, QualName,
    interface::{ElementFlags, NodeOrText, QuirksMode, TreeSink},
    tendril::{StrTendril, TendrilSink},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::portfolio::{self, AudioFile, MAX_WORKS, Work};
use crate::{
    AppState,
    media::{self, Picture, Problem, Reader},
};

pub const HTML_PATH: &str = ".slop/page/index.html";
pub const CSS_PATH: &str = ".slop/page/style.css";
pub const MAX_HTML_BYTES: u64 = 64 * 1024;
pub const MAX_CSS_BYTES: u64 = 32 * 1024;
/// Elements in one page, Slop's parts included.
const MAX_ELEMENTS: usize = 2_000;
const MAX_DEPTH: usize = 32;
const MAX_WARNINGS: usize = 60;
const MAX_CLASSES: usize = 8;
const MAX_CLASS_LEN: usize = 40;
const MAX_TITLE: usize = 80;
const MAX_ALT: usize = 300;
const MAX_HREF: usize = 500;
/// z-index stays small: the design can layer its own parts, not reach above Slop's.
const MAX_Z: i32 = 100;
/// Pictures used only in the design can be small (a pattern tile, a divider).
const PICTURE_MIN: (u32, u32) = (1, 1);

/// Font families a design can use. Slop serves them itself (web/public/fonts, all SIL Open Font
/// License): a clean sans, an editorial serif, a pixel font, a handwritten one, a monospace and
/// a display face.
pub const FONTS: &[&str] = &[
    "Inter",
    "Fraunces",
    "Pixelify Sans",
    "Caveat",
    "JetBrains Mono",
    "Bebas Neue",
];
const GENERIC_FONTS: &[&str] = &["serif", "sans-serif", "monospace", "cursive", "system-ui"];

/// Elements a page can use, as they are.
pub const TAGS: &[&str] = &[
    "section",
    "div",
    "header",
    "footer",
    "article",
    "aside",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "p",
    "span",
    "strong",
    "em",
    "b",
    "i",
    "u",
    "s",
    "small",
    "mark",
    "code",
    "pre",
    "sub",
    "sup",
    "br",
    "hr",
    "ul",
    "ol",
    "li",
    "dl",
    "dt",
    "dd",
    "blockquote",
    "figure",
    "figcaption",
    "table",
    "caption",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "th",
    "td",
    "a",
    "img",
];
const VOID: &[&str] = &["br", "hr", "img"];
/// Whitespace between these elements' children isn't content (and React refuses it in tables).
const NO_TEXT: &[&str] = &["table", "thead", "tbody", "tfoot", "tr"];

/// Slop's own parts, drawn by the website where the design puts them.
pub const SLOP_TAGS: &[&str] = &[
    "slop-work",
    "slop-works",
    "slop-video",
    "slop-track",
    "slop-services",
];

/// Links get these, whatever the page says.
pub const LINK_REL: &str = "nofollow noopener noreferrer ugc";

// ---------------------------------------------------------------------------------------------
// What's stored

/// One node of a design's page: text, or an allowed element (or one of Slop's parts).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Node {
    Text(String),
    Element {
        tag: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        attrs: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        children: Vec<Node>,
    },
}

/// A checked design, stored with its portfolio set (`portfolio_sets.design`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Design {
    /// The page, as the website draws it. Pictures and tracks are linked at the commit.
    pub tree: Vec<Node>,
    /// The same page as HTML, for reviewers.
    pub html: String,
    /// The rebuilt style sheet.
    pub css: String,
    /// Pictures the page uses (`<img>` and `url()`), checked like the works.
    pub pictures: Vec<Picture>,
    /// Tracks the page plays (`<slop-track>`), checked like the works.
    pub tracks: Vec<AudioFile>,
    /// What was removed and why, for the creative and the reviewer.
    pub warnings: Vec<String>,
    /// The page places Slop's services itself (`<slop-services>`); otherwise they show below it.
    pub has_services: bool,
}

/// What a People page shows of a design.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct PageDesign {
    pub tree: Vec<Node>,
    pub css: String,
    pub has_services: bool,
}

impl From<Design> for PageDesign {
    fn from(d: Design) -> Self {
        Self {
            tree: d.tree,
            css: d.css,
            has_services: d.has_services,
        }
    }
}

/// Warnings, without repeats, up to a limit.
#[derive(Default)]
struct Warnings(Vec<String>);

impl Warnings {
    fn push(&mut self, w: String) {
        if !self.0.contains(&w) {
            self.0.push(w);
        }
    }

    fn finish(mut self) -> Vec<String> {
        if self.0.len() > MAX_WARNINGS {
            let more = self.0.len() - MAX_WARNINGS;
            self.0.truncate(MAX_WARNINGS);
            self.0.push(format!("…and {more} more like these"));
        }
        self.0
    }
}

// ---------------------------------------------------------------------------------------------
// Reading .slop/page/

/// Reads the design at the portfolio's commit (`base` is its `.slop/` folder). None when the
/// repository has no `.slop/page/index.html`. Also returns a fingerprint of what it is (not where
/// it was read from), for telling a new design from the one already shown or waiting.
pub(crate) async fn read(
    state: &AppState,
    base: &url::Url,
    reader: &mut Reader<'_>,
    works: &[Work],
) -> Result<Option<(Design, String)>, Problem> {
    let html = fetch_text(state, base, "page/index.html", HTML_PATH, MAX_HTML_BYTES).await?;
    let css = fetch_text(state, base, "page/style.css", CSS_PATH, MAX_CSS_BYTES).await?;
    let Some(html) = html else {
        if css.is_some() {
            return Err(Problem(format!(
                "{CSS_PATH} needs an {HTML_PATH} beside it: the page is the HTML, styled by the CSS"
            )));
        }
        return Ok(None);
    };
    let draft = sanitize(&html, css.as_deref(), works)?;

    // Every file the design adds counts with the works.
    let mut work_files = BTreeSet::new();
    for w in works {
        match w {
            Work::Image { picture, .. } => {
                work_files.insert(picture.path.as_str());
            }
            Work::Video { poster, .. } => {
                work_files.extend(poster.iter().map(|p| p.path.as_str()));
            }
            Work::Audio { audio, poster, .. } => {
                work_files.insert(audio.path.as_str());
                work_files.extend(poster.iter().map(|p| p.path.as_str()));
            }
        }
    }
    let extra: BTreeSet<&str> = draft
        .pictures
        .iter()
        .chain(&draft.tracks)
        .map(|(path, _)| path.as_str())
        .filter(|p| !work_files.contains(p))
        .collect();
    if works.len() + extra.len() > MAX_WORKS {
        return Err(Problem(format!(
            "your page's design uses {} pictures and tracks that aren't works, and portfolio.json lists {} works: together they can be up to {MAX_WORKS} files. Use fewer, or reuse your works' pictures",
            extra.len(),
            works.len()
        )));
    }

    let mut urls = HashMap::new();
    let mut pictures = Vec::with_capacity(draft.pictures.len());
    for (path, at) in &draft.pictures {
        let picture = reader
            .picture(path, PICTURE_MIN)
            .await
            .map_err(|Problem(p)| Problem(format!("{at}: {p}")))?;
        urls.insert(path.clone(), picture.url.clone());
        pictures.push(picture);
    }
    let mut tracks = Vec::with_capacity(draft.tracks.len());
    for (path, at) in &draft.tracks {
        let track = portfolio::read_audio(state, base, path)
            .await
            .map_err(|Problem(p)| Problem(format!("{at}: {p}")))?;
        urls.insert(path.clone(), track.url.clone());
        tracks.push(track);
    }

    let fingerprint = {
        let value = json!([
            draft.tree,
            draft.css,
            pictures
                .iter()
                .map(|p| [&p.path, &p.sha256])
                .collect::<Vec<_>>(),
            tracks
                .iter()
                .map(|t| [&t.path, &t.sha256])
                .collect::<Vec<_>>(),
        ]);
        hex::encode(Sha256::digest(value.to_string().as_bytes()))
    };
    let mut tree = draft.tree;
    link_files(&mut tree, &pictures, &tracks);
    let css = css
        .as_deref()
        .map(|c| sanitize_css(c, &urls).css)
        .unwrap_or_default();
    let mut html = String::new();
    to_html(&tree, &mut html);
    Ok(Some((
        Design {
            tree,
            html,
            css,
            pictures,
            tracks,
            warnings: draft.warnings,
            has_services: draft.has_services,
        },
        fingerprint,
    )))
}

/// A text file in `.slop/`, checked for size and UTF-8. None when it isn't there.
async fn fetch_text(
    state: &AppState,
    base: &url::Url,
    path: &str,
    name: &str,
    max: u64,
) -> Result<Option<String>, Problem> {
    let bytes = match media::download(state, base.join(path)?, max).await {
        Ok(Some(b)) => b,
        Ok(None) => return Ok(None),
        Err(e) if e.to_string() == "too big" => {
            return Err(Problem(format!(
                "{name} is over {} KB: keep it under that (pictures are separate files)",
                max / 1024
            )));
        }
        Err(e) => return Err(Problem(format!("couldn't download {name}: {e:#}"))),
    };
    text(name, bytes).map(Some)
}

fn text(name: &str, bytes: Vec<u8>) -> Result<String, Problem> {
    String::from_utf8(bytes).map_err(|e| {
        let at = e.utf8_error().valid_up_to();
        let line = 1 + e.as_bytes()[..at].iter().filter(|b| **b == b'\n').count();
        Problem(format!(
            "{name} line {line} isn't UTF-8 text: save the file as UTF-8"
        ))
    })
}

/// A design before its files are fetched: pictures and tracks are still paths in `.slop/`.
#[derive(Debug)]
pub struct Draft {
    pub tree: Vec<Node>,
    /// The style sheet, with `url()`s still naming paths.
    pub css: String,
    /// Pictures, as (path in `.slop/`, where it's used), each once.
    pub pictures: Vec<(String, String)>,
    pub tracks: Vec<(String, String)>,
    pub warnings: Vec<String>,
    pub has_works: bool,
    pub has_services: bool,
}

/// Checks and rebuilds a design's files, without fetching anything: what `read` does first, and
/// what the tests (and the templates' test) call.
pub fn sanitize(html: &str, css: Option<&str>, works: &[Work]) -> Result<Draft, Problem> {
    let mut walk = HtmlWalk {
        works,
        warnings: Warnings::default(),
        elements: 0,
        pictures: Vec::new(),
        tracks: Vec::new(),
        has_works: false,
        has_services: false,
    };
    let tree = walk.document(html)?;
    let mut warnings = walk.warnings;
    let mut pictures = walk.pictures;
    let mut css_out = String::new();
    if let Some(css) = css {
        let out = sanitize_css(css, &HashMap::new());
        for w in out.warnings {
            warnings.push(w);
        }
        for (path, line) in out.paths {
            if !pictures.iter().any(|(p, _)| *p == path) {
                pictures.push((path, format!("{CSS_PATH} line {line}")));
            }
        }
        css_out = out.css;
    }
    Ok(Draft {
        tree,
        css: css_out,
        pictures,
        tracks: walk.tracks,
        warnings: warnings.finish(),
        has_works: walk.has_works,
        has_services: walk.has_services,
    })
}

/// A path in the page's files, as a browser would read it from `.slop/page/`: `images/x.png`
/// is `.slop/page/images/x.png`, `../works/x.png` is `.slop/works/x.png`. Returns the path in
/// `.slop/`, or why it can't be used.
fn page_path(raw: &str) -> Result<String, &'static str> {
    let raw = raw.trim();
    let lower = raw.to_ascii_lowercase();
    if raw.is_empty() {
        return Err("it's empty");
    }
    if lower.contains(':') || raw.starts_with("//") {
        return Err(
            "files come from your repository: use a path like images/hero.png (from .slop/page/) or ../works/knight.png",
        );
    }
    if raw.starts_with('/') {
        return Err(
            "paths start from .slop/page/: use images/hero.png, or ../works/knight.png for .slop/works/",
        );
    }
    let mut parts = vec!["page"];
    for seg in raw.split('/') {
        match seg {
            "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err("../ can't go above .slop/");
                }
            }
            s => parts.push(s),
        }
    }
    Ok(parts.join("/"))
}

fn picture_path(raw: &str) -> Result<String, String> {
    let path = page_path(raw).map_err(str::to_string)?;
    media::clean_file_path(
        &path,
        &[".png", ".jpg", ".jpeg", ".webp", ".gif"],
        "a picture: use .png, .jpg, .webp or .gif",
    )
    .map_err(|Problem(p)| p)
}

fn track_path(raw: &str) -> Result<String, String> {
    let path = page_path(raw).map_err(str::to_string)?;
    media::clean_file_path(
        &path,
        &[".mp3", ".ogg", ".oga", ".opus", ".wav", ".flac", ".m4a"],
        "an audio file: use .mp3, .ogg, .wav, .flac or .m4a",
    )
    .map_err(|Problem(p)| p)
}

/// Puts the fetched files' links where the draft named their paths.
fn link_files(nodes: &mut [Node], pictures: &[Picture], tracks: &[AudioFile]) {
    for node in nodes {
        let Node::Element {
            tag,
            attrs,
            children,
        } = node
        else {
            continue;
        };
        match tag.as_str() {
            "img" => {
                if let Some(p) = attrs
                    .get("src")
                    .and_then(|src| pictures.iter().find(|p| p.path == *src))
                {
                    attrs.insert("src".into(), p.url.clone());
                    attrs.insert("width".into(), p.width.to_string());
                    attrs.insert("height".into(), p.height.to_string());
                }
            }
            "slop-track" => {
                if let Some(t) = attrs
                    .get("src")
                    .and_then(|src| tracks.iter().find(|t| t.path == *src))
                {
                    attrs.insert("src".into(), t.url.clone());
                    attrs.insert("format".into(), t.format.clone());
                }
            }
            _ => {}
        }
        link_files(children, pictures, tracks);
    }
}

/// The tree as HTML, for reviewers to read.
pub fn to_html(nodes: &[Node], out: &mut String) {
    fn escape(s: &str, out: &mut String, attr: bool) {
        for c in s.chars() {
            match c {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' if attr => out.push_str("&quot;"),
                c => out.push(c),
            }
        }
    }
    for node in nodes {
        match node {
            Node::Text(t) => escape(t, out, false),
            Node::Element {
                tag,
                attrs,
                children,
            } => {
                out.push('<');
                out.push_str(tag);
                for (k, v) in attrs {
                    out.push(' ');
                    out.push_str(k);
                    out.push_str("=\"");
                    escape(v, out, true);
                    out.push('"');
                }
                out.push('>');
                if !VOID.contains(&tag.as_str()) {
                    to_html(children, out);
                    out.push_str("</");
                    out.push_str(tag);
                    out.push('>');
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// HTML: parsed by html5ever into a small DOM that remembers each element's line

type Handle = Rc<DomNode>;

struct DomNode {
    parent: RefCell<Option<Weak<DomNode>>>,
    children: RefCell<Vec<Handle>>,
    data: DomData,
    line: u64,
}

enum DomData {
    Document,
    Text(RefCell<String>),
    Element {
        name: QualName,
        attrs: RefCell<Vec<Attribute>>,
        template: Option<Handle>,
    },
    /// Comments, doctypes, processing instructions: never shown.
    Other,
}

struct Sink {
    document: Handle,
    line: Cell<u64>,
}

impl Sink {
    fn node(&self, data: DomData) -> Handle {
        Rc::new(DomNode {
            parent: RefCell::new(None),
            children: RefCell::new(Vec::new()),
            data,
            line: self.line.get(),
        })
    }

    fn detach(target: &Handle) {
        let parent = target.parent.borrow_mut().take().and_then(|p| p.upgrade());
        if let Some(parent) = parent {
            parent
                .children
                .borrow_mut()
                .retain(|c| !Rc::ptr_eq(c, target));
        }
    }

    fn adopt(parent: &Handle, child: &Handle) {
        *child.parent.borrow_mut() = Some(Rc::downgrade(parent));
    }
}

impl TreeSink for Sink {
    type Handle = Handle;
    type Output = Handle;
    type ElemName<'a>
        = &'a QualName
    where
        Self: 'a;

    fn finish(self) -> Handle {
        self.document
    }

    fn parse_error(&self, _msg: Cow<'static, str>) {}

    fn get_document(&self) -> Handle {
        self.document.clone()
    }

    fn elem_name<'a>(&'a self, target: &'a Handle) -> &'a QualName {
        match &target.data {
            DomData::Element { name, .. } => name,
            _ => panic!("not an element"),
        }
    }

    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> Handle {
        let template = flags.template.then(|| self.node(DomData::Document));
        self.node(DomData::Element {
            name,
            attrs: RefCell::new(attrs),
            template,
        })
    }

    fn create_comment(&self, _text: StrTendril) -> Handle {
        self.node(DomData::Other)
    }

    fn create_pi(&self, _target: StrTendril, _data: StrTendril) -> Handle {
        self.node(DomData::Other)
    }

    fn append(&self, parent: &Handle, child: NodeOrText<Handle>) {
        match child {
            NodeOrText::AppendText(text) => {
                if let Some(last) = parent.children.borrow().last()
                    && let DomData::Text(t) = &last.data
                {
                    t.borrow_mut().push_str(&text);
                    return;
                }
                let node = self.node(DomData::Text(RefCell::new(text.to_string())));
                Sink::adopt(parent, &node);
                parent.children.borrow_mut().push(node);
            }
            NodeOrText::AppendNode(node) => {
                Sink::detach(&node);
                Sink::adopt(parent, &node);
                parent.children.borrow_mut().push(node);
            }
        }
    }

    fn append_based_on_parent_node(
        &self,
        element: &Handle,
        prev_element: &Handle,
        child: NodeOrText<Handle>,
    ) {
        let has_parent = element
            .parent
            .borrow()
            .as_ref()
            .and_then(Weak::upgrade)
            .is_some();
        if has_parent {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev_element, child);
        }
    }

    fn append_doctype_to_document(&self, _: StrTendril, _: StrTendril, _: StrTendril) {}

    fn get_template_contents(&self, target: &Handle) -> Handle {
        match &target.data {
            DomData::Element {
                template: Some(t), ..
            } => t.clone(),
            _ => panic!("not a template"),
        }
    }

    fn same_node(&self, x: &Handle, y: &Handle) -> bool {
        Rc::ptr_eq(x, y)
    }

    fn set_quirks_mode(&self, _mode: QuirksMode) {}

    fn append_before_sibling(&self, sibling: &Handle, child: NodeOrText<Handle>) {
        let Some(parent) = sibling.parent.borrow().as_ref().and_then(Weak::upgrade) else {
            return;
        };
        let at = parent
            .children
            .borrow()
            .iter()
            .position(|c| Rc::ptr_eq(c, sibling))
            .unwrap_or(0);
        let node = match child {
            NodeOrText::AppendText(text) => {
                if at > 0
                    && let DomData::Text(t) = &parent.children.borrow()[at - 1].data
                {
                    t.borrow_mut().push_str(&text);
                    return;
                }
                self.node(DomData::Text(RefCell::new(text.to_string())))
            }
            NodeOrText::AppendNode(node) => {
                Sink::detach(&node);
                node
            }
        };
        // Detaching may have moved the sibling.
        let at = parent
            .children
            .borrow()
            .iter()
            .position(|c| Rc::ptr_eq(c, sibling))
            .unwrap_or(0);
        Sink::adopt(&parent, &node);
        parent.children.borrow_mut().insert(at, node);
    }

    fn add_attrs_if_missing(&self, target: &Handle, attrs: Vec<Attribute>) {
        if let DomData::Element {
            attrs: existing, ..
        } = &target.data
        {
            let mut existing = existing.borrow_mut();
            for a in attrs {
                if !existing.iter().any(|e| e.name == a.name) {
                    existing.push(a);
                }
            }
        }
    }

    fn remove_from_parent(&self, target: &Handle) {
        Sink::detach(target);
    }

    fn reparent_children(&self, node: &Handle, new_parent: &Handle) {
        let children = std::mem::take(&mut *node.children.borrow_mut());
        for child in children {
            Sink::adopt(new_parent, &child);
            new_parent.children.borrow_mut().push(child);
        }
    }

    fn set_current_line(&self, line: u64) {
        self.line.set(line);
    }
}

fn element_name(node: &Handle) -> Option<&str> {
    match &node.data {
        DomData::Element { name, .. } if name.ns == html5ever::ns!(html) => Some(&name.local),
        _ => None,
    }
}

/// Why a tag is removed with what's inside it.
fn dropped(tag: &str) -> Option<&'static str> {
    Some(match tag {
        "script" | "noscript" => "pages can't run scripts",
        "style" | "link" => "styles go in style.css",
        "iframe" | "frame" | "frameset" | "object" | "embed" | "applet" | "portal"
        | "fencedframe" | "param" => {
            "pages can't show other pages: use <slop-video> for a YouTube video"
        }
        "input" | "button" | "select" | "textarea" | "option" | "optgroup" | "datalist"
        | "output" => "pages can't have buttons or forms: Slop's Hire button is above your page",
        "svg" | "math" | "canvas" => "use a picture file (PNG, JPEG, WebP or GIF) instead",
        "video" | "audio" | "source" | "track" | "picture" | "map" | "area" => {
            "use <slop-video> for videos, <slop-track> for tracks and <img> for pictures"
        }
        "meta" | "base" | "title" => "Slop's page sets these itself",
        "template" | "slot" | "dialog" | "xmp" | "plaintext" | "noembed" | "noframes" => {
            "it isn't supported"
        }
        _ => return None,
    })
}

struct HtmlWalk<'w> {
    works: &'w [Work],
    warnings: Warnings,
    elements: usize,
    /// (path in .slop/, where it's first used)
    pictures: Vec<(String, String)>,
    tracks: Vec<(String, String)>,
    has_works: bool,
    has_services: bool,
}

struct At {
    line: u64,
}

impl std::fmt::Display for At {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{HTML_PATH} line {}", self.line)
    }
}

impl HtmlWalk<'_> {
    fn warn(&mut self, line: u64, what: String) {
        self.warnings.push(format!("{}: {what}", At { line }));
    }

    fn document(&mut self, src: &str) -> Result<Vec<Node>, Problem> {
        let sink = Sink {
            document: Rc::new(DomNode {
                parent: RefCell::new(None),
                children: RefCell::new(Vec::new()),
                data: DomData::Document,
                line: 1,
            }),
            line: Cell::new(1),
        };
        let doc = html5ever::parse_document(sink, Default::default()).one(src);
        let mut out = Vec::new();
        let html = doc
            .children
            .borrow()
            .iter()
            .find(|c| element_name(c) == Some("html"))
            .cloned();
        let Some(html) = html else {
            return Ok(out);
        };
        for part in html.children.borrow().iter() {
            match element_name(part) {
                Some("head") => {
                    for child in part.children.borrow().iter() {
                        if let Some(tag) = element_name(child) {
                            let why = dropped(tag).unwrap_or("it isn't shown");
                            self.warn(child.line, format!("<{tag}> was removed: {why}"));
                        }
                    }
                }
                Some("body") => self.children(part, 0, false, "body", &mut out)?,
                _ => {}
            }
        }
        Ok(out)
    }

    fn children(
        &mut self,
        node: &Handle,
        depth: usize,
        in_link: bool,
        parent: &str,
        out: &mut Vec<Node>,
    ) -> Result<(), Problem> {
        let children = node.children.borrow().clone();
        for child in &children {
            self.node(child, depth, in_link, parent, out)?;
        }
        Ok(())
    }

    fn node(
        &mut self,
        node: &Handle,
        depth: usize,
        in_link: bool,
        parent: &str,
        out: &mut Vec<Node>,
    ) -> Result<(), Problem> {
        let line = node.line;
        let (name, attrs) = match &node.data {
            DomData::Text(t) => {
                let text = self.clean_text(&t.borrow(), line);
                if text.trim().is_empty() {
                    if NO_TEXT.contains(&parent) || text.is_empty() {
                        return Ok(());
                    }
                    push_text(out, " ");
                } else {
                    push_text(out, &text);
                }
                return Ok(());
            }
            DomData::Element { name, attrs, .. } => (name, attrs.borrow().clone()),
            _ => return Ok(()),
        };
        let tag: &str = &name.local;
        if name.ns != html5ever::ns!(html) {
            // <svg> or <math>, removed whole (nothing inside them is looked at).
            let why = dropped(tag).unwrap_or("it isn't supported");
            self.warn(
                line,
                format!("<{tag}> was removed with what's inside: {why}"),
            );
            return Ok(());
        }
        if depth >= MAX_DEPTH {
            return Err(Problem(format!(
                "{}: elements are nested more than {MAX_DEPTH} deep; flatten the page a little",
                At { line }
            )));
        }
        if let Some(why) = dropped(tag) {
            self.warn(
                line,
                format!("<{tag}> was removed with what's inside: {why}"),
            );
            return Ok(());
        }
        if tag.starts_with("slop-") {
            return self.slop(tag, &attrs, node, depth, in_link, parent, out);
        }
        let tag = if tag == "h1" {
            self.warn(
                line,
                "<h1> became <h2>: the page's one <h1> is your name, in Slop's header".into(),
            );
            "h2"
        } else {
            tag
        };
        if !TAGS.contains(&tag) {
            self.warn(
                line,
                format!("<{tag}> isn't one of the tags pages can use, so it was taken out (what was inside it stays)"),
            );
            return self.children(node, depth, in_link, parent, out);
        }
        self.count(line)?;
        let Some((tag, kept)) = self.attributes(tag, &attrs, line, in_link) else {
            return Ok(());
        };
        let mut children = Vec::new();
        if !VOID.contains(&tag) {
            self.children(node, depth + 1, in_link || tag == "a", tag, &mut children)?;
        }
        out.push(Node::Element {
            tag: tag.into(),
            attrs: kept,
            children,
        });
        Ok(())
    }

    fn count(&mut self, line: u64) -> Result<(), Problem> {
        self.elements += 1;
        if self.elements > MAX_ELEMENTS {
            return Err(Problem(format!(
                "{}: the page has more than {MAX_ELEMENTS} elements; make it simpler",
                At { line }
            )));
        }
        Ok(())
    }

    /// Removes characters that change text direction or are invisible controls: they can make
    /// text read differently from how it looks.
    fn clean_text(&mut self, text: &str, line: u64) -> String {
        let bad = |c: char| {
            matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
                || (c.is_control() && !matches!(c, '\n' | '\t' | '\r'))
        };
        if !text.chars().any(bad) {
            return text.to_string();
        }
        self.warn(
            line,
            "invisible direction or control characters were removed from the text".into(),
        );
        text.chars().filter(|c| !bad(*c)).collect()
    }

    /// The attributes an element keeps. None when the element itself goes (an `<img>` with no
    /// usable picture). Links without a usable address become `<span>`s.
    fn attributes<'t>(
        &mut self,
        tag: &'t str,
        attrs: &[Attribute],
        line: u64,
        in_link: bool,
    ) -> Option<(&'t str, BTreeMap<String, String>)> {
        let mut kept = BTreeMap::new();
        let mut tag = tag;
        for a in attrs {
            let name: &str = &a.name.local;
            let value = a.value.trim();
            let keep = match (name, tag) {
                ("class", _) => match self.classes(value, line) {
                    Some(c) => {
                        kept.insert("class".into(), c);
                        continue;
                    }
                    None => continue,
                },
                ("lang", _) => valid_lang(value),
                ("href", "a") => match link(value) {
                    Ok(href) => {
                        kept.insert("href".into(), href);
                        continue;
                    }
                    Err(why) => {
                        self.warn(
                            line,
                            format!("the link to \"{}\" was removed: {why}", short(value)),
                        );
                        continue;
                    }
                },
                ("src", "img") => match picture_path(value) {
                    Ok(path) => {
                        if !self.pictures.iter().any(|(p, _)| *p == path) {
                            self.pictures.push((path.clone(), At { line }.to_string()));
                        }
                        kept.insert("src".into(), path);
                        continue;
                    }
                    Err(why) => {
                        self.warn(
                            line,
                            format!("<img src=\"{}\"> was removed: {why}", short(value)),
                        );
                        return None;
                    }
                },
                ("alt", "img") => {
                    if value.chars().count() > MAX_ALT {
                        self.warn(
                            line,
                            format!("alt text is up to {MAX_ALT} characters: it was cut"),
                        );
                    }
                    kept.insert("alt".into(), value.chars().take(MAX_ALT).collect());
                    continue;
                }
                // Slop sets a picture's size from the file itself.
                ("width" | "height" | "loading" | "decoding", "img") => continue,
                ("start", "ol") => value.parse::<u32>().is_ok_and(|n| n < 100_000),
                ("reversed", "ol") => true,
                ("colspan" | "rowspan", "td" | "th") => {
                    value.parse::<u32>().is_ok_and(|n| (1..=50).contains(&n))
                }
                ("scope", "th") => matches!(value, "row" | "col" | "rowgroup" | "colgroup"),
                _ => {
                    let why = if name.starts_with("on") {
                        "pages can't run scripts"
                    } else if name == "style" {
                        "styles go in style.css"
                    } else if name == "id" {
                        "use a class instead"
                    } else if matches!(name, "srcset" | "sizes") {
                        "pictures have one file, from src"
                    } else {
                        "it isn't one of the attributes pages can use"
                    };
                    self.warn(line, format!("<{tag} {name}> was removed: {why}"));
                    continue;
                }
            };
            if keep {
                kept.insert(name.to_string(), value.to_string());
            } else {
                self.warn(
                    line,
                    format!(
                        "<{tag} {name}=\"{}\"> was removed: it isn't a usable value",
                        short(value)
                    ),
                );
            }
        }
        if tag == "img" {
            if !kept.contains_key("src") {
                self.warn(line, "<img> without a src was removed".into());
                return None;
            }
            if !kept.contains_key("alt") {
                self.warn(
                    line,
                    "<img> needs alt text saying what's in the picture (alt=\"\" if it's only decoration)".into(),
                );
                kept.insert("alt".into(), String::new());
            }
        }
        if tag == "a" {
            if in_link {
                tag = "span";
            } else if kept.contains_key("href") {
                kept.insert("rel".into(), LINK_REL.into());
                kept.insert("target".into(), "_blank".into());
            } else {
                tag = "span";
            }
        }
        Some((tag, kept))
    }

    fn classes(&mut self, value: &str, line: u64) -> Option<String> {
        let mut kept = Vec::new();
        for c in value.split_ascii_whitespace() {
            if !valid_class(c) {
                self.warn(
                    line,
                    format!("class \"{}\" was removed: class names are letters, digits, - and _, up to {MAX_CLASS_LEN} characters, not starting with a digit", short(c)),
                );
            } else if kept.len() == MAX_CLASSES {
                self.warn(
                    line,
                    format!("an element can have up to {MAX_CLASSES} classes: \"{c}\" was removed"),
                );
            } else if !kept.contains(&c) {
                kept.push(c);
            }
        }
        (!kept.is_empty()).then(|| kept.join(" "))
    }

    /// One of Slop's parts. Anything inside it is moved after it.
    #[allow(clippy::too_many_arguments)]
    fn slop(
        &mut self,
        tag: &str,
        attrs: &[Attribute],
        node: &Handle,
        depth: usize,
        in_link: bool,
        parent: &str,
        out: &mut Vec<Node>,
    ) -> Result<(), Problem> {
        let line = node.line;
        let at = At { line };
        let given: BTreeMap<String, String> = attrs
            .iter()
            .map(|a| (a.name.local.to_string(), a.value.trim().to_string()))
            .collect();
        let takes: &[&str] = match tag {
            "slop-work" => &["ref"],
            "slop-video" => &["youtube", "title"],
            "slop-track" => &["src", "title"],
            "slop-works" | "slop-services" => &[],
            _ => {
                self.warn(
                    line,
                    format!(
                        "<{tag}> isn't one of Slop's parts ({}), so it was taken out",
                        SLOP_TAGS.join(", ")
                    ),
                );
                return self.children(node, depth, in_link, parent, out);
            }
        };
        let mut placed = true;
        if in_link {
            self.warn(
                line,
                format!("<{tag}> can't be inside a link: it was removed"),
            );
            placed = false;
        } else if (tag == "slop-works" && self.has_works)
            || (tag == "slop-services" && self.has_services)
        {
            self.warn(line, format!("<{tag}> shows once: this one was removed"));
            placed = false;
        }
        if placed {
            for k in given.keys().filter(|k| !takes.contains(&k.as_str())) {
                self.warn(
                    line,
                    format!(
                        "<{tag} {k}> was removed: {tag} takes {}",
                        match takes {
                            [] => "no attributes".to_string(),
                            t => t.join(" and "),
                        }
                    ),
                );
            }
            let mut kept = BTreeMap::new();
            match tag {
                "slop-work" => {
                    let n = self.works.len();
                    let rule = format!(
                        "ref is the work's number in portfolio.json's works list, from 1 to {n}"
                    );
                    let Some(r) = given.get("ref") else {
                        return Err(Problem(format!(
                            "{at}: <slop-work> needs ref=\"…\": {rule}"
                        )));
                    };
                    let Some(i) = r.parse::<usize>().ok().filter(|i| (1..=n).contains(i)) else {
                        return Err(Problem(format!(
                            "{at}: <slop-work ref=\"{}\"> isn't one of your works: {rule}",
                            short(r)
                        )));
                    };
                    kept.insert("work".into(), self.works[i - 1].key());
                }
                "slop-works" => self.has_works = true,
                "slop-services" => self.has_services = true,
                "slop-video" => {
                    let Some(v) = given.get("youtube") else {
                        return Err(Problem(format!(
                            "{at}: <slop-video> needs youtube=\"…\": a YouTube link or video ID"
                        )));
                    };
                    let id = if is_youtube_id(v) {
                        Some(v.clone())
                    } else {
                        media::youtube_id(v)
                    };
                    let Some(id) = id else {
                        return Err(Problem(format!(
                            "{at}: <slop-video youtube=\"{}\"> isn't a YouTube link or video ID",
                            short(v)
                        )));
                    };
                    kept.insert("youtube".into(), id);
                }
                "slop-track" => {
                    let Some(src) = given.get("src") else {
                        return Err(Problem(format!(
                            "{at}: <slop-track> needs src=\"…\": the track's path, e.g. tracks/theme.ogg"
                        )));
                    };
                    let path = track_path(src).map_err(|why| {
                        Problem(format!("{at}: <slop-track src=\"{}\">: {why}", short(src)))
                    })?;
                    if !self.tracks.iter().any(|(p, _)| *p == path) {
                        self.tracks.push((path.clone(), at.to_string()));
                    }
                    kept.insert("src".into(), path);
                }
                _ => {}
            }
            if let Some(title) = given.get("title").filter(|_| takes.contains(&"title")) {
                if title.chars().count() > MAX_TITLE || title.chars().any(char::is_control) {
                    self.warn(line, format!("<{tag} title> was removed: titles are one line of up to {MAX_TITLE} characters"));
                } else if !title.is_empty() {
                    kept.insert("title".into(), title.clone());
                }
            }
            self.count(line)?;
            out.push(Node::Element {
                tag: tag.into(),
                attrs: kept,
                children: Vec::new(),
            });
        }
        let mut inside = Vec::new();
        self.children(node, depth, in_link, parent, &mut inside)?;
        let has_content = inside
            .iter()
            .any(|n| !matches!(n, Node::Text(t) if t.trim().is_empty()));
        if has_content {
            self.warn(
                line,
                format!("<{tag}> takes nothing inside it, so what was inside now follows it: write <{tag} …></{tag}>"),
            );
            out.extend(inside);
        }
        Ok(())
    }
}

fn push_text(out: &mut Vec<Node>, text: &str) {
    if let Some(Node::Text(t)) = out.last_mut() {
        if text == " " && t.ends_with(char::is_whitespace) {
            return;
        }
        t.push_str(text);
    } else {
        out.push(Node::Text(text.to_string()));
    }
}

fn short(s: &str) -> String {
    let s: String = s.chars().filter(|c| !c.is_control()).collect();
    if s.chars().count() > 60 {
        format!("{}…", s.chars().take(60).collect::<String>())
    } else {
        s
    }
}

fn valid_class(c: &str) -> bool {
    c.len() <= MAX_CLASS_LEN
        && c.bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_' || b == b'-')
        && c.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn valid_lang(l: &str) -> bool {
    let mut parts = l.split('-');
    parts
        .next()
        .is_some_and(|p| (2..=3).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphabetic()))
        && parts.all(|p| (1..=8).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric()))
        && l.len() <= 35
}

fn is_youtube_id(v: &str) -> bool {
    v.len() == 11
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// A link a page can have: https (no user name or password) or mailto.
fn link(href: &str) -> Result<String, &'static str> {
    if href.chars().count() > MAX_HREF {
        return Err("links are up to 500 characters");
    }
    let url = url::Url::parse(href)
        .map_err(|_| "links are full addresses starting with https:// or mailto:")?;
    match url.scheme() {
        "https"
            if url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none() =>
        {
            Ok(url.to_string())
        }
        "mailto" => Ok(url.to_string()),
        _ => Err("links start with https:// or mailto:"),
    }
}

// ---------------------------------------------------------------------------------------------
// CSS: parsed by cssparser, rebuilt from what's allowed

/// Properties a design can set. Longhands and shorthands are listed separately.
const PROPERTIES: &str = concat!(
    // Text
    "color font-family font-size font-weight font-style font-variant font-variant-caps ",
    "font-variant-numeric font-stretch font-feature-settings font-kerning line-height ",
    "letter-spacing word-spacing text-align text-align-last text-decoration ",
    "text-decoration-line text-decoration-color text-decoration-style text-decoration-thickness ",
    "text-underline-offset text-transform text-shadow text-indent text-wrap text-overflow ",
    "white-space word-break overflow-wrap hyphens vertical-align list-style list-style-type ",
    "list-style-position list-style-image quotes content counter-reset counter-increment ",
    "tab-size columns column-count column-width column-rule column-rule-color column-rule-style ",
    "column-rule-width column-span caret-color accent-color image-rendering ",
    // Backgrounds
    "background background-color background-image background-size background-position ",
    "background-position-x background-position-y background-repeat background-attachment ",
    "background-clip background-origin background-blend-mode ",
    // Boxes and spacing
    "margin margin-top margin-right margin-bottom margin-left margin-inline margin-block ",
    "margin-inline-start margin-inline-end margin-block-start margin-block-end padding ",
    "padding-top padding-right padding-bottom padding-left padding-inline padding-block ",
    "padding-inline-start padding-inline-end padding-block-start padding-block-end width height ",
    "min-width min-height max-width max-height inline-size block-size min-inline-size ",
    "max-inline-size min-block-size max-block-size box-sizing aspect-ratio overflow overflow-x ",
    "overflow-y display visibility opacity float clear object-fit object-position ",
    "mix-blend-mode filter backdrop-filter clip-path isolation ",
    // Borders and shadows
    "border border-top border-right border-bottom border-left border-width border-style ",
    "border-color border-top-width border-right-width border-bottom-width border-left-width ",
    "border-top-style border-right-style border-bottom-style border-left-style border-top-color ",
    "border-right-color border-bottom-color border-left-color border-radius ",
    "border-top-left-radius border-top-right-radius border-bottom-left-radius ",
    "border-bottom-right-radius border-collapse border-spacing border-image border-image-source ",
    "border-image-slice border-image-width border-image-outset border-image-repeat outline ",
    "outline-color outline-style outline-width outline-offset box-shadow ",
    // Layout
    "position top right bottom left inset z-index flex flex-direction flex-wrap flex-flow ",
    "flex-grow flex-shrink flex-basis order align-items align-content align-self justify-items ",
    "justify-content justify-self place-items place-content place-self gap row-gap column-gap ",
    "grid grid-template grid-template-columns grid-template-rows grid-template-areas ",
    "grid-auto-columns grid-auto-rows grid-auto-flow grid-column grid-row grid-area ",
    "grid-column-start grid-column-end grid-row-start grid-row-end table-layout caption-side ",
    "empty-cells ",
    // Movement (Slop turns it off for people who ask for reduced motion)
    "transform transform-origin transform-style translate rotate scale perspective ",
    "perspective-origin backface-visibility transition transition-property transition-duration ",
    "transition-timing-function transition-delay animation animation-name animation-duration ",
    "animation-timing-function animation-delay animation-iteration-count animation-direction ",
    "animation-fill-mode animation-play-state cursor ",
);

/// Whether a space-separated list has `word`.
fn listed(list: &str, word: &str) -> bool {
    list.split_ascii_whitespace().any(|w| w == word)
}

/// Properties that can name a picture with `url()`.
const URL_PROPERTIES: &[&str] = &[
    "background",
    "background-image",
    "border-image",
    "border-image-source",
    "list-style",
    "list-style-image",
];

/// Properties whose values can have quoted strings.
const STRING_PROPERTIES: &[&str] = &[
    "content",
    "quotes",
    "grid-template-areas",
    "grid-template",
    "grid",
    "list-style",
    "list-style-type",
];

const FUNCTIONS: &str = concat!(
    "rgb rgba hsl hsla hwb lab lch oklab oklch color color-mix linear-gradient radial-gradient ",
    "conic-gradient repeating-linear-gradient repeating-radial-gradient ",
    "repeating-conic-gradient calc min max clamp translate translatex translatey translatez ",
    "translate3d rotate rotatex rotatey rotatez rotate3d scale scalex scaley scalez scale3d ",
    "skew skewx skewy matrix matrix3d perspective cubic-bezier steps repeat minmax fit-content ",
    "blur brightness contrast drop-shadow grayscale hue-rotate invert opacity saturate sepia ",
    "counter counters inset circle ellipse polygon ",
);

const UNITS: &str = concat!(
    "px em rem ex ch lh rlh vw vh vmin vmax svw svh lvw lvh dvw dvh cqw cqh cqi cqb cqmin cqmax ",
    "pt pc cm mm in q deg grad rad turn s ms fr dpi dpcm dppx x ",
);

const DISPLAYS: &[&str] = &[
    "block",
    "inline",
    "inline-block",
    "flex",
    "inline-flex",
    "grid",
    "inline-grid",
    "flow-root",
    "none",
    "contents",
    "list-item",
    "table",
    "inline-table",
    "table-row",
    "table-cell",
    "table-caption",
    "table-header-group",
    "table-row-group",
    "table-footer-group",
    "table-column",
    "table-column-group",
];

const PSEUDO_CLASSES: &[&str] = &[
    "hover",
    "focus",
    "focus-visible",
    "focus-within",
    "active",
    "first-child",
    "last-child",
    "only-child",
    "first-of-type",
    "last-of-type",
    "only-of-type",
    "empty",
];
const PSEUDO_FUNCTIONS: &[&str] = &[
    "not",
    "is",
    "where",
    "nth-child",
    "nth-last-child",
    "nth-of-type",
    "nth-last-of-type",
];
const PSEUDO_ELEMENTS: &[&str] = &[
    "before",
    "after",
    "first-letter",
    "first-line",
    "marker",
    "selection",
];

const MEDIA_WORDS: &[&str] = &["screen", "print", "all", "and", "not", "only", "or"];

/// Why a rule or declaration was removed.
#[derive(Debug)]
pub struct Removed(String);

type CssError<'i> = ParseError<'i, Removed>;

fn removed<'i>(input: &Parser<'i, '_>, why: impl Into<String>) -> CssError<'i> {
    input.new_custom_error(Removed(why.into()))
}

pub(crate) struct CssOut {
    pub css: String,
    pub warnings: Vec<String>,
    /// Pictures `url()` names, in `.slop/`, with the line each is first on.
    pub paths: Vec<(String, u32)>,
}

struct CssState<'u> {
    warnings: Vec<String>,
    paths: Vec<(String, u32)>,
    urls: &'u HashMap<String, String>,
}

impl CssState<'_> {
    fn warn(&mut self, line: u32, what: String) {
        let w = format!("{CSS_PATH} line {}: {what}", line + 1);
        if !self.warnings.contains(&w) {
            self.warnings.push(w);
        }
    }

    /// A rule or declaration that was dropped.
    fn dropped(&mut self, error: CssError<'_>, text: &str) {
        let what = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let what = short(what.trim_end_matches(';').trim_end());
        let line = error.location.line;
        let why = match error.kind {
            ParseErrorKind::Custom(Removed(why)) => why,
            ParseErrorKind::Basic(BasicParseErrorKind::AtRuleInvalid(name)) => {
                format!("@{name} isn't allowed here")
            }
            ParseErrorKind::Basic(BasicParseErrorKind::QualifiedRuleInvalid) => {
                "rules can't be nested inside other rules".into()
            }
            ParseErrorKind::Basic(BasicParseErrorKind::EndOfInput) => "it isn't finished".into(),
            ParseErrorKind::Basic(_) => "Slop couldn't read it".into(),
        };
        if what.is_empty() {
            self.warn(line, format!("something was removed: {why}"));
        } else {
            self.warn(line, format!("\"{what}\" was removed: {why}"));
        }
    }
}

/// Rebuilds a style sheet from what's allowed. `urls` maps pictures' paths (in `.slop/`) to
/// their links; a path not in it stays a path (the first pass, before the files are fetched).
pub(crate) fn sanitize_css(src: &str, urls: &HashMap<String, String>) -> CssOut {
    let mut state = CssState {
        warnings: Vec::new(),
        paths: Vec::new(),
        urls,
    };
    let mut input = ParserInput::new(src);
    let mut parser = Parser::new(&mut input);
    let mut out = String::new();
    let mut rules = Rules {
        st: &mut state,
        nested: false,
    };
    let results: Vec<_> = StyleSheetParser::new(&mut parser, &mut rules).collect();
    for r in results {
        match r {
            Ok(rule) => out.push_str(&rule),
            Err((e, text)) => state.dropped(e, text),
        }
    }
    CssOut {
        css: out,
        warnings: state.warnings,
        paths: state.paths,
    }
}

struct Rules<'a, 'u> {
    st: &'a mut CssState<'u>,
    /// Inside @media.
    nested: bool,
}

enum AtPrelude {
    Media(String),
    Keyframes(String),
}

impl<'i> QualifiedRuleParser<'i> for Rules<'_, '_> {
    type Prelude = String;
    type QualifiedRule = String;
    type Error = Removed;

    fn parse_prelude<'t>(&mut self, input: &mut Parser<'i, 't>) -> Result<String, CssError<'i>> {
        let mut out = String::new();
        selector(input, &mut out, 0)?;
        let out = out.trim().to_string();
        if out.is_empty() {
            return Err(removed(input, "it has no selector"));
        }
        Ok(out)
    }

    fn parse_block<'t>(
        &mut self,
        prelude: String,
        _start: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<String, CssError<'i>> {
        let body = declarations(self.st, input);
        Ok(if body.is_empty() {
            String::new()
        } else {
            format!("{prelude}{{{body}}}")
        })
    }
}

impl<'i> AtRuleParser<'i> for Rules<'_, '_> {
    type Prelude = AtPrelude;
    type AtRule = String;
    type Error = Removed;

    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<AtPrelude, CssError<'i>> {
        let name = name.to_ascii_lowercase();
        match name.as_str() {
            "media" | "keyframes" if self.nested => {
                Err(removed(input, format!("@{name} can't be inside @media")))
            }
            "media" => {
                let mut out = String::new();
                media_query(input, &mut out)?;
                Ok(AtPrelude::Media(out.trim().to_string()))
            }
            "keyframes" => {
                let name = match input.next()?.clone() {
                    Token::Ident(n) | Token::QuotedString(n) if valid_class(&n) => n.to_string(),
                    _ => {
                        return Err(removed(
                            input,
                            "@keyframes names are letters, digits, - and _",
                        ));
                    }
                };
                input.expect_exhausted()?;
                Ok(AtPrelude::Keyframes(name))
            }
            "import" => Err(removed(
                input,
                "@import isn't allowed: the whole style is style.css, and Slop loads nothing else",
            )),
            "font-face" => Err(removed(
                input,
                format!(
                    "@font-face isn't allowed: use one of the fonts Slop has ({})",
                    FONTS.join(", ")
                ),
            )),
            _ => Err(removed(
                input,
                format!("@{name} isn't allowed: only @media and @keyframes"),
            )),
        }
    }

    fn parse_block<'t>(
        &mut self,
        prelude: AtPrelude,
        _start: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<String, CssError<'i>> {
        match prelude {
            AtPrelude::Media(query) => {
                let mut inner = Rules {
                    st: &mut *self.st,
                    nested: true,
                };
                let results: Vec<_> = RuleBodyParser::new(input, &mut inner).collect();
                let mut body = String::new();
                for r in results {
                    match r {
                        Ok(rule) => body.push_str(&rule),
                        Err((e, text)) => self.st.dropped(e, text),
                    }
                }
                Ok(if body.is_empty() {
                    String::new()
                } else {
                    format!("@media {query}{{{body}}}")
                })
            }
            AtPrelude::Keyframes(name) => {
                let mut frames = Frames { st: &mut *self.st };
                let results: Vec<_> = RuleBodyParser::new(input, &mut frames).collect();
                let mut body = String::new();
                for r in results {
                    match r {
                        Ok(rule) => body.push_str(&rule),
                        Err((e, text)) => self.st.dropped(e, text),
                    }
                }
                Ok(format!("@keyframes {name}{{{body}}}"))
            }
        }
    }
}

impl<'i> DeclarationParser<'i> for Rules<'_, '_> {
    type Declaration = String;
    type Error = Removed;
}

impl<'i> RuleBodyItemParser<'i, String, Removed> for Rules<'_, '_> {
    fn parse_declarations(&self) -> bool {
        false
    }

    fn parse_qualified(&self) -> bool {
        true
    }
}

/// The frames of a @keyframes rule.
struct Frames<'a, 'u> {
    st: &'a mut CssState<'u>,
}

impl<'i> QualifiedRuleParser<'i> for Frames<'_, '_> {
    type Prelude = String;
    type QualifiedRule = String;
    type Error = Removed;

    fn parse_prelude<'t>(&mut self, input: &mut Parser<'i, 't>) -> Result<String, CssError<'i>> {
        let mut out = Vec::new();
        loop {
            match input.next()?.clone() {
                Token::Ident(i) if i.eq_ignore_ascii_case("from") => out.push("from".to_string()),
                Token::Ident(i) if i.eq_ignore_ascii_case("to") => out.push("to".to_string()),
                t @ Token::Percentage { unit_value, .. } if (0.0..=1.0).contains(&unit_value) => {
                    let mut s = String::new();
                    let _ = t.to_css(&mut s);
                    out.push(s)
                }
                _ => return Err(removed(input, "keyframes are from, to or percentages")),
            }
            if input.is_exhausted() {
                break;
            }
            input.expect_comma()?;
        }
        Ok(out.join(","))
    }

    fn parse_block<'t>(
        &mut self,
        prelude: String,
        _start: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<String, CssError<'i>> {
        let body = declarations(self.st, input);
        Ok(format!("{prelude}{{{body}}}"))
    }
}

impl<'i> AtRuleParser<'i> for Frames<'_, '_> {
    type Prelude = ();
    type AtRule = String;
    type Error = Removed;
}

impl<'i> DeclarationParser<'i> for Frames<'_, '_> {
    type Declaration = String;
    type Error = Removed;
}

impl<'i> RuleBodyItemParser<'i, String, Removed> for Frames<'_, '_> {
    fn parse_declarations(&self) -> bool {
        false
    }

    fn parse_qualified(&self) -> bool {
        true
    }
}

/// A rule's declarations; each one that isn't allowed is dropped with a warning.
fn declarations(st: &mut CssState<'_>, input: &mut Parser<'_, '_>) -> String {
    let mut parser = Declarations { st: &mut *st };
    let results: Vec<_> = RuleBodyParser::new(input, &mut parser).collect();
    let mut out = String::new();
    for r in results {
        match r {
            Ok(d) => {
                out.push_str(&d);
                out.push(';');
            }
            Err((e, text)) => st.dropped(e, text),
        }
    }
    out
}

struct Declarations<'a, 'u> {
    st: &'a mut CssState<'u>,
}

impl<'i> DeclarationParser<'i> for Declarations<'_, '_> {
    type Declaration = String;
    type Error = Removed;

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _start: &ParserState,
    ) -> Result<String, CssError<'i>> {
        declaration(self.st, &name, input)
    }
}

impl<'i> AtRuleParser<'i> for Declarations<'_, '_> {
    type Prelude = ();
    type AtRule = String;
    type Error = Removed;
}

impl<'i> QualifiedRuleParser<'i> for Declarations<'_, '_> {
    type Prelude = ();
    type QualifiedRule = String;
    type Error = Removed;
}

impl<'i> RuleBodyItemParser<'i, String, Removed> for Declarations<'_, '_> {
    fn parse_declarations(&self) -> bool {
        true
    }

    fn parse_qualified(&self) -> bool {
        false
    }
}

/// What a value walk saw at its top level, for the checks of particular properties.
#[derive(Default)]
struct Seen {
    idents: Vec<String>,
    numbers: Vec<f32>,
    important: bool,
}

fn declaration<'i>(
    st: &mut CssState<'_>,
    name: &str,
    input: &mut Parser<'i, '_>,
) -> Result<String, CssError<'i>> {
    let prop = name.to_ascii_lowercase();
    if prop.starts_with("--") {
        return Err(removed(input, "custom properties (--name) aren't allowed"));
    }
    if prop.starts_with('-') {
        return Err(removed(
            input,
            "browser-specific (-webkit-, -moz-) properties aren't allowed",
        ));
    }
    if prop == "font" {
        return Err(removed(
            input,
            "use font-family, font-size, font-weight and line-height instead of the font shorthand",
        ));
    }
    if !listed(PROPERTIES, &prop) {
        return Err(removed(
            input,
            format!("{prop} isn't one of the properties pages can use"),
        ));
    }
    let line = input.current_source_location().line;
    let mut out = String::new();
    if prop == "font-family" {
        font_family(st, input, &mut out, line)?;
        return Ok(format!("font-family:{out}"));
    }
    let mut seen = Seen::default();
    let mut last = TokenSerializationType::Nothing;
    value(st, &prop, input, &mut out, &mut seen, &mut last, 0)?;
    let out = out.trim().to_string();
    if out.is_empty() {
        return Err(removed(input, "it has no value"));
    }
    if seen.important {
        st.warn(line, format!("!important was dropped from {prop}"));
    }
    let only = |allowed: &[&str]| {
        seen.idents
            .iter()
            .all(|i| allowed.contains(&i.as_str()) || is_global(i))
    };
    match prop.as_str() {
        "position" if !only(&["static", "relative", "absolute"]) => {
            return Err(removed(
                input,
                "position can be static, relative or absolute: fixed and sticky could cover Slop's page",
            ));
        }
        "z-index"
            if seen
                .numbers
                .iter()
                .any(|n| n.fract() != 0.0 || n.abs() > MAX_Z as f32) =>
        {
            return Err(removed(
                input,
                format!("z-index is a whole number from -{MAX_Z} to {MAX_Z}"),
            ));
        }
        "display" if !only(DISPLAYS) => {
            return Err(removed(input, "that display value isn't allowed"));
        }
        "content"
            if !only(&[
                "none",
                "normal",
                "open-quote",
                "close-quote",
                "no-open-quote",
                "no-close-quote",
            ]) =>
        {
            return Err(removed(input, "content can be text, counters and quotes"));
        }
        _ => {}
    }
    Ok(format!("{prop}:{out}"))
}

fn is_global(ident: &str) -> bool {
    matches!(ident, "inherit" | "initial" | "unset" | "revert")
}

/// Writes `token` after `last`, keeping tokens apart where they'd otherwise run together.
fn write_token(out: &mut String, last: &mut TokenSerializationType, token: &Token<'_>) {
    let kind = token.serialization_type();
    if last.needs_separator_when_before(kind) {
        out.push_str("/**/");
    }
    *last = kind;
    match token {
        Token::Ident(s) => {
            let _ = serialize_identifier(s, out);
        }
        Token::QuotedString(s) => {
            // No "<" anywhere in the sheet, so it can't end a <style> early wherever it lands.
            let mut quoted = String::new();
            let _ = serialize_string(s, &mut quoted);
            out.push_str(&quoted.replace('<', "\\3c "));
        }
        Token::WhiteSpace(_) => {
            if !out.ends_with(' ') {
                out.push(' ');
            }
        }
        t => {
            let _ = t.to_css(out);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn value<'i>(
    st: &mut CssState<'_>,
    prop: &str,
    input: &mut Parser<'i, '_>,
    out: &mut String,
    seen: &mut Seen,
    last: &mut TokenSerializationType,
    depth: usize,
) -> Result<(), CssError<'i>> {
    if depth > 8 {
        return Err(removed(input, "it's nested too deep"));
    }
    loop {
        let token = match input.next_including_whitespace() {
            Ok(t) => t.clone(),
            Err(_) => return Ok(()),
        };
        match &token {
            Token::WhiteSpace(_) | Token::Comma => write_token(out, last, &token),
            Token::Ident(i) => {
                let lower = i.to_ascii_lowercase();
                if ["-webkit-", "-moz-", "-ms-", "-o-"]
                    .iter()
                    .any(|p| lower.starts_with(p))
                {
                    return Err(removed(input, "browser-specific values aren't allowed"));
                }
                if depth == 0 {
                    seen.idents.push(lower);
                }
                write_token(out, last, &token);
            }
            Token::Number { value, .. } => {
                if depth == 0 {
                    seen.numbers.push(*value);
                }
                write_token(out, last, &token);
            }
            Token::Percentage { .. } => write_token(out, last, &token),
            Token::Dimension { unit, .. } => {
                if !listed(UNITS, &unit.to_ascii_lowercase()) {
                    return Err(removed(input, format!("the unit \"{unit}\" isn't allowed")));
                }
                write_token(out, last, &token);
            }
            Token::Hash(h) | Token::IDHash(h) => {
                if !(matches!(h.len(), 3 | 4 | 6 | 8) && h.bytes().all(|b| b.is_ascii_hexdigit())) {
                    return Err(removed(input, format!("#{h} isn't a colour")));
                }
                write_token(out, last, &token);
            }
            Token::QuotedString(s) => {
                if !STRING_PROPERTIES.contains(&prop) {
                    return Err(removed(input, format!("{prop} can't have quoted text")));
                }
                if s.chars().count() > 200 {
                    return Err(removed(input, "quoted text is up to 200 characters"));
                }
                write_token(out, last, &token);
            }
            Token::Delim('/' | '+' | '-' | '*') => write_token(out, last, &token),
            Token::Delim('!') => {
                input.expect_ident_matching("important")?;
                seen.important = true;
            }
            Token::UnquotedUrl(u) => {
                let u = u.to_string();
                picture_url(st, prop, input, &u, out, last)?;
            }
            Token::Function(f) => {
                let f = f.to_ascii_lowercase();
                if f == "url" {
                    let u = input.parse_nested_block(|i| {
                        let s = i.expect_string()?.to_string();
                        i.expect_exhausted()?;
                        Ok::<_, CssError<'i>>(s)
                    })?;
                    picture_url(st, prop, input, &u, out, last)?;
                    continue;
                }
                if !listed(FUNCTIONS, &f) {
                    let why = match f.as_str() {
                        "var" | "env" => "variables aren't allowed".to_string(),
                        "attr" => "attr() isn't allowed".to_string(),
                        "image-set" | "image" | "element" | "cross-fade" | "paint" => {
                            "pictures come from url() in background properties".to_string()
                        }
                        _ => format!("{f}() isn't allowed"),
                    };
                    return Err(removed(input, why));
                }
                write_token(out, last, &token);
                input.parse_nested_block(|i| value(st, prop, i, out, seen, last, depth + 1))?;
                out.push(')');
                *last = TokenSerializationType::Other;
            }
            Token::ParenthesisBlock => {
                write_token(out, last, &token);
                input.parse_nested_block(|i| value(st, prop, i, out, seen, last, depth + 1))?;
                out.push(')');
                *last = TokenSerializationType::Other;
            }
            Token::SquareBracketBlock if prop.starts_with("grid") => {
                write_token(out, last, &token);
                input.parse_nested_block(|i| {
                    loop {
                        match i.next_including_whitespace() {
                            Ok(Token::Ident(n)) if valid_class(n) => {
                                let n = n.clone();
                                write_token(out, last, &Token::Ident(n));
                            }
                            Ok(t @ Token::WhiteSpace(_)) => {
                                let t = t.clone();
                                write_token(out, last, &t);
                            }
                            Ok(_) => return Err(removed(i, "grid line names are plain names")),
                            Err(_) => return Ok(()),
                        }
                    }
                })?;
                out.push(']');
                *last = TokenSerializationType::Other;
            }
            _ => {
                return Err(removed(
                    input,
                    "it has something Slop doesn't allow in a value",
                ));
            }
        }
    }
}

/// `url(…)`: a picture from the repository, only in background, border-image and list-style.
fn picture_url<'i>(
    st: &mut CssState<'_>,
    prop: &str,
    input: &Parser<'i, '_>,
    raw: &str,
    out: &mut String,
    last: &mut TokenSerializationType,
) -> Result<(), CssError<'i>> {
    if !URL_PROPERTIES.contains(&prop) {
        return Err(removed(
            input,
            format!(
                "{prop} can't name a file: url() is only for background, border-image and list-style pictures"
            ),
        ));
    }
    let path = picture_path(raw)
        .map_err(|why| removed(input, format!("url(\"{}\"): {why}", short(raw))))?;
    let line = input.current_source_location().line + 1;
    if !st.paths.iter().any(|(p, _)| *p == path) {
        st.paths.push((path.clone(), line));
    }
    let link = st.urls.get(&path).cloned().unwrap_or(path);
    if last.needs_separator_when_before(TokenSerializationType::Other) {
        out.push_str("/**/");
    }
    out.push_str("url(");
    let _ = serialize_string(&link, out);
    out.push(')');
    *last = TokenSerializationType::Other;
    Ok(())
}

/// `font-family`: Slop's fonts and the generic families. Others are dropped with a warning.
fn font_family<'i>(
    st: &mut CssState<'_>,
    input: &mut Parser<'i, '_>,
    out: &mut String,
    line: u32,
) -> Result<(), CssError<'i>> {
    let names = input.parse_comma_separated(|i| {
        if let Ok(s) = i.try_parse(|i| i.expect_string_cloned()) {
            return Ok(s.to_string());
        }
        let mut words = vec![i.expect_ident()?.to_string()];
        while let Ok(w) = i.try_parse(|i| i.expect_ident_cloned()) {
            words.push(w.to_string());
        }
        Ok::<_, CssError<'i>>(words.join(" "))
    })?;
    let mut kept = Vec::new();
    for name in names {
        if let Some(f) = FONTS.iter().find(|f| f.eq_ignore_ascii_case(&name)) {
            let mut s = String::new();
            let _ = serialize_string(f, &mut s);
            kept.push(s);
        } else if let Some(g) = GENERIC_FONTS.iter().find(|g| g.eq_ignore_ascii_case(&name)) {
            kept.push(g.to_string());
        } else {
            st.warn(
                line,
                format!(
                    "the font \"{}\" was removed: pages can use {}, or serif, sans-serif, monospace, cursive and system-ui",
                    short(&name),
                    FONTS.join(", ")
                ),
            );
        }
    }
    if kept.is_empty() {
        return Err(removed(input, "none of its fonts are ones Slop has"));
    }
    out.push_str(&kept.join(","));
    Ok(())
}

/// A selector list, checked token by token: tags, classes, combinators, a few pseudo-classes and
/// pseudo-elements. No ids (pages have none), no attribute selectors, nothing outside the page.
fn selector<'i>(
    input: &mut Parser<'i, '_>,
    out: &mut String,
    depth: usize,
) -> Result<(), CssError<'i>> {
    if depth > 4 {
        return Err(removed(input, "the selector is nested too deep"));
    }
    let mut last = TokenSerializationType::Nothing;
    loop {
        let token = match input.next_including_whitespace() {
            Ok(t) => t.clone(),
            Err(_) => break,
        };
        match &token {
            Token::WhiteSpace(_) | Token::Comma => write_token(out, &mut last, &token),
            Token::Delim('>' | '+' | '~' | '*') => write_token(out, &mut last, &token),
            Token::Ident(tag) => {
                let tag = tag.to_ascii_lowercase();
                if !TAGS.contains(&tag.as_str()) {
                    return Err(removed(
                        input,
                        if matches!(tag.as_str(), "html" | "body" | ":root") {
                            format!(
                                "{tag} isn't part of your page: your page's outermost box is .page"
                            )
                        } else if tag.starts_with("slop-") {
                            "Slop's parts keep Slop's own look".to_string()
                        } else {
                            format!("<{tag}> isn't one of the tags pages can use")
                        },
                    ));
                }
                write_token(out, &mut last, &Token::Ident(tag.into()));
            }
            Token::Delim('.') => {
                write_token(out, &mut last, &token);
                match input.next_including_whitespace()?.clone() {
                    Token::Ident(c) if valid_class(&c) => {
                        write_token(out, &mut last, &Token::Ident(c))
                    }
                    _ => return Err(removed(input, "class names are letters, digits, - and _")),
                }
            }
            Token::Colon => {
                write_token(out, &mut last, &token);
                match input.next_including_whitespace()?.clone() {
                    Token::Ident(p) => {
                        let p = p.to_ascii_lowercase();
                        if PSEUDO_ELEMENTS.contains(&p.as_str())
                            && matches!(
                                p.as_str(),
                                "before" | "after" | "first-letter" | "first-line"
                            )
                        {
                            // The old one-colon form of ::before and the like.
                            out.push(':');
                        } else if !PSEUDO_CLASSES.contains(&p.as_str()) {
                            return Err(removed(input, format!(":{p} isn't allowed")));
                        }
                        write_token(out, &mut last, &Token::Ident(p.into()));
                    }
                    Token::Colon => {
                        write_token(out, &mut last, &Token::Colon);
                        let p = input.expect_ident()?.to_ascii_lowercase();
                        if !PSEUDO_ELEMENTS.contains(&p.as_str()) {
                            return Err(removed(input, format!("::{p} isn't allowed")));
                        }
                        write_token(out, &mut last, &Token::Ident(p.into()));
                    }
                    Token::Function(f) => {
                        let f = f.to_ascii_lowercase();
                        if !PSEUDO_FUNCTIONS.contains(&f.as_str()) {
                            return Err(removed(input, format!(":{f}() isn't allowed")));
                        }
                        write_token(out, &mut last, &Token::Function(f.clone().into()));
                        if f.starts_with("nth-") {
                            input.parse_nested_block(|i| nth(i, out))?;
                        } else {
                            input.parse_nested_block(|i| selector(i, out, depth + 1))?;
                        }
                        out.push(')');
                        last = TokenSerializationType::Other;
                    }
                    _ => return Err(removed(input, "Slop couldn't read the selector")),
                }
            }
            Token::Hash(_) | Token::IDHash(_) => {
                return Err(removed(input, "ids aren't allowed: use a class"));
            }
            Token::SquareBracketBlock => {
                return Err(removed(
                    input,
                    "attribute selectors aren't allowed: use a class",
                ));
            }
            Token::Delim('&') => return Err(removed(input, "nested rules aren't allowed")),
            _ => return Err(removed(input, "Slop couldn't read the selector")),
        }
    }
    Ok(())
}

/// The inside of :nth-child() and the like: odd, even, or An+B.
fn nth<'i>(input: &mut Parser<'i, '_>, out: &mut String) -> Result<(), CssError<'i>> {
    let (a, b) = cssparser::parse_nth(input)?;
    input.expect_exhausted()?;
    out.push_str(&format!("{a}n{}{b}", if b < 0 { "" } else { "+" }));
    Ok(())
}

fn media_query<'i>(input: &mut Parser<'i, '_>, out: &mut String) -> Result<(), CssError<'i>> {
    let mut last = TokenSerializationType::Nothing;
    loop {
        let token = match input.next_including_whitespace() {
            Ok(t) => t.clone(),
            Err(_) => return Ok(()),
        };
        match &token {
            Token::WhiteSpace(_) | Token::Comma => write_token(out, &mut last, &token),
            Token::Ident(w) if MEDIA_WORDS.contains(&w.to_ascii_lowercase().as_str()) => {
                write_token(out, &mut last, &token)
            }
            Token::ParenthesisBlock => {
                write_token(out, &mut last, &token);
                input.parse_nested_block(|i| media_feature(i, out, 0))?;
                out.push(')');
                last = TokenSerializationType::Other;
            }
            _ => return Err(removed(input, "Slop couldn't read the media query")),
        }
    }
}

fn media_feature<'i>(
    input: &mut Parser<'i, '_>,
    out: &mut String,
    depth: usize,
) -> Result<(), CssError<'i>> {
    if depth > 3 {
        return Err(removed(input, "the media query is nested too deep"));
    }
    let mut last = TokenSerializationType::Nothing;
    loop {
        let token = match input.next_including_whitespace() {
            Ok(t) => t.clone(),
            Err(_) => return Ok(()),
        };
        match &token {
            Token::WhiteSpace(_)
            | Token::Colon
            | Token::Ident(_)
            | Token::Number { .. }
            | Token::Delim('>' | '=' | '/') => write_token(out, &mut last, &token),
            Token::Dimension { unit, .. } if listed(UNITS, &unit.to_ascii_lowercase()) => {
                write_token(out, &mut last, &token)
            }
            Token::ParenthesisBlock => {
                write_token(out, &mut last, &token);
                input.parse_nested_block(|i| media_feature(i, out, depth + 1))?;
                out.push(')');
                last = TokenSerializationType::Other;
            }
            _ => return Err(removed(input, "Slop couldn't read the media query")),
        }
    }
}

#[cfg(test)]
mod tests;
