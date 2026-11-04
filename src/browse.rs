// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Zekiah-A

//! On-demand student navigation. Bulk crawling happens only after an explicit export.
use super::*;
use console::Term;
use std::{collections::HashMap, io::Read};

pub(super) struct Options {
    pub download_dir: PathBuf,
    pub open_files: bool,
    pub fresh: bool,
}

#[derive(Debug, PartialEq)]
enum Kind {
    Folder,
    Document,
    File,
    Link,
    Tool,
}

fn kind(item: &Value) -> Kind {
    let handler = item["contentHandler"].as_str().unwrap_or("");
    if item["contentDetail"]
        .as_object()
        .is_some_and(|m| m.values().any(|v| v["isBbPage"] == true))
        || handler == "resource/x-bb-document"
    {
        Kind::Document
    } else if is_container(item) {
        Kind::Folder
    } else if handler == "resource/x-bb-file" {
        Kind::File
    } else if handler == "resource/x-bb-externallink" {
        let url = item["contentDetail"][handler]["url"].as_str().unwrap_or("");
        if !attachments(&format!("<a href=\"{url}\">file</a>")).is_empty() {
            Kind::File
        } else {
            Kind::Link
        }
    } else if handler == "resource/x-bb-courselink" {
        Kind::Link
    } else {
        Kind::Tool
    }
}

// Blackboard text is source material, never terminal escape sequences.
fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| {
            (!c.is_control() || matches!(c, '\n' | '\t'))
                && !matches!(*c,'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}')
        })
        .collect()
}
pub(super) fn label(text: &str) -> String {
    clean(text).replace(['\n', '\t'], " ")
}
fn title(item: &Value) -> &str {
    item["title"].as_str().unwrap_or("Untitled")
}

fn choose(prompt: &str, labels: &[String], selected: usize) -> Result<Option<usize>> {
    first_menu(|| {
        Ok(FuzzySelect::with_theme(&ColorfulTheme::default())
            .with_prompt(format!("{} · type to filter · Esc back", label(prompt)))
            .items(labels)
            .default(selected.min(labels.len().saturating_sub(1)))
            .max_length(16)
            .report(false)
            .interact_opt()?)
    })
}

fn cached<T: Clone>(
    map: &mut HashMap<String, T>,
    key: &str,
    fetch: impl FnOnce() -> Result<T>,
) -> Result<T> {
    if let Some(value) = map.get(key) {
        return Ok(value.clone());
    }
    let value = fetch()?;
    map.insert(key.into(), value.clone());
    Ok(value)
}

struct Session<'a> {
    client: &'a Client,
    course: &'a str,
    name: &'a str,
    options: Options,
    children: HashMap<String, Vec<Value>>,
    items: HashMap<String, Value>,
    announcements: Option<Vec<Resource>>,
}
impl Session<'_> {
    fn children(&mut self, id: &str) -> Result<Vec<Value>> {
        cached(&mut self.children, id, || {
            pages(
                self.client,
                &format!(
                    "/learn/api/v1/courses/{}/contents/{id}/children?limit=100",
                    self.course
                ),
            )
        })
    }
    fn item(&mut self, id: &str) -> Result<Value> {
        cached(&mut self.items, id, || {
            get(
                self.client,
                &format!("/learn/api/v1/courses/{}/contents/{id}", self.course),
            )
        })
    }
    fn refresh(&mut self) {
        self.children.clear();
        self.items.clear();
        self.announcements = None;
        self.options.fresh = true;
    }
    fn page(&mut self, item: &Value) -> Result<Page> {
        let id = item["id"].as_str().context("Missing content ID")?;
        let detail = self.item(id)?;
        let children = if kind(&detail) == Kind::Document && is_container(&detail) {
            self.children(id)?
        } else {
            Vec::new()
        };
        Page::from_items(&detail, &children, self.course)
    }
    fn save_file(&self, page: &Page, index: usize) -> Result<PathBuf> {
        let file = &page.files[index];
        let path = file_path(
            &self.options.download_dir,
            self.course,
            &page.id,
            page.version,
            index,
            &file.name,
        );
        let reusable = page.version.is_some() && path.metadata().is_ok_and(|m| m.len() > 0);
        if !self.options.fresh && reusable {
            println!("Cached: {}", path.display());
        } else {
            fs::create_dir_all(path.parent().context("Missing download directory")?)?;
            println!("Downloading {}…", label(&file.name));
            download(self.client, &file.url, &path)?;
            println!("Saved: {}", path.display());
        }
        Ok(path)
    }
    fn select_file(&self, page: &Page, index: usize) -> Result<()> {
        let path = self.save_file(page, index)?;
        if !self.options.open_files {
            return Ok(());
        }
        let mut prefix = Vec::new();
        fs::File::open(&path)?.take(4096).read_to_end(&mut prefix)?;
        let html = std::str::from_utf8(&prefix).is_ok_and(|s| s.trim_start().starts_with('<'));
        if html
            || path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("txt"))
        {
            let text = fs::read_to_string(&path).context("Cannot display file as UTF-8 text")?;
            let markdown = if html {
                html2md::parse_html(&text)
            } else {
                text
            };
            read_markdown(&page.files[index].name, &markdown)?;
        } else if viewer_file(&path) || prefix.starts_with(b"%PDF-") {
            open(&fs::canonicalize(&path)?.to_string_lossy())?;
        } else {
            println!("Saved for manual use; this file type isn't opened automatically.");
        }
        Ok(())
    }
    fn export_section(&self, id: &str, title: &str) -> Result<()> {
        let mut resources = Vec::new();
        if id != "ROOT" {
            resources.push(resource(
                &get(
                    self.client,
                    &format!("/learn/api/v1/courses/{}/contents/{id}", self.course),
                )?,
                self.course,
                "",
            )?);
        }
        crawl(
            self.client,
            self.course,
            id,
            if id == "ROOT" { "" } else { title },
            &mut HashSet::new(),
            &mut resources,
        )?;
        // Whole-course export includes announcements; a section export stays local to that section.
        if id == "ROOT" {
            resources.extend(announcements(self.client, self.course)?);
        }
        let path = PathBuf::from("exports")
            .join(safe(self.course))
            .join(safe(id));
        export(self.client, &resources, &path, self.name)
    }
    fn view_page(&mut self, page: &Page) -> Result<()> {
        if page.files.len() == 1 && !page.has_text {
            return self.select_file(page, 0);
        }
        if page.files.is_empty() && page.links.is_empty() {
            loop {
                let text = if page.markdown.trim().is_empty() {
                    "No readable text is available for this item."
                } else {
                    &page.markdown
                };
                let action = read_text(
                    &page.title,
                    text,
                    &["Export this item".into(), "Open source in browser".into()],
                )?;
                let result = match action {
                    Some(0) => {
                        let out = PathBuf::from("exports")
                            .join(safe(self.course))
                            .join(safe(&page.id));
                        export(self.client, &page.resources, &out, self.name)
                    }
                    Some(1) => open(&page.source),
                    _ => return Ok(()),
                };
                if let Err(error) = result {
                    eprintln!("Could not open item: {error:#}");
                }
            }
        }
        let mut selected = 0;
        loop {
            let mut labels = Vec::new();
            if page.has_text {
                labels.push("Read text".into());
            }
            labels.extend(
                page.files
                    .iter()
                    .map(|f| format!("Open file · {}", label(&f.name))),
            );
            labels.extend(
                page.links
                    .iter()
                    .map(|(text, _)| format!("Open link · {}", label(text))),
            );
            labels.extend([
                "Export this item".into(),
                "Open source in browser".into(),
                "Back".into(),
            ]);
            let Some(choice) = choose(&page.title, &labels, selected)? else {
                return Ok(());
            };
            selected = choice;
            let result = if page.has_text && choice == 0 {
                read_markdown(&page.title, &page.markdown)
            } else {
                let index = choice - usize::from(page.has_text);
                if index < page.files.len() {
                    self.select_file(page, index)
                } else if index < page.files.len() + page.links.len() {
                    open(&page.links[index - page.files.len()].1)
                } else {
                    match index - page.files.len() - page.links.len() {
                        0 => {
                            let path = PathBuf::from("exports")
                                .join(safe(self.course))
                                .join(safe(&page.id));
                            export(self.client, &page.resources, &path, self.name)
                        }
                        1 => open(&page.source),
                        _ => return Ok(()),
                    }
                }
            };
            if let Err(error) = result {
                eprintln!("Could not open item: {error:#}");
            }
        }
    }
    fn view_announcements(&mut self) -> Result<()> {
        let mut selected = 0;
        loop {
            if self.announcements.is_none() {
                self.announcements = Some(announcements(self.client, self.course)?);
            }
            let items = self.announcements.as_ref().unwrap();
            let mut labels: Vec<_> = items
                .iter()
                .map(|r| {
                    format!(
                        "{} · {}",
                        r.metadata["startDateRestriction"]
                            .as_str()
                            .unwrap_or("")
                            .split('T')
                            .next()
                            .unwrap_or(""),
                        label(&r.title)
                    )
                })
                .collect();
            let count = labels.len();
            labels.extend(["Refresh".into(), "Back".into()]);
            let Some(choice) =
                choose(&format!("{} / Announcements", self.name), &labels, selected)?
            else {
                return Ok(());
            };
            selected = choice;
            if choice == count {
                self.announcements = None;
                continue;
            }
            if choice > count {
                return Ok(());
            }
            let resource = items[choice].clone();
            let page = Page::from_resources(vec![resource]);
            self.view_page(&page)?;
        }
    }
}

struct Frame {
    id: String,
    title: String,
    selected: usize,
}

pub(super) fn run(client: &Client, course: &str, name: &str, options: Options) -> Result<()> {
    let mut session = Session {
        client,
        course,
        name,
        options,
        children: HashMap::new(),
        items: HashMap::new(),
        announcements: None,
    };
    let mut stack = vec![Frame {
        id: "ROOT".into(),
        title: name.into(),
        selected: 0,
    }];
    while !stack.is_empty() {
        let frame = stack.last().unwrap();
        let current = frame.id.clone();
        let children = match session.children(&current) {
            Ok(items) => items,
            Err(error) => {
                eprintln!("Could not load this section: {error:#}");
                if choose("Load failed", &["Retry".into(), "Back".into()], 0)? == Some(0) {
                    session.refresh();
                } else {
                    stack.pop();
                }
                continue;
            }
        };
        let mut labels: Vec<_> = children
            .iter()
            .map(|item| {
                format!(
                    "{} · {}",
                    match kind(item) {
                        Kind::Folder => "Folder",
                        Kind::Document => "Document",
                        Kind::File => "File",
                        Kind::Link => "Link",
                        Kind::Tool => "Web tool",
                    },
                    label(title(item))
                )
            })
            .collect();
        let count = labels.len();
        if current == "ROOT" {
            labels.push("Announcements".into());
        }
        labels.extend([
            "Export this section".into(),
            "Refresh".into(),
            "Back".into(),
        ]);
        let breadcrumb = stack
            .iter()
            .map(|f| label(&f.title))
            .collect::<Vec<_>>()
            .join(" / ");
        if children.is_empty() {
            println!("This section is empty, or has no released items.");
        }
        let Some(choice) = choose(&breadcrumb, &labels, frame.selected)? else {
            stack.pop();
            continue;
        };
        stack.last_mut().unwrap().selected = choice;
        let result = if choice < count {
            let item = &children[choice];
            if kind(item) == Kind::Folder {
                let id = item["id"].as_str().context("Missing folder ID")?;
                if stack.iter().any(|f| f.id == id) {
                    eprintln!("This link points to a folder already open.");
                } else {
                    stack.push(Frame {
                        id: id.into(),
                        title: title(item).into(),
                        selected: 0,
                    });
                }
                Ok(())
            } else {
                match session.page(item) {
                    Ok(page) => session.view_page(&page),
                    Err(error) => Err(error),
                }
            }
        } else if current == "ROOT" && choice == count {
            session.view_announcements()
        } else {
            match choice - count - usize::from(current == "ROOT") {
                0 => session.export_section(&current, &stack.last().unwrap().title),
                1 => {
                    session.refresh();
                    Ok(())
                }
                _ => {
                    stack.pop();
                    Ok(())
                }
            }
        };
        if let Err(error) = result {
            eprintln!("Could not load item: {error:#}");
        }
    }
    Ok(())
}

struct Page {
    id: String,
    title: String,
    source: String,
    markdown: String,
    has_text: bool,
    files: Vec<Attachment>,
    links: Vec<(String, String)>,
    version: Option<i64>,
    resources: Vec<Resource>,
}
impl Page {
    fn from_items(item: &Value, children: &[Value], course: &str) -> Result<Self> {
        let parent = resource(item, course, "")?;
        let mut resources = vec![parent];
        for child in children {
            resources.push(resource(child, course, title(item))?);
        }
        let mut page = Self::from_resources(resources);
        if matches!(kind(item), Kind::Tool | Kind::Link)
            && page.links.is_empty()
            && page.files.is_empty()
        {
            page.links
                .push(("Blackboard website".into(), page.source.clone()));
        }
        Ok(page)
    }
    fn from_resources(resources: Vec<Resource>) -> Self {
        let parent = &resources[0];
        let mut page = Self {
            id: parent.id.clone(),
            title: parent.title.clone(),
            source: parent.source.clone(),
            markdown: String::new(),
            has_text: false,
            files: Vec::new(),
            links: Vec::new(),
            version: None,
            resources: Vec::new(),
        };
        let mut files = HashSet::new();
        let mut links = HashSet::new();
        for resource in &resources {
            let modified = resource.metadata["modifiedDate"].as_i64();
            page.version = page.version.max(modified);
            if resource.kind == "announcement" {
                page.version = None;
            }
            page.markdown.push_str(&html2md::parse_html(&resource.html));
            page.markdown.push_str("\n\n");
            let doc = Html::parse_fragment(&resource.html);
            let text = doc.root_element().text().collect::<String>();
            let mut text = text;
            for file in &resource.files {
                text = text.replace(&file.name, "");
            }
            page.has_text |= !text.trim().is_empty();
            for file in &resource.files {
                if files.insert(file.url.clone()) {
                    page.files.push(file.clone());
                }
            }
            for anchor in doc.select(&Selector::parse("a[href]").unwrap()) {
                let href = anchor.value().attr("href").unwrap_or("");
                if let Ok(url) = site().join(href)
                    && url.scheme() == "https"
                    && !files.contains(url.as_str())
                    && links.insert(url.to_string())
                {
                    let text = anchor.text().collect::<String>();
                    page.links.push((
                        if text.trim().is_empty() {
                            url.host_str().unwrap_or("Link").into()
                        } else {
                            text
                        },
                        url.into(),
                    ));
                }
            }
            if let Some(url) =
                resource.metadata["contentDetail"]["resource/x-bb-externallink"]["url"].as_str()
                && let Ok(url) = Url::parse(url)
                && url.scheme() == "https"
                && !files.contains(url.as_str())
                && links.insert(url.to_string())
            {
                page.links.push((resource.title.clone(), url.into()));
            }
        }
        // Attachments may first appear as a link in another block of the same document.
        page.links.retain(|(_, url)| !files.contains(url));
        page.resources = resources;
        page
    }
}

fn file_path(
    root: &Path,
    course: &str,
    item: &str,
    version: Option<i64>,
    index: usize,
    name: &str,
) -> PathBuf {
    root.join(safe(course))
        .join(safe(item))
        .join(version.map_or_else(|| "unversioned".into(), |v| v.to_string()))
        .join(format!("{:02}-{}", index + 1, safe(name)))
}
fn viewer_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "pdf"
            | "ppt"
            | "pptx"
            | "odp"
            | "doc"
            | "docx"
            | "odt"
            | "xls"
            | "xlsx"
            | "ods"
            | "png"
            | "jpg"
            | "jpeg"
            | "webp"
            | "gif"
            | "zip"
            | "mp3"
            | "mp4"
    )
}
fn open(target: &str) -> Result<()> {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let mut child = Command::new(program)
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("Could not launch the default viewer; use the saved path instead")?;
    thread::spawn(move || {
        if let Ok(status) = child.wait()
            && !status.success()
        {
            eprintln!("Default viewer could not open the item; use its saved path or source link.");
        }
    });
    Ok(())
}

fn rendered(markdown: &str, width: usize) -> String {
    let skin = if std::env::var_os("NO_COLOR").is_some() {
        termimad::MadSkin::no_style()
    } else {
        termimad::MadSkin::default()
    };
    skin.text(&clean(markdown), Some(width.max(20))).to_string()
}
fn read_markdown(title: &str, markdown: &str) -> Result<()> {
    read_text(title, markdown, &[]).map(|_| ())
}
fn read_text(title: &str, markdown: &str, extras: &[String]) -> Result<Option<usize>> {
    let (height, width) = Term::stdout().size();
    let text = rendered(markdown, usize::from(width).saturating_sub(2));
    let lines: Vec<_> = text.lines().collect();
    let pages: Vec<_> = lines
        .chunks(usize::from(height).saturating_sub(9).max(4))
        .collect();
    let mut page = 0;
    loop {
        println!(
            "\n{} · page {}/{}\n",
            label(title),
            page + 1,
            pages.len().max(1)
        );
        if let Some(lines) = pages.get(page) {
            println!("{}", lines.join("\n"));
        }
        let mut actions = Vec::new();
        if page + 1 < pages.len() {
            actions.push("Next page".into());
        }
        if page > 0 {
            actions.push("Previous page".into());
        }
        let extra_start = actions.len();
        actions.extend_from_slice(extras);
        actions.push("Back".into());
        let default = if page + 1 < pages.len() {
            0
        } else {
            actions.len() - 1
        };
        let Some(choice) = choose("Read", &actions, default)? else {
            return Ok(None);
        };
        if choice >= extra_start && choice < extra_start + extras.len() {
            return Ok(Some(choice - extra_start));
        }
        match actions[choice].as_str() {
            "Next page" => page += 1,
            "Previous page" => page -= 1,
            _ => return Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_text_cannot_emit_source_control_sequences() {
        let text = clean("hi\x1b[31m\x07\r\u{202e}\ntext\tOK");
        assert_eq!(text, "hi[31m\ntext\tOK");
    }
}
