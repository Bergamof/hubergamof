//! Static files of the web UI (`SvelteKit` `adapter-static` output).
//!
//! - `/` serves `index.html`; existing files are served with their MIME type;
//! - `_app/immutable/**` (content-hashed names) is cached for a year, the rest
//!   is revalidated every time (`no-cache`);
//! - any other `GET` that does not look like a file gets `200.html`, the SPA
//!   shell that routes on the client;
//! - HTML pages carry a CSP allowing exactly their inline scripts (the
//!   `SvelteKit` bootstrap) by hash;
//! - a file precompressed by the build (`.br`, `.gz` next to it) is served
//!   compressed to a client accepting that encoding;
//! - without a UI build, a plain text page explains how to make one.

use std::borrow::Cow;

use axum::body::Body;
use axum::extract::State;
use axum::http::header::{
    ACCEPT_ENCODING, CACHE_CONTROL, CONTENT_ENCODING, CONTENT_SECURITY_POLICY, CONTENT_TYPE, VARY,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use rust_embed::RustEmbed;
use sha2::{Digest as _, Sha256};

use super::AppState;
use super::security::CONTENT_SECURITY_POLICY_BASE;

/// File served for `/`.
pub const INDEX_FILE: &str = "index.html";

/// SPA shell served for client-side routes.
pub const FALLBACK_FILE: &str = "200.html";

/// Directory of the content-hashed build outputs.
const IMMUTABLE_PREFIX: &str = "_app/immutable/";

/// Directory of the `SvelteKit` runtime files.
const APP_PREFIX: &str = "_app/";

/// Encodings of the precompressed files, preferred first: (`Content-Encoding`,
/// file extension).
const PRECOMPRESSED: [(&str, &str); 2] = [("br", "br"), ("gzip", "gz")];

const CACHE_IMMUTABLE: &str = "public, max-age=31536000, immutable";
const CACHE_REVALIDATE: &str = "no-cache";

/// Shown instead of the UI when the binary was built without it.
pub const MISSING_UI_MESSAGE: &str = "L'interface web de HuBergamof n'est pas construite.\n\
     Lancez `make ui-build` puis recompilez le gestionnaire.\n";

/// Where the UI files come from.
pub trait AssetSource: Send + Sync + 'static {
    /// Content of the file at `path` (relative, `/`-separated, e.g.
    /// `_app/immutable/entry/start.js`).
    fn get(&self, path: &str) -> Option<Cow<'static, [u8]>>;
}

/// UI build embedded in the binary.
///
/// `allow_missing`: the crate builds without `ui/build/` (Rust-only CI jobs),
/// the server then explains how to build the UI. Debug builds read the files
/// from disk at run time, release builds embed them (`build.rs` makes Cargo
/// rebuild when `ui/build/` changes).
#[derive(RustEmbed)]
#[folder = "../../ui/build/"]
#[allow_missing = true]
pub struct EmbeddedUi;

impl AssetSource for EmbeddedUi {
    fn get(&self, path: &str) -> Option<Cow<'static, [u8]>> {
        <Self as RustEmbed>::get(path).map(|file| file.data)
    }
}

/// Handler of every `GET`/`HEAD` that is not another route.
pub(super) async fn serve_ui(
    State(state): State<AppState>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let assets = state.assets.as_ref();
    let Some(path) = asset_path(uri.path()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let accepted = headers
        .get(ACCEPT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if let Some(content) = assets.get(&path) {
        return file_response(assets, &path, content, accepted);
    }
    if looks_like_file(uri.path().trim_start_matches('/')) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match assets.get(FALLBACK_FILE) {
        Some(content) => file_response(assets, FALLBACK_FILE, content, accepted),
        None => missing_ui_response(),
    }
}

/// The precompressed variant of `path` the client accepts, with its
/// `Content-Encoding`.
fn precompressed(
    assets: &dyn AssetSource,
    path: &str,
    accepted: &str,
) -> Option<(&'static str, Cow<'static, [u8]>)> {
    PRECOMPRESSED
        .iter()
        .filter(|(encoding, _)| accepts_encoding(accepted, encoding))
        .find_map(|(encoding, extension)| {
            assets
                .get(&format!("{path}.{extension}"))
                .map(|content| (*encoding, content))
        })
}

/// Whether the `Accept-Encoding` value `accepted` allows `encoding` with a
/// non-zero quality: named, else through `*` (a name overrides `*`, RFC 9110
/// §12.5.3).
fn accepts_encoding(accepted: &str, encoding: &str) -> bool {
    let codings: Vec<(&str, bool)> = accepted
        .split(',')
        .map(|item| {
            let mut parts = item.split(';').map(str::trim);
            let name = parts.next().unwrap_or_default();
            let refused = parts.any(|parameter| {
                parameter
                    .strip_prefix("q=")
                    .and_then(|quality| quality.parse::<f32>().ok())
                    .is_some_and(|quality| quality <= 0.0)
            });
            (name, !refused)
        })
        .collect();
    let allowed = |matches: &dyn Fn(&str) -> bool| {
        codings
            .iter()
            .find(|(name, _)| matches(name))
            .map(|&(_, allowed)| allowed)
    };
    allowed(&|name| name.eq_ignore_ascii_case(encoding))
        .or_else(|| allowed(&|name| name == "*"))
        .unwrap_or(false)
}

/// Relative path of the requested file, `None` when the path tries to leave
/// the UI directory.
fn asset_path(request_path: &str) -> Option<String> {
    let path = request_path.trim_start_matches('/');
    if path
        .split('/')
        .any(|segment| segment == ".." || segment.contains('\\'))
    {
        return None;
    }
    if path.is_empty() || path.ends_with('/') {
        Some(format!("{path}{INDEX_FILE}"))
    } else {
        Some(path.to_owned())
    }
}

/// Request paths (without the leading `/`) that name a build output rather
/// than a client-side route.
fn looks_like_file(path: &str) -> bool {
    let file_name = path.rsplit('/').next().unwrap_or(path);
    path.starts_with(APP_PREFIX) || file_name.contains('.')
}

fn file_response(
    assets: &dyn AssetSource,
    path: &str,
    content: Cow<'static, [u8]>,
    accepted: &str,
) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let content_type = if needs_charset(&mime) {
        format!("{mime}; charset=utf-8")
    } else {
        mime.to_string()
    };
    let cache = if path.starts_with(IMMUTABLE_PREFIX) {
        CACHE_IMMUTABLE
    } else {
        CACHE_REVALIDATE
    };
    let mut response = Response::new(Body::empty());
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&content_type) {
        headers.insert(CONTENT_TYPE, value);
    }
    headers.insert(CACHE_CONTROL, HeaderValue::from_static(cache));
    // Caches keep each encoding apart.
    headers.insert(VARY, HeaderValue::from_static("accept-encoding"));
    if mime == mime_guess::mime::TEXT_HTML
        && let Ok(policy) = HeaderValue::from_str(&html_content_security_policy(&content))
    {
        headers.insert(CONTENT_SECURITY_POLICY, policy);
    }
    let body = match precompressed(assets, path, accepted) {
        Some((encoding, compressed)) => {
            headers.insert(CONTENT_ENCODING, HeaderValue::from_static(encoding));
            compressed
        }
        None => content,
    };
    *response.body_mut() = Body::from(body);
    response
}

fn needs_charset(mime: &mime_guess::Mime) -> bool {
    use mime_guess::mime;
    mime.get_param(mime::CHARSET).is_none()
        && (mime.type_() == mime::TEXT
            || mime.subtype() == mime::JAVASCRIPT
            || mime.subtype() == mime::JSON
            || mime.suffix() == Some(mime::XML))
}

fn missing_ui_response() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(CACHE_CONTROL, CACHE_REVALIDATE)],
        MISSING_UI_MESSAGE,
    )
        .into_response()
}

/// [`CONTENT_SECURITY_POLICY_BASE`] with a `script-src` allowing the inline
/// scripts of `html` by hash, so that no `'unsafe-inline'` is needed.
#[must_use]
pub fn html_content_security_policy(html: &[u8]) -> String {
    let mut policy = format!("{CONTENT_SECURITY_POLICY_BASE}; script-src 'self'");
    for script in inline_scripts(&String::from_utf8_lossy(html)) {
        policy.push_str(" 'sha256-");
        policy.push_str(&BASE64.encode(Sha256::digest(script)));
        policy.push('\'');
    }
    policy
}

/// Bodies of the `<script>` elements without `src`, verbatim (the hash covers
/// the exact text between the tags).
fn inline_scripts(html: &str) -> impl Iterator<Item = &str> {
    const OPEN: &str = "<script";
    const CLOSE: &str = "</script>";
    let mut rest = html;
    std::iter::from_fn(move || {
        loop {
            let start = rest.find(OPEN)?;
            let after_open = &rest[start + OPEN.len()..];
            let tag_end = after_open.find('>')?;
            let attributes = &after_open[..tag_end];
            let is_script = attributes
                .chars()
                .next()
                .is_none_or(|next| next.is_ascii_whitespace() || next == '/');
            if !is_script {
                // Another element whose name starts with `script`.
                rest = after_open;
                continue;
            }
            let body_and_rest = &after_open[tag_end + 1..];
            let body_end = body_and_rest.find(CLOSE)?;
            let body = &body_and_rest[..body_end];
            rest = &body_and_rest[body_end + CLOSE.len()..];
            if !attributes.contains("src=") {
                return Some(body);
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_paths_map_to_files() {
        assert_eq!(asset_path("/").as_deref(), Some(INDEX_FILE));
        assert_eq!(asset_path("/docs/").as_deref(), Some("docs/index.html"));
        assert_eq!(
            asset_path("/_app/immutable/a.js").as_deref(),
            Some("_app/immutable/a.js")
        );
        assert_eq!(asset_path("/devices/ABC").as_deref(), Some("devices/ABC"));
    }

    #[test]
    fn escaping_paths_are_refused() {
        assert_eq!(asset_path("/../Cargo.toml"), None);
        assert_eq!(asset_path("/_app/../../secret"), None);
        assert_eq!(asset_path("/a\\..\\b"), None);
    }

    #[test]
    fn files_are_told_apart_from_routes() {
        assert!(looks_like_file("robots.txt"));
        assert!(looks_like_file("_app/version"));
        assert!(looks_like_file("images/key.png"));
        assert!(!looks_like_file("devices/ABC"));
        assert!(!looks_like_file("settings"));
        assert!(!looks_like_file(""));
        assert!(!looks_like_file("docs/"));
    }

    #[test]
    fn inline_scripts_are_extracted_verbatim() {
        let html = "<head><script type=\"module\" src=\"/a.js\"></script>\
                    <script>\n  start();\n</script><scripts></scripts>\
                    <script type=\"module\">go()</script></head>";
        let scripts: Vec<_> = inline_scripts(html).collect();
        assert_eq!(scripts, ["\n  start();\n", "go()"]);
    }

    #[test]
    fn html_policy_allows_inline_scripts_by_hash() {
        let policy = html_content_security_policy(b"<script>alert(1)</script>");
        // echo -n 'alert(1)' | openssl dgst -sha256 -binary | base64
        assert!(policy.starts_with(CONTENT_SECURITY_POLICY_BASE));
        assert!(policy.ends_with(
            "; script-src 'self' 'sha256-bhHHL3z2vDgxUt0W3dWQOrprscmda2Y5pLsLg4GF+pI='"
        ));
        assert!(!policy.contains("unsafe-eval"));
        assert!(html_content_security_policy(b"<p>no script</p>").ends_with("script-src 'self'"));
    }

    /// Files of a build, by path.
    struct Files(Vec<(&'static str, &'static [u8])>);

    impl AssetSource for Files {
        fn get(&self, path: &str) -> Option<Cow<'static, [u8]>> {
            self.0
                .iter()
                .find(|(name, _)| *name == path)
                .map(|(_, content)| Cow::Borrowed(*content))
        }
    }

    #[test]
    fn accepted_encodings_are_parsed() {
        assert!(accepts_encoding("gzip, deflate, br, zstd", "br"));
        assert!(accepts_encoding("GZIP;q=0.5", "gzip"));
        assert!(accepts_encoding("*", "br"));
        assert!(!accepts_encoding("br;q=0, gzip", "br"));
        assert!(!accepts_encoding("br;q=0.0", "br"));
        assert!(!accepts_encoding("br;q=0, *", "br"), "named beats *");
        assert!(accepts_encoding("*;q=0, gzip", "gzip"));
        assert!(!accepts_encoding("*;q=0", "gzip"));
        assert!(!accepts_encoding("brotli", "br"));
        assert!(!accepts_encoding("", "gzip"));
    }

    #[test]
    fn precompressed_files_are_preferred_when_accepted() {
        let files = Files(vec![
            ("a.js", b"plain"),
            ("a.js.br", b"brotli"),
            ("a.js.gz", b"gzip"),
            ("b.js", b"plain"),
            ("b.js.gz", b"gzip"),
        ]);
        let pick = |path, accepted| precompressed(&files, path, accepted);
        assert_eq!(
            pick("a.js", "gzip, br"),
            Some(("br", Cow::Borrowed(&b"brotli"[..])))
        );
        assert_eq!(
            pick("a.js", "gzip"),
            Some(("gzip", Cow::Borrowed(&b"gzip"[..])))
        );
        assert_eq!(
            pick("b.js", "br, gzip"),
            Some(("gzip", Cow::Borrowed(&b"gzip"[..])))
        );
        assert_eq!(pick("b.js", "br"), None);
        assert_eq!(pick("a.js", ""), None);
    }

    #[test]
    fn a_compressed_page_keeps_the_policy_of_its_scripts() {
        let page: &[u8] = b"<script>go()</script>";
        let files = Files(vec![(INDEX_FILE, page), ("index.html.gz", b"compressed")]);
        let response = file_response(&files, INDEX_FILE, Cow::Borrowed(page), "gzip");
        let headers = response.headers();
        assert_eq!(headers[CONTENT_ENCODING], "gzip");
        assert_eq!(headers[VARY], "accept-encoding");
        assert_eq!(
            headers[CONTENT_SECURITY_POLICY].to_str().unwrap(),
            html_content_security_policy(page)
        );
        assert!(
            headers[CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("text/html")
        );
    }

    #[test]
    fn text_files_declare_utf8() {
        let mime = |path| mime_guess::from_path(path).first_or_octet_stream();
        assert!(needs_charset(&mime("a.html")));
        assert!(needs_charset(&mime("a.js")));
        assert!(needs_charset(&mime("a.css")));
        assert!(needs_charset(&mime("a.svg")));
        assert!(!needs_charset(&mime("a.png")));
        assert!(!needs_charset(&mime("a.woff2")));
    }
}
