//! Opt-in outbound-request log, for answering "did this provider reach the
//! network, or was the figure a cache hit?".
//!
//! Set `AI_USAGEBAR_LOG_REQUESTS=1` and every request built through [`send`]
//! appends to `ai-usagebar-requests.log` in the system temp directory:
//!
//! ```text
//! 2026-09-22T02:55:01.130Z run pid=3312 version=1.20.2
//! 2026-09-22T02:55:01.138Z request vendor=zai method=GET target=https://api.z.ai/api/monitor/usage/quota/limit
//! 2026-09-22T02:55:01.245Z response vendor=zai status=200
//! ```
//!
//! The `run` header carries the pid, so two runs appended to one file stay
//! tellable apart — a before/after comparison is a diff of two sections rather
//! than of two files. Each binary opens its run through [`begin_run`] before it
//! does any work, so a run that lists no request is proof it made none, not a
//! run that quietly never happened.
//!
//! **Fixed vendor lines carry scheme, host, port and path — never the query
//! string.** A custom provider's URL is wholly user-supplied, so its log line
//! carries only the origin: either the path or query can itself be a token.
//! Date ranges and pagination cursors would also add nothing to a line whose
//! job is naming a fixed endpoint. A logged fixed path is untrusted text on its
//! way to a file, so it goes through [`sanitize_untrusted_line`]: a segment can
//! carry an account label or an escape sequence, and one embedded newline
//! forges a log line.
//!
//! Transport errors are *classified*, never rendered: `reqwest`'s `Display`
//! embeds the request URL — query string included — so an error line names the
//! kind (`timeout`, `connect`, …) instead of the error.
//!
//! Disabled, the whole thing costs one cached environment lookup and no
//! allocation. Tests drive [`send_with`] against an explicit [`Sink`] so none
//! of them reads the ambient environment or writes the real temp directory.

use std::ffi::OsStr;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use reqwest::RequestBuilder;
use reqwest::Response;

use crate::display::sanitize_untrusted_line;

/// The exact value `1` turns the log on. Every other value is off.
const ENV_VAR: &str = "AI_USAGEBAR_LOG_REQUESTS";

const FILE_NAME: &str = "ai-usagebar-requests.log";

#[derive(Clone, Copy)]
enum TargetDetail {
    Endpoint,
    Origin,
}

fn enabled_value(value: Option<&OsStr>) -> bool {
    value == Some(OsStr::new("1"))
}

/// Where log lines go — and, as [`Sink::disabled`], the no-op every code path
/// sees when the log is off.
#[derive(Debug)]
pub struct Sink {
    path: Option<PathBuf>,
    headered: AtomicBool,
    write_lock: Mutex<()>,
}

impl Sink {
    /// Writes nowhere. The production default when the env var is unset.
    pub fn disabled() -> Self {
        Self {
            path: None,
            headered: AtomicBool::new(false),
            write_lock: Mutex::new(()),
        }
    }

    /// Appends to `path`, creating it if needed.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self {
            path: Some(path.into()),
            headered: AtomicBool::new(false),
            write_lock: Mutex::new(()),
        }
    }

    /// The production resolver: only `AI_USAGEBAR_LOG_REQUESTS=1` picks the
    /// file in the system temp directory, next to the tray's own trace log.
    pub fn from_env() -> Self {
        let value = std::env::var_os(ENV_VAR);
        if enabled_value(value.as_deref()) {
            Self::at(std::env::temp_dir().join(FILE_NAME))
        } else {
            Self::disabled()
        }
    }

    /// Whether anything would be written. Callers check this before doing work
    /// that only a log line needs, such as cloning a request.
    pub fn is_enabled(&self) -> bool {
        self.path.is_some()
    }

    /// Announces the run before any request is made.
    ///
    /// Without this, a run that reaches no endpoint would leave nothing behind,
    /// and "the log lists no request for this provider" would be
    /// indistinguishable from "this run never happened". With the header
    /// written up front, a run that lists no request is proof it made none.
    pub fn begin_run(&self) {
        self.header();
    }

    /// One header per sink, so everything below a `run` line belongs to that
    /// pid — the whole point of appending across runs.
    fn header(&self) {
        let Some(path) = self.path.as_deref() else {
            return;
        };
        let Ok(_guard) = self.write_lock.lock() else {
            return;
        };
        let _ = self.write_header(path);
    }

    fn write_header(&self, path: &Path) -> bool {
        if self.headered.load(Ordering::Acquire) {
            return true;
        }
        if !write_line(
            path,
            &format!(
                "{} run pid={} version={}",
                timestamp(),
                std::process::id(),
                env!("CARGO_PKG_VERSION"),
            ),
        ) {
            return false;
        }
        self.headered.store(true, Ordering::Release);
        true
    }

    fn line(&self, event: &str, fields: &[(&str, String)]) {
        let Some(path) = self.path.as_deref() else {
            return;
        };
        let mut line = format!("{} {event}", timestamp());
        for (key, value) in fields {
            line.push(' ');
            line.push_str(key);
            line.push('=');
            line.push_str(value);
        }
        let Ok(_guard) = self.write_lock.lock() else {
            return;
        };
        if self.write_header(path) {
            let _ = write_line(path, &line);
        }
    }
}

/// Opens this run in the log, if the log is on. Each binary calls it once,
/// before doing any work.
pub fn begin_run() {
    sink().begin_run();
}

/// Sends a fixed vendor request, recording its endpoint and outcome.
///
/// Every outbound request in the crate goes through this module — a guard test
/// walks the tree and fails on a bare `.send()` outside it. That is not
/// ceremony: a vendor that opens its own connection is invisible to the log,
/// and the log is the only way to tell a live read from a cache hit.
pub async fn send(vendor: &str, request: RequestBuilder) -> Result<Response, reqwest::Error> {
    send_with_detail(sink(), vendor, request, TargetDetail::Endpoint).await
}

/// Sends a custom-provider request without exposing its user-supplied path.
pub async fn send_custom(
    vendor: &str,
    request: RequestBuilder,
) -> Result<Response, reqwest::Error> {
    send_with_detail(sink(), vendor, request, TargetDetail::Origin).await
}

/// [`send`] against an explicit sink, so a test never touches the ambient
/// environment or the real temp directory.
pub async fn send_with(
    sink: &Sink,
    vendor: &str,
    request: RequestBuilder,
) -> Result<Response, reqwest::Error> {
    send_with_detail(sink, vendor, request, TargetDetail::Endpoint).await
}

async fn send_with_detail(
    sink: &Sink,
    vendor: &str,
    request: RequestBuilder,
    detail: TargetDetail,
) -> Result<Response, reqwest::Error> {
    // The description is derived from the built request rather than passed in
    // by the caller, so the line cannot drift from what is actually sent. It
    // costs a clone of the builder, which is why it is behind the switch.
    let described = if sink.is_enabled() {
        request
            .try_clone()
            .and_then(|clone| clone.build().ok())
            .map(|built| (built.method().as_str().to_string(), built.url().to_string()))
    } else {
        None
    };

    if let Some((method, url)) = described {
        sink.line(
            "request",
            &[
                ("vendor", vendor.to_string()),
                ("method", method),
                ("target", target_with_detail(&url, detail)),
            ],
        );
    }

    let result = request.send().await;

    if sink.is_enabled() {
        let outcome = match &result {
            Ok(response) => ("status", response.status().as_u16().to_string()),
            Err(error) => ("error", error_kind(error).to_string()),
        };
        sink.line("response", &[("vendor", vendor.to_string()), outcome]);
    }

    result
}

/// Scheme, host, port and path of a fixed vendor URL.
///
/// A custom provider uses [`send_custom`], which records only its origin: the
/// configured path is user-supplied and can itself be a credential.
pub fn target(url: &str) -> String {
    target_with_detail(url, TargetDetail::Endpoint)
}

fn target_with_detail(url: &str, detail: TargetDetail) -> String {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return "<unparseable-url>".to_string();
    };
    let host = parsed.host_str().unwrap_or("<no-host>");
    let port = parsed
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    let origin = format!("{}://{host}{port}", parsed.scheme());
    match detail {
        TargetDetail::Endpoint => format!("{origin}{}", sanitize_untrusted_line(parsed.path())),
        TargetDetail::Origin => origin,
    }
}

/// The category of a transport failure.
///
/// `reqwest::Error`'s `Display` embeds the request URL — query string
/// included — so the log never renders one.
fn error_kind(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connect"
    } else if error.is_body() {
        "body"
    } else if error.is_decode() {
        "decode"
    } else {
        "other"
    }
}

fn timestamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn write_line(path: &Path, line: &str) -> bool {
    use std::io::Write;
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return false;
    };
    writeln!(file, "{line}").is_ok()
}

/// The process-wide sink: the env var decides, once.
fn sink() -> &'static Sink {
    static SINK: OnceLock<Sink> = OnceLock::new();
    SINK.get_or_init(Sink::from_env)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).expect("the sink wrote its file")
    }

    #[test]
    fn only_one_enables_the_log() {
        assert!(enabled_value(Some(OsStr::new("1"))));
        for value in [
            None,
            Some(OsStr::new("")),
            Some(OsStr::new("0")),
            Some(OsStr::new("true")),
        ] {
            assert!(!enabled_value(value), "{value:?}");
        }
    }

    #[test]
    fn the_target_drops_the_query_string() {
        assert_eq!(
            target("https://api.example.test/usage?token=super-secret&page=2"),
            "https://api.example.test/usage"
        );
        let logged = target("https://api.example.test/v1/usage?api_key=abc123");
        assert!(!logged.contains("abc123"), "{logged}");
    }

    #[test]
    fn a_custom_target_drops_its_path_query_and_userinfo() {
        let logged = target_with_detail(
            "https://user:pass@api.example.test:8443/bot/super-secret/usage?token=also-secret",
            TargetDetail::Origin,
        );
        assert_eq!(logged, "https://api.example.test:8443");
        assert!(!logged.contains("secret"), "{logged}");
    }

    #[test]
    fn the_target_drops_userinfo_and_keeps_a_non_default_port() {
        assert_eq!(
            target("https://user:pass@example.test:8443/usage?token=x"),
            "https://example.test:8443/usage"
        );
        // A default port is normalized away rather than rendered.
        assert_eq!(
            target("https://example.test:443/usage"),
            "https://example.test/usage"
        );
    }

    #[test]
    fn the_target_cannot_forge_a_line_and_survives_junk() {
        let forged = target("https://example.test/a%0Arequest%20vendor%3Dfake");
        assert!(!forged.contains('\n'), "{forged:?}");
        // A raw newline in the input never reaches the output either: the
        // parser percent-encodes or strips it, and the sink sanitizes again.
        assert!(!target("https://example.test/a\nb").contains('\n'));
        assert_eq!(target("not a url"), "<unparseable-url>");
    }

    #[test]
    fn a_run_that_reaches_nothing_still_announces_itself() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join(FILE_NAME);
        let sink = Sink::at(&path);

        // No request at all. The header is what makes "no request lines" mean
        // "none was made" instead of "nothing ran", and the second call must
        // not duplicate it.
        sink.begin_run();
        sink.begin_run();

        let log = read(&path);
        assert_eq!(log.matches(" run pid=").count(), 1, "{log}");
        assert!(!log.contains(" request "), "{log}");
    }

    #[test]
    fn a_failed_header_write_retries_on_the_next_run_marker() {
        let tmp = tempfile::TempDir::new().unwrap();
        let parent = tmp.path().join("later");
        let path = parent.join(FILE_NAME);
        let sink = Sink::at(&path);

        sink.begin_run();
        assert!(!path.exists());

        std::fs::create_dir(&parent).unwrap();
        sink.begin_run();

        let log = read(&path);
        assert_eq!(log.matches(" run pid=").count(), 1, "{log}");
    }

    #[tokio::test]
    async fn a_disabled_sink_writes_nothing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join(FILE_NAME);
        let sink = Sink::disabled();
        let client = reqwest::Client::new();

        sink.begin_run();
        // A closed port: the send fails, which is fine — the point is that a
        // disabled sink stays silent on the header, the attempt and the
        // outcome.
        let result = send_with(
            &sink,
            "custom",
            client.get("http://127.0.0.1:1/usage?token=leak"),
        )
        .await;

        assert!(result.is_err());
        assert!(!sink.is_enabled());
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn an_enabled_sink_records_the_header_the_attempt_and_the_status() {
        let mut server = mockito::Server::new_async().await;
        // The mock demands the query, so the test proves it travelled in the
        // request while the fixed-vendor log still has no trace of it.
        let mock = server
            .mock("GET", "/usage")
            .match_query(mockito::Matcher::UrlEncoded(
                "token".into(),
                "super-secret".into(),
            ))
            .with_status(200)
            .create_async()
            .await;
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join(FILE_NAME);
        let sink = Sink::at(&path);
        let client = reqwest::Client::new();

        let response = send_with(
            &sink,
            "zai",
            client.get(format!("{}/usage?token=super-secret", server.url())),
        )
        .await
        .expect("the mock answers");

        assert_eq!(response.status().as_u16(), 200);
        mock.assert_async().await;

        let log = read(&path);
        assert!(
            log.contains(&format!(" run pid={}", std::process::id())),
            "{log}"
        );
        assert!(
            log.contains(" request vendor=zai method=GET target="),
            "{log}"
        );
        assert!(log.contains("/usage"), "{log}");
        assert!(log.contains(" response vendor=zai status=200"), "{log}");
        assert!(!log.contains("super-secret"), "{log}");
    }

    #[tokio::test]
    async fn a_custom_request_logs_only_its_origin() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/bot/super-secret/usage")
            .match_query(mockito::Matcher::UrlEncoded(
                "token".into(),
                "also-secret".into(),
            ))
            .with_status(200)
            .create_async()
            .await;
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join(FILE_NAME);
        let sink = Sink::at(&path);
        let client = reqwest::Client::new();

        let response = send_with_detail(
            &sink,
            "custom",
            client.get(format!(
                "{}/bot/super-secret/usage?token=also-secret",
                server.url()
            )),
            TargetDetail::Origin,
        )
        .await
        .expect("the mock answers");

        assert_eq!(response.status().as_u16(), 200);
        mock.assert_async().await;

        let log = read(&path);
        assert!(log.contains(&format!("target={}", server.url())), "{log}");
        assert!(!log.contains("super-secret"), "{log}");
        assert!(!log.contains("also-secret"), "{log}");
    }

    #[tokio::test]
    async fn a_transport_failure_is_classified_not_rendered() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join(FILE_NAME);
        let sink = Sink::at(&path);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(200))
            .build()
            .unwrap();

        // A closed port, so the failure is a transport one whose `Display`
        // would otherwise carry the URL and its query string. Which kind it is
        // depends on the platform — a refusal is `connect`, a dropped SYN is
        // `timeout` — so the assertion is on the classification having
        // happened, not on the platform's verdict.
        let result = send_with(
            &sink,
            "custom",
            client.get("http://127.0.0.1:1/usage?token=super-secret"),
        )
        .await;

        assert!(result.is_err());
        let log = read(&path);
        let kind = log
            .lines()
            .find(|line| line.contains(" response vendor=custom "))
            .and_then(|line| line.split("error=").nth(1))
            .unwrap_or_else(|| panic!("no classified error line in {log}"));
        assert!(
            ["timeout", "connect", "body", "decode", "other"].contains(&kind),
            "{log}"
        );
        assert!(!log.contains("super-secret"), "{log}");
    }

    #[tokio::test]
    async fn the_header_is_written_once_per_sink_however_many_requests() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join(FILE_NAME);
        let sink = Sink::at(&path);
        let client = reqwest::Client::new();

        for _ in 0..3 {
            let _ = send_with(&sink, "zai", client.get("http://127.0.0.1:1/usage")).await;
        }

        let log = read(&path);
        assert_eq!(log.matches(" run pid=").count(), 1, "{log}");
        assert_eq!(log.matches(" request vendor=zai").count(), 3, "{log}");
    }

    /// A vendor that opens its own connection is invisible to the log, and the
    /// log is the only answer to "did this provider reach the network, or was
    /// the figure a cache hit?" — so a bare `.send()` is a gap, not a style
    /// choice. The needle is exact: `reqwest`'s `send` takes no arguments,
    /// while the channel sends (`worker.send(WorkerCmd::…)`) all pass one.
    ///
    /// Walking the tree rather than listing files is deliberate: a new vendor
    /// module is covered the day it lands, instead of the day someone
    /// remembers to add it here.
    #[test]
    fn no_request_bypasses_the_log() {
        let mut offenders = Vec::new();
        for file in crate::guard::rs_files_in("src") {
            if file.ends_with("request_log.rs") {
                continue;
            }
            let source = std::fs::read_to_string(&file).expect("readable module");
            for (n, line) in crate::guard::production_code(&source).lines().enumerate() {
                if line.contains(".send()") {
                    offenders.push(format!("{}:{}", file.display(), n + 1));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "these requests bypass request_log::send, so they are invisible to \
             AI_USAGEBAR_LOG_REQUESTS:\n{}",
            offenders.join("\n")
        );
    }
}
