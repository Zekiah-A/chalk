// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Zekiah-A

use anyhow::{Context, Result, bail, ensure};
use dialoguer::{FuzzySelect, Input, MultiSelect, Select, theme::ColorfulTheme};
use reqwest::{Url, blocking::Client, cookie::Jar};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, OnceLock},
    thread,
    time::{Duration, Instant},
};

static SITE: OnceLock<Url> = OnceLock::new();

fn site() -> &'static Url {
    #[cfg(test)]
    return SITE.get_or_init(|| Url::parse("https://blackboard.example").unwrap());
    #[cfg(not(test))]
    SITE.get().expect("site configured at startup")
}
fn base() -> &'static str {
    site().as_str().trim_end_matches('/')
}
fn same_site(url: &Url) -> bool {
    url.origin() == site().origin()
}
fn parse_site(value: &str) -> Result<Url> {
    let value = value.trim();
    ensure!(
        !value.is_empty(),
        "Provide your university's Blackboard domain"
    );
    let address = if value.contains("://") {
        value.to_owned()
    } else {
        format!("https://{value}")
    };
    let url =
        Url::parse(&address).context("Provide your university's Blackboard domain or HTTPS URL")?;
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.path() == "/"
            && url.query().is_none()
            && url.fragment().is_none(),
        "Use an HTTPS site address without credentials, a path, query or fragment"
    );
    Ok(url)
}
fn configure_site(args: &[String]) -> Result<()> {
    let mut supplied = None;
    if args.first().is_some_and(|a| a == "login") {
        let mut arguments = args.iter().skip(1);
        while let Some(argument) = arguments.next() {
            let value = if argument == "--url" {
                arguments
                    .next()
                    .context("--url requires a Blackboard domain or HTTPS URL")?
            } else {
                ensure!(
                    !argument.starts_with('-'),
                    "Unknown login option: {argument}"
                );
                argument
            };
            ensure!(
                supplied.is_none(),
                "Specify one Blackboard address: chalk login learn.youruni.ac.uk"
            );
            supplied = Some(value.as_str());
        }
    } else {
        ensure!(
            !args.iter().any(|a| a == "--url"),
            "Configure your university with `chalk login learn.youruni.ac.uk`"
        );
    }
    let path = state_dir().join("site.json");
    let stored: Option<String> = if supplied.is_none() && path.exists() {
        Some(serde_json::from_slice(&fs::read(&path)?)?)
    } else {
        None
    };
    let url = parse_site(
        supplied
            .or(stored.as_deref())
            .context("Configure your university first: chalk login learn.youruni.ac.uk")?,
    )?;
    if supplied.is_some() {
        private_dir(&state_dir())?;
        fs::write(&path, serde_json::to_vec(url.as_str())?)?;
    }
    SITE.set(url)
        .map_err(|_| anyhow::anyhow!("Site already configured"))
}
mod browse;
mod mcp;

const BANNER: &str = include_str!("../assets/banner.txt");

// Keep the welcome mark only while the first menu is on screen. Menus must
// disable selection reports so their cleanup leaves the cursor below the mark.
fn first_menu<T>(interact: impl FnOnce() -> Result<T>) -> Result<T> {
    use std::sync::atomic::{AtomicBool, Ordering};
    static SHOWN: AtomicBool = AtomicBool::new(false);
    let term = console::Term::stderr();
    if !term.is_term() || SHOWN.swap(true, Ordering::Relaxed) {
        return interact();
    }
    let width = usize::from(term.size().1).max(1);
    let lines: usize = format!("{BANNER}\n")
        .lines()
        .map(|line| console::measure_text_width(line).div_ceil(width).max(1))
        .sum();
    term.write_line(BANNER)?;
    let result = interact();
    term.clear_last_lines(lines)?;
    result
}

fn download_host(host: &str) -> bool {
    host == site().host_str().unwrap()
        || host.ends_with(".blackboard.com")
        || host.ends_with(".blackboardcdn.com")
}

#[derive(Clone, Serialize, Deserialize)]
struct Cookie {
    name: String,
    value: String,
    domain: String,
    path: String,
    #[serde(default)]
    secure: bool,
    #[serde(default)]
    expires: f64,
}

fn state_dir() -> PathBuf {
    let root = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state")
        });
    state_path(&root)
}
fn state_path(root: &Path) -> PathBuf {
    let preferred = root.join("chalk");
    let legacy = root.join("coursegrab");
    if !preferred.exists() && legacy.exists() {
        legacy
    } else {
        preferred
    }
}
fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
fn save_session(cookies: &[Cookie]) -> Result<()> {
    let dir = state_dir();
    private_dir(&dir)?;
    let path = dir.join("session.json");
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(&serde_json::to_vec(cookies)?)?;
    Ok(())
}
fn client(cookies: &[Cookie]) -> Result<Client> {
    let jar = Arc::new(Jar::default());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs_f64();
    for c in cookies {
        if Some(c.domain.trim_start_matches('.')) != site().host_str()
            || (c.expires > 0.0 && c.expires < now)
        {
            continue;
        }
        let mut line = format!(
            "{}={}; Domain={}; Path={}",
            c.name, c.value, c.domain, c.path
        );
        if c.secure {
            line.push_str("; Secure");
        }
        jar.add_cookie_str(&line, site());
    }
    Ok(Client::builder()
        .cookie_provider(jar)
        .timeout(Duration::from_secs(90))
        .user_agent(concat!(
            "Chalk/",
            env!("CARGO_PKG_VERSION"),
            " (personal Blackboard client)"
        ))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                return attempt.error("too many redirects");
            }
            if attempt.url().scheme() != "https"
                || !attempt.url().host_str().is_some_and(download_host)
            {
                return attempt.stop();
            }
            attempt.follow()
        }))
        .build()?)
}
fn get(client: &Client, path: &str) -> Result<Value> {
    let url = site().join(path)?;
    ensure!(same_site(&url), "Refusing off-site API URL");
    for attempt in 0..3 {
        let response = client.get(url.clone()).send()?;
        let status = response.status();
        if (status.as_u16() == 429 || status.is_server_error()) && attempt < 2 {
            let delay = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(2_u64.pow(attempt))
                .min(30);
            thread::sleep(Duration::from_secs(delay));
            continue;
        }
        ensure!(
            status.as_u16() != 401 && status.as_u16() != 403,
            "Session expired or access denied. Run `chalk login`."
        );
        ensure!(
            status.is_success(),
            "API returned {status} for {}",
            url.path()
        );
        ensure!(
            response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .contains("json"),
            "Blackboard returned a login page. Run `chalk login`."
        );
        return Ok(response.json()?);
    }
    bail!("API retries exhausted")
}
fn pages(client: &Client, path: &str) -> Result<Vec<Value>> {
    let mut next = path.to_string();
    let mut output = Vec::new();
    let mut seen = HashSet::new();
    while !next.is_empty() {
        ensure!(seen.insert(next.clone()), "API pagination loop");
        let data = get(client, &next)?;
        output.extend(
            data["results"]
                .as_array()
                .context("Missing results in API response")?
                .iter()
                .cloned(),
        );
        next = data["paging"]["nextPage"]
            .as_str()
            .unwrap_or("")
            .to_string();
    }
    Ok(output)
}

fn login() -> Result<()> {
    let state = state_dir();
    private_dir(&state)?;
    let profile = state.join("browser");
    private_dir(&profile)?;
    let browser = std::env::var("CHALK_BROWSER")
        .or_else(|_| std::env::var("COURSEGRAB_BROWSER"))
        .ok()
        .or_else(|| {
            [
                "brave",
                "brave-browser",
                "chromium",
                "chromium-browser",
                "google-chrome",
            ]
            .iter()
            .find(|name| {
                Command::new("which")
                    .arg(name)
                    .output()
                    .is_ok_and(|o| o.status.success())
            })
            .map(|s| s.to_string())
        })
        .context("Install Brave/Chromium, or set CHALK_BROWSER to its executable")?;
    // A fresh debugging port is bound only to loopback; never debug the user's normal profile.
    let port_file = profile.join("DevToolsActivePort");
    let _ = fs::remove_file(&port_file);
    let mut child = Command::new(browser)
        .arg(format!("--user-data-dir={}", profile.display()))
        .args([
            "--remote-debugging-address=127.0.0.1",
            "--remote-debugging-port=0",
            "--no-first-run",
            "--no-default-browser-check",
            base(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    println!(
        "Sign into your university’s Blackboard in the Chalk browser window. Waiting up to 5 minutes…"
    );
    let result = (|| -> Result<Vec<Cookie>> {
        let deadline = Instant::now() + Duration::from_secs(300);
        let port_info = loop {
            if let Ok(text) = fs::read_to_string(&port_file)
                && text.lines().count() >= 2
            {
                break text;
            }
            ensure!(Instant::now() < deadline, "Browser did not start");
            ensure!(child.try_wait()?.is_none(), "Browser exited before sign-in");
            thread::sleep(Duration::from_millis(300));
        };
        let lines: Vec<_> = port_info.lines().collect();
        let socket_url = format!("ws://127.0.0.1:{}{}", lines[0], lines[1]);
        let (mut socket, _) = tungstenite::connect(socket_url.as_str())?;
        if let tungstenite::stream::MaybeTlsStream::Plain(stream) = socket.get_mut() {
            stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        }
        let mut id = 0_u64;
        while Instant::now() < deadline {
            id += 1;
            socket.send(tungstenite::Message::Text(
                json!({"id":id,"method":"Storage.getCookies"})
                    .to_string()
                    .into(),
            ))?;
            let data = loop {
                let message = socket.read()?;
                if let Ok(text) = message.to_text() {
                    let value: Value = serde_json::from_str(text)?;
                    if value["id"] == id {
                        break value;
                    }
                }
            };
            ensure!(data.get("error").is_none(), "Browser refused cookie access");
            let all: Vec<Cookie> = serde_json::from_value(data["result"]["cookies"].clone())?;
            let cookies: Vec<_> = all
                .into_iter()
                .filter(|c| Some(c.domain.trim_start_matches('.')) == site().host_str())
                .collect();
            if !cookies.is_empty() && get(&client(&cookies)?, "/learn/api/v1/users/me").is_ok() {
                return Ok(cookies);
            }
            thread::sleep(Duration::from_secs(2));
        }
        bail!("Login timed out; run `chalk login` to try again")
    })();
    // End debugging and release browser memory when the login flow finishes.
    let _ = child.kill();
    let _ = child.wait();
    save_session(&result?)?;
    println!("Signed in. Session saved locally with owner-only permissions.");
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Resource {
    id: String,
    title: String,
    path: String,
    kind: String,
    source: String,
    html: String,
    files: Vec<Attachment>,
    metadata: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Attachment {
    name: String,
    url: String,
}
fn safe(text: &str) -> String {
    let s: String = text
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(120)
        .collect();
    let s = s.trim().trim_matches('.');
    if s.is_empty() {
        "untitled".into()
    } else {
        s.into()
    }
}
fn attachments(html: &str) -> Vec<Attachment> {
    let doc = Html::parse_fragment(html);
    let selector = Selector::parse("a[href], img[src]").unwrap();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for element in doc.select(&selector) {
        let value = element.value();
        let href = value
            .attr("href")
            .or_else(|| value.attr("src"))
            .unwrap_or("");
        let Ok(url) = site().join(href) else {
            continue;
        };
        if !same_site(&url)
            || !url.path().starts_with("/bbcswebdav/")
            || !seen.insert(url.to_string())
        {
            continue;
        }
        let meta: Value = value
            .attr("data-bbfile")
            .and_then(|v| serde_json::from_str(v).ok())
            .unwrap_or(Value::Null);
        let text = element.text().collect::<String>();
        let name = meta["displayName"]
            .as_str()
            .or(meta["linkName"].as_str())
            .or(value.attr("alt"))
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                if text.trim().is_empty() {
                    url.path_segments()
                        .and_then(|mut p| p.next_back())
                        .unwrap_or("attachment")
                        .into()
                } else {
                    text.trim().into()
                }
            });
        out.push(Attachment {
            name,
            url: url.into(),
        });
    }
    out
}

fn select_with_children(all: Vec<Resource>, selected: &[Resource]) -> Vec<Resource> {
    let mut ids: HashSet<String> = selected.iter().map(|r| r.id.clone()).collect();
    loop {
        let before = ids.len();
        for resource in &all {
            if resource.metadata["parentId"]
                .as_str()
                .is_some_and(|p| ids.contains(p))
            {
                ids.insert(resource.id.clone());
            }
        }
        if before == ids.len() {
            break;
        }
    }
    all.into_iter().filter(|r| ids.contains(&r.id)).collect()
}
fn crawl(
    client: &Client,
    course: &str,
    parent: &str,
    path: &str,
    visited: &mut HashSet<String>,
    out: &mut Vec<Resource>,
) -> Result<()> {
    ensure!(visited.len() < 10000, "Content tree exceeded safety limit");
    if !visited.insert(parent.into()) {
        return Ok(());
    }
    let items = pages(
        client,
        &format!("/learn/api/v1/courses/{course}/contents/{parent}/children?limit=100"),
    )?;
    for item in items {
        let resource = resource(&item, course, path)?;
        let id = resource.id.clone();
        let item_path = resource.path.clone();
        out.push(resource);
        if is_container(&item) {
            crawl(client, course, &id, &item_path, visited, out)?;
        }
    }
    Ok(())
}

fn is_container(item: &Value) -> bool {
    item["contentDetail"].as_object().is_some_and(|m| {
        m.values()
            .any(|v| v["isFolder"] == true || v["isLesson"] == true)
    }) || matches!(
        item["contentHandler"].as_str(),
        Some("resource/x-bb-folder" | "resource/x-bb-lesson")
    )
}

fn resource(item: &Value, course: &str, path: &str) -> Result<Resource> {
    let base = base();
    let id = item["id"]
        .as_str()
        .context("Content has no ID")?
        .to_string();
    let title = item["title"].as_str().unwrap_or("Untitled").to_string();
    let handler = item["contentHandler"].as_str().unwrap_or("");
    let item_path = if path.is_empty() {
        title.clone()
    } else {
        format!("{path}/{title}")
    };
    let html = item["body"]["rawText"]
        .as_str()
        .or(item["body"]["displayText"].as_str())
        .unwrap_or("")
        .to_string();
    let mut html = if html.is_empty() {
        item["description"].as_str().unwrap_or("").into()
    } else {
        html
    };
    let mut files = attachments(&html);
    if let Some(link) = item["contentDetail"]["resource/x-bb-externallink"]["url"].as_str() {
        html.push_str(&format!(
            "\n<p>Linked resource: {}</p>",
            link.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        ));
        if let Ok(url) = site().join(link)
            && same_site(&url)
            && url.path().starts_with("/bbcswebdav/")
        {
            files.push(Attachment {
                name: title.clone(),
                url: url.into(),
            });
        }
    }
    Ok(Resource {
        id: id.clone(),
        title,
        path: item_path.clone(),
        kind: handler.into(),
        source: if item["contentDetail"]
            .as_object()
            .is_some_and(|m| m.values().any(|v| v["isBbPage"] == true))
        {
            format!("{base}/ultra/courses/{course}/document/{id}?view=content")
        } else {
            format!("{base}/ultra/courses/{course}/outline?contentId={id}")
        },
        files,
        html,
        metadata: item.clone(),
    })
}
fn announcements(client: &Client, course: &str) -> Result<Vec<Resource>> {
    let base = base();
    Ok(pages(client,&format!("/learn/api/v1/courses/{course}/announcements?limit=100&sort=startDateRestriction(desc)"))?.into_iter().map(|item| {
        let id=item["id"].as_str().unwrap_or("").to_string(); let title=item["title"].as_str().unwrap_or("Untitled").to_string();
        let body=item["body"]["rawText"].as_str().unwrap_or("");
        let html=format!("<p>Published: {}</p>\n{body}",item["startDateRestriction"].as_str().or(item["createdDate"].as_str()).unwrap_or("Unknown"));
        Resource{id:id.clone(),path:format!("Announcements/{title}"),title,kind:"announcement".into(),source:format!("{base}/ultra/courses/{course}/announcements/announcement-detail?courseId={course}&announcementId={id}"),files:attachments(&html),html,metadata:item.clone()}
    }).collect())
}
fn export(client: &Client, resources: &[Resource], out: &Path, course: &str) -> Result<()> {
    fs::create_dir_all(out)?;
    let mut index = format!(
        "# Blackboard context: {course}\n\nExported from your university’s Blackboard. Source text is authoritative; this export does not infer requirements or resolve contradictory announcements. External tools, discussions and hidden content are not included.\n\n"
    );
    let mut context = index.clone();
    let mut jobs = Vec::new();
    for resource in resources {
        let folder = out
            .join(resource.path.split('/').map(safe).collect::<PathBuf>())
            .join(safe(&resource.id));
        fs::create_dir_all(&folder)?;
        let markdown = format!(
            "# {}\n\nSource: {}\n\n{}\n",
            resource.title,
            resource.source,
            html2md::parse_html(&resource.html)
        );
        fs::write(folder.join("content.md"), &markdown)?;
        index.push_str(&format!("- {} — {}\n", resource.path, resource.source));
        context.push_str(&format!("\n---\n\nPath: {}\n\n{markdown}", resource.path));
        for (i, file) in resource.files.iter().enumerate() {
            let filename = format!("{:02}-{}", i + 1, safe(&file.name));
            jobs.push((file.clone(), folder.join(&filename)));
            context.push_str(&format!(
                "\nAttachment: {} ({})\n",
                file.name,
                folder.join(filename).strip_prefix(out)?.display()
            ));
        }
    }
    fs::write(out.join("INDEX.md"), index)?;
    fs::write(out.join("CONTEXT.md"), &context)?;
    fs::write(
        out.join("manifest.json"),
        serde_json::to_vec_pretty(resources)?,
    )?;
    let failures = std::sync::Mutex::new(Vec::new());
    let queue = std::sync::Mutex::new(jobs.into_iter());
    thread::scope(|scope| {
        for _ in 0..4 {
            let queue = &queue;
            let failures = &failures;
            scope.spawn(move || {
                loop {
                    let Some((file, path)) = queue.lock().unwrap().next() else {
                        break;
                    };
                    if let Err(error) = download(client, &file.url, &path) {
                        failures
                            .lock()
                            .unwrap()
                            .push(format!("{}: {error:#}", file.name));
                    }
                }
            });
        }
    });
    let failures = failures.into_inner().unwrap();
    fs::write(out.join("download-errors.txt"), failures.join("\n"))?;
    // Optional system helper: keep originals regardless of text extraction availability.
    let mut text_notes = Vec::new();
    for resource in resources {
        let folder = out
            .join(resource.path.split('/').map(safe).collect::<PathBuf>())
            .join(safe(&resource.id));
        for (i, file) in resource.files.iter().enumerate() {
            let path = folder.join(format!("{:02}-{}", i + 1, safe(&file.name)));
            if path.exists()
                && (path.extension().is_none()
                    || path
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("html")))
            {
                let bytes = fs::read(&path)?;
                if let Ok(html) = std::str::from_utf8(&bytes) {
                    let text = html2md::parse_html(html);
                    fs::write(path.with_extension("md"), &text)?;
                    context.push_str(&format!(
                        "\n---\n\n## Attachment text: {}\n\nSource: {}\n\n{}\n",
                        file.name, resource.source, text
                    ));
                }
            }
            if !file.name.to_lowercase().ends_with(".pdf") || !path.exists() {
                continue;
            }
            match Command::new("pdftotext")
                .arg("-layout")
                .arg(&path)
                .arg("-")
                .output()
            {
                Ok(output) if output.status.success() => {
                    let text = String::from_utf8_lossy(&output.stdout);
                    if text.trim().is_empty() {
                        text_notes.push(format!("{}: no PDF text; OCR may be needed", file.name));
                    }
                    fs::write(path.with_extension("txt"), text.as_bytes())?;
                    context.push_str(&format!(
                        "\n---\n\n## Attachment text: {}\n\nSource: {}\n\n{}\n",
                        file.name, resource.source, text
                    ));
                }
                _ => text_notes.push(format!(
                    "{}: PDF text extraction unavailable; original retained",
                    file.name
                )),
            }
        }
    }
    fs::write(out.join("CONTEXT.md"), context)?;
    fs::write(out.join("extraction-notes.txt"), text_notes.join("\n"))?;
    eprintln!("Exported {} items to {}", resources.len(), out.display());
    ensure!(
        failures.is_empty(),
        "{} attachments failed; see download-errors.txt",
        failures.len()
    );
    Ok(())
}
fn download(client: &Client, url: &str, path: &Path) -> Result<()> {
    let url = Url::parse(url)?;
    ensure!(same_site(&url), "Off-site attachment");
    let mut response = client.get(url).send()?;
    ensure!(
        response.status().is_success(),
        "Download returned {}",
        response.status()
    );
    let is_html = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .contains("text/html");
    ensure!(
        !is_html
            || path.extension().is_none()
            || path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("html")),
        "HTML returned in place of file (possibly expired session)"
    );
    ensure!(
        !response.url().path().contains("/login"),
        "Download redirected to sign-in"
    );
    let temp = path.with_file_name(format!(
        "{}.part",
        path.file_name().unwrap().to_string_lossy()
    ));
    let result = (|| -> Result<()> {
        let mut file = fs::File::create(&temp)?;
        if is_html {
            let text = response.text()?;
            let doc = Html::parse_document(&text);
            ensure!(
                doc.select(&Selector::parse("input[type=password]").unwrap())
                    .next()
                    .is_none(),
                "Login form returned in place of attachment"
            );
            file.write_all(text.as_bytes())?;
        } else {
            response.copy_to(&mut file)?;
        }
        file.sync_all()?;
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("chalk {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{BANNER}");
        println!("chalk {}\n", env!("CARGO_PKG_VERSION"));
        println!(
            "chalk [browse | grab | login | export | mcp]\n\nNo arguments / browse: navigate course folders, read text, open selected files\nbrowse [--course ID_OR_NAME] [--save-only] [--fresh] [--out DIR]\ngrab: bulk download course files and text for you or an AI agent\nlogin [DOMAIN_OR_HTTPS_URL]: configure your university and sign in (also accepts --url)\nmcp: start the stdio MCP server (sign in first)\nexport --course ID_OR_NAME [--assessment] [--all-announcements] [--out DIR]\n\nBrowsing: Enter selects, type to filter, Esc goes back.\nUse --version to show the installed version.\nSet CHALK_BROWSER to a Chromium/Brave executable if needed."
        );
        return Ok(());
    }
    configure_site(&args)?;
    if args.first().is_some_and(|a| a == "mcp") {
        return mcp::serve();
    }
    if args.first().is_some_and(|a| a == "login") {
        return login();
    }
    if let Some(command) = args.first().filter(|a| !a.starts_with('-')) {
        ensure!(
            ["browse", "grab", "export"].contains(&command.as_str()),
            "Unknown command: {command}. Use --help."
        );
    }
    let session = state_dir().join("session.json");
    if !session.exists() {
        login()?;
    }
    let cookies: Vec<Cookie> = serde_json::from_slice(&fs::read(session)?)?;
    let client = client(&cookies)?;
    let me = get(&client, "/learn/api/v1/users/me")?;
    let user = me["id"].as_str().context("Missing user ID")?;
    let memberships = pages(
        &client,
        &format!(
            "/learn/api/v1/users/{user}/memberships?expand=course.effectiveAvailability&limit=100"
        ),
    )?;
    let courses: Vec<_> = memberships
        .iter()
        .filter_map(|m| m.get("course"))
        .filter(|c| c["id"].is_string())
        .collect();
    ensure!(!courses.is_empty(), "No courses returned");
    let flag = |name: &str| -> Option<String> {
        args.windows(2).find(|p| p[0] == name).map(|p| p[1].clone())
    };
    let theme = ColorfulTheme::default();
    let labels: Vec<_> = courses
        .iter()
        .map(|c| {
            format!(
                "{}  {}",
                c["name"]
                    .as_str()
                    .or(c["displayName"].as_str())
                    .unwrap_or(""),
                c["courseId"].as_str().unwrap_or("")
            )
        })
        .collect();
    let query = flag("--course");
    let pick_course = || -> Result<Option<&Value>> {
        if let Some(query) = &query {
            let matches: Vec<_> = courses
                .iter()
                .enumerate()
                .filter(|(i, c)| {
                    c["id"].as_str() == Some(query.as_str())
                        || labels[*i].to_lowercase().contains(&query.to_lowercase())
                })
                .collect();
            ensure!(
                matches.len() == 1,
                "Course must match exactly one entry; found {}",
                matches.len()
            );
            Ok(Some(*matches[0].1))
        } else {
            let labels: Vec<_> = labels.iter().map(|s| browse::label(s)).collect();
            first_menu(|| {
                Ok(FuzzySelect::with_theme(&theme)
                    .with_prompt("Course (type to search)")
                    .items(&labels)
                    .report(false)
                    .interact_opt()?
                    .map(|index| courses[index]))
            })
        }
    };
    if !args.first().is_some_and(|a| a == "grab" || a == "export") {
        while let Some(course) = pick_course()? {
            let id = course["id"].as_str().unwrap();
            let title = course["name"]
                .as_str()
                .or(course["displayName"].as_str())
                .unwrap_or(id);
            browse::run(
                &client,
                id,
                title,
                browse::Options {
                    download_dir: flag("--out")
                        .map(PathBuf::from)
                        .unwrap_or_else(|| state_dir().join("downloads")),
                    open_files: !args.iter().any(|a| a == "--save-only"),
                    fresh: args.iter().any(|a| a == "--fresh"),
                },
            )?;
            if query.is_some() {
                break;
            }
        }
        return Ok(());
    }
    let Some(course) = pick_course()? else {
        return Ok(());
    };
    let id = course["id"].as_str().unwrap();
    let title = course["name"]
        .as_str()
        .or(course["displayName"].as_str())
        .unwrap_or(id);
    let batch = args.first().is_some_and(|a| a == "export");
    let mode = if batch {
        0
    } else {
        first_menu(|| {
            Ok(Select::with_theme(&theme)
                .with_prompt("What to grab?")
                .items([
                    "Content and announcements",
                    "Content only",
                    "Announcements only",
                ])
                .default(0)
                .report(false)
                .interact()?)
        })?
    };
    let mut resources = Vec::new();
    if mode != 2 {
        println!("Reading course content…");
        crawl(&client, id, "ROOT", "", &mut HashSet::new(), &mut resources)?;
    }
    if args.iter().any(|a| a == "--assessment") {
        resources.retain(|r| r.path.to_lowercase().contains("assessment"));
    }
    if mode != 1 && (!batch || args.iter().any(|a| a == "--all-announcements")) {
        resources.extend(announcements(&client, id)?);
    }
    if !batch {
        let all_resources = resources.clone();
        let query: String = Input::with_theme(&theme)
            .with_prompt("Filter title/path/extension (blank = all)")
            .allow_empty(true)
            .interact_text()?;
        if !query.is_empty() {
            let q = query.to_lowercase();
            resources.retain(|r| {
                r.path.to_lowercase().contains(&q)
                    || r.files.iter().any(|f| f.name.to_lowercase().contains(&q))
            });
        }
        let labels: Vec<_> = resources
            .iter()
            .map(|r| format!("{}  [{} files]", r.path, r.files.len()))
            .collect();
        ensure!(!labels.is_empty(), "No matching items");
        let selected = MultiSelect::with_theme(&theme)
            .with_prompt("Space: toggle · a: toggle all · Enter: download")
            .items(&labels)
            .defaults(&vec![true; labels.len()])
            .interact()?;
        let selected: Vec<_> = selected.into_iter().map(|i| resources[i].clone()).collect();
        resources = select_with_children(all_resources, &selected);
    }
    ensure!(!resources.is_empty(), "Nothing selected");
    let out = PathBuf::from(flag("--out").unwrap_or_else(|| format!("exports/{}", safe(title))));
    export(&client, &resources, &out, title)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirects_accept_blackboard_hosts_without_accepting_lookalikes() {
        assert!(download_host("files.blackboard.com"));
        assert!(!download_host("blackboard.com.evil.example"));
        assert!(!download_host("evilblackboard.com"));
        let html = r#"<a href="https://evil.example/bbcswebdav/secret.pdf">Bad</a><a href="http://blackboard.example/bbcswebdav/file">Bad</a><a href="https://blackboard.example.evil.example/bbcswebdav/file">Bad</a><img src="/bbcswebdav/xid-2_1" alt="diagram.png">"#;
        let files = attachments(html);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "diagram.png");
    }

    #[test]
    fn ultra_empty_anchor_keeps_pdf_filename_and_deduplicates() {
        let html = r#"<a href="/bbcswebdav/xid-123_1" data-bbfile="{&quot;displayName&quot;:&quot;Assessment brief.pdf&quot;}"></a><a href="/bbcswebdav/xid-123_1">duplicate</a>"#;
        let files = attachments(html);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "Assessment brief.pdf");
        assert_eq!(
            files[0].url,
            "https://blackboard.example/bbcswebdav/xid-123_1"
        );
    }

    #[test]
    fn unsafe_titles_cannot_escape_export_directory() {
        for title in ["..", ".", "../../home", "a\\b", "a/b", "\n", ""] {
            let part = safe(title);
            assert!(!part.is_empty());
            assert_ne!(part, "..");
            assert!(!part.contains('/') && !part.contains('\\'));
        }
    }
}
