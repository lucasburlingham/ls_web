use std::{
    collections::HashMap,
    env,
    fs,
    io::{ prelude::*, BufRead, BufReader },
    net::{ TcpListener, TcpStream },
    path::Path,
    process::Command,
};

use colored::Colorize;
use flate2::write::GzEncoder;
use flate2::Compression;
use tar::Builder;
use urlencoding::decode;
use walkdir::WalkDir;
use zip::write::FileOptions;
use zip::ZipWriter;

fn main() {
    let (host, port, serve_dir, sort_dirs_first) = parse_args();
    let address = format!("{host}:{port}");

    let listener = TcpListener::bind(&address).unwrap();
    println!("{}{}", "Server started at: http://".green().bold(), address.green().bold());
    println!("Serving directory: {}", serve_dir.green().bold());
    println!("Type {} to stop the server...", "CTRL+C".red().bold());

    for stream in listener.incoming() {
        let stream = stream.unwrap();
        println!(
            "Connection established from {}",
            stream.peer_addr().unwrap().to_string().blue().bold()
        );

        handle_connection(stream, &serve_dir, sort_dirs_first);
    }

    println!("Shutting down server...");
}

fn parse_args() -> (String, u16, String, bool) {
    let mut host = "0.0.0.0".to_string();
    let mut port = 7878u16;
    let mut dir = ".".to_string();
    let mut sort_dirs_first = false;
    let mut args = env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--host" | "-h" => {
                if let Some(value) = args.next() {
                    host = value;
                } else {
                    eprintln!("Missing value for {arg}");
                    std::process::exit(1);
                }
            }
            "--port" | "-p" => {
                if let Some(value) = args.next() {
                    port = value.parse().unwrap_or_else(|_| {
                        eprintln!("Invalid port: {value}");
                        std::process::exit(1);
                    });
                } else {
                    eprintln!("Missing value for {arg}");
                    std::process::exit(1);
                }
            }
            "--dir" | "-d" => {
                if let Some(value) = args.next() {
                    dir = value;
                } else {
                    eprintln!("Missing value for {arg}");
                    std::process::exit(1);
                }
            }
            _ if arg.starts_with("--host=") => {
                host = arg[7..].to_string();
            }
            _ if arg.starts_with("--port=") => {
                port = arg[7..].parse().unwrap_or_else(|_| {
                    eprintln!("Invalid port: {arg}");
                    std::process::exit(1);
                });
            }
            _ if arg.starts_with("--dir=") => {
                dir = arg[6..].to_string();
            }
            "--sort-dirs-first" => {
                sort_dirs_first = true;
            }
            _ => {
                eprintln!("Unknown argument: {arg}");
                eprintln!(
                    "Usage: ls_web [--host HOST] [--port PORT] [--dir DIR] [--sort-dirs-first]"
                );
                std::process::exit(1);
            }
        }
    }

    (host, port, dir, sort_dirs_first)
}

fn handle_connection(mut stream: TcpStream, serve_dir: &str, sort_dirs_first: bool) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }

    let request_line = request_line.trim_end();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let raw_path = parts.next().unwrap_or("/");

    // Read headers
    let mut headers = HashMap::new();
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            headers.insert(key.to_lowercase(), value.trim().to_string());
        }
    }

    let mut body = Vec::new();
    if method == "POST" {
        if let Some(cl) = headers.get("content-length") {
            if let Ok(len) = cl.trim().parse::<usize>() {
                body.resize(len, 0);
                let _ = reader.read_exact(&mut body);
            }
        }
    }

    if method == "GET" {
        let mut parts = raw_path.splitn(2, '?');
        let request_path = parts.next().unwrap_or("/");
        let query = parts.next();

        let request_path = if request_path.starts_with('/') {
            &request_path[1..]
        } else {
            request_path
        };

        let request_path = decode(request_path).unwrap_or_else(|_| request_path.into());

        let base_dir = Path::new(serve_dir);
        let canonical_base = fs::canonicalize(base_dir).unwrap_or_else(|_| base_dir.to_path_buf());

        // Download endpoint
        if request_path.starts_with("download") {
            handle_download(&mut stream, &canonical_base, query);
            return;
        }



        let candidate = base_dir.join(&*request_path);
        let canonical_path = fs::canonicalize(&candidate).unwrap_or_else(|_| candidate.clone());

        if !canonical_path.starts_with(&canonical_base) {
            respond_with_status(&mut stream, "403 Forbidden", "Forbidden", "text/plain");
            return;
        }

        if canonical_path.is_dir() {
            let request_url = if raw_path == "/" { "/" } else { raw_path };
            let html = render_directory_listing(
                &canonical_base,
                &canonical_path,
                request_url,
                sort_dirs_first,
            );
            respond_with_status(&mut stream, "200 OK", &html, "text/html; charset=utf-8");
            return;
        }

        if canonical_path.is_file() {
            let file_name = canonical_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file");

            if let Err(_) = respond_with_file_stream(
                &mut stream,
                &canonical_path,
                file_name,
                "application/octet-stream",
            ) {
                respond_with_status(&mut stream, "500 Internal Server Error", "Failed to read file", "text/plain");
            }

            return;
        }

        respond_with_status(&mut stream, "404 Not Found", "Not found", "text/plain");
        return;
    }

    if method == "POST" {
        let mut parts = raw_path.splitn(2, '?');
        let request_path = parts.next().unwrap_or("/");
        let query = parts.next();

        let request_path = if request_path.starts_with('/') {
            &request_path[1..]
        } else {
            request_path
        };

        let request_path = decode(request_path).unwrap_or_else(|_| request_path.into());

        let base_dir = Path::new(serve_dir);
        let canonical_base = fs::canonicalize(base_dir).unwrap_or_else(|_| base_dir.to_path_buf());

        if request_path.starts_with("upload") {
            handle_upload(&mut stream, &canonical_base, query, &body, headers.get("content-type").map(|s| s.as_str()));
            return;
        }

        respond_with_status(&mut stream, "405 Method Not Allowed", "Method not allowed", "text/plain");
        return;
    }

    respond_with_status(&mut stream, "405 Method Not Allowed", "Method not allowed", "text/plain");
}

fn render_directory_listing(
    _base_dir: &Path,
    dir: &Path,
    request_url: &str,
    sort_dirs_first: bool
) -> String {
    let abs_dir = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let mut entries: Vec<_> = fs
        ::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();

    if sort_dirs_first {
        entries.sort_by_key(|e| {
            let is_file = e
                .file_type()
                .map(|t| t.is_file())
                .unwrap_or(true);
            (is_file, e.file_name())
        });
    } else {
        entries.sort_by_key(|e| e.file_name());
    }

    let mut items = Vec::new();

    if request_url != "/" {
        let parent = Path::new(request_url).parent().unwrap_or(Path::new("/"));
        let parent_url = if parent == Path::new("") { "/" } else { parent.to_str().unwrap_or("/") };
        items.push(format!("<li><a href=\"{}\">..</a></li>", parent_url));
    }

    for entry in entries {
        let file_type = entry.file_type().unwrap();
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if name == "ls_web" {
            continue;
        }

        let is_dir = file_type.is_dir();
        let display_name = if is_dir { format!("{}/", name) } else { name.to_string() };

        let mut href = if request_url.ends_with('/') {
            format!("{}{}", request_url, urlencoding::encode(&name))
        } else if request_url == "/" {
            format!("/{}", urlencoding::encode(&name))
        } else {
            format!("{}/{}", request_url, urlencoding::encode(&name))
        };

        if is_dir && !href.ends_with('/') {
            href.push('/');
        }

        items.push(
            format!("<li data-path=\"{}\"><a href=\"{}\">{}</a></li>", href, href, display_name)
        );
    }

    let mut html = String::new();
    html.push_str(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>Directory listing</title><style>"
    );
    html.push_str(
        ":root {\n  --bg: #fff;\n  --fg: #0b101a;\n  --panel: rgba(255, 255, 255, 0.9);\n  --border: rgba(22, 24, 32, 0.75);\n  --muted: rgba(0, 0, 0, 0.55);\n  --link: #0066cc;\n  --link-hover: #004a99;\n  --shadow: rgba(0, 0, 0, 0.12);\n}\n"
    );
    html.push_str(
        "body {\n  font-family: system-ui, -apple-system, BlinkMacSystemFont, Segoe UI, Roboto, Ubuntu, sans-serif;\n  margin: 2rem;\n  padding: 0;\n  background: var(--bg);\n  color: var(--fg);\n}\n"
    );
    html.push_str(
        ".container {\n  max-width: 920px;\n  margin: 2.5rem auto;\n  padding: 1.75rem 1.75rem 2rem;\n  background: var(--panel);\n  border: 1px solid var(--border);\n  border-radius: 14px;\n  box-shadow: 0 18px 40px var(--shadow);\n}\n"
    );
    html.push_str(
        "h1 {\n  margin: 0 0 0.35rem;\n  font-size: 1.7rem;\n  letter-spacing: -0.03em;\n}\n"
    );
    html.push_str(
        ".path {\n  color: var(--muted);\n  margin-bottom: 1.1rem;\n  font-size: 0.92rem;\n}\n"
    );
    html.push_str("ul {\n  list-style: none;\n  margin: 0;\n  padding: 0;\n}\n");
    html.push_str("li {\n  margin: 0.45rem 0;\n}\n");
    html.push_str(
        "li a {\n  display: block;\n  padding: 0.35rem 0.55rem;\n  border-radius: 8px;\n  transition: background 120ms ease-in-out;\n  color: var(--link);\n  text-decoration: none;\n  font-weight: 500;\n}\n"
    );
    html.push_str(
        "li a:hover {\n  background: rgba(0, 0, 0, 0.04);\n  color: var(--link-hover);\n}\n"
    );
    html.push_str(
        ".context-menu {\n  position: absolute;\n  display: none;\n  min-width: 200px;\n  padding: 0.25rem 0;\n  background: var(--panel);\n  border: 1px solid var(--border);\n  border-radius: 10px;\n  box-shadow: 0 12px 24px rgba(0, 0, 0, 0.18);\n  z-index: 1000;\n}\n"
    );
    html.push_str(
        ".context-menu button {\n  width: 100%;\n  padding: 0.55rem 1rem;\n  border: none;\n  background: transparent;\n  color: var(--fg);\n  text-align: left;\n  cursor: pointer;\n  font-size: 0.95rem;\n}\n"
    );
    html.push_str(".context-menu button:hover {\n  background: rgba(0, 0, 0, 0.06);\n}\n");
    html.push_str(
        ".note {\n  margin-top: 1.8rem;\n  color: var(--muted);\n  font-size: 0.88rem;\n}\n"
    );
    html.push_str(
        ".dropzone {\n  border: 2px dashed rgba(0,0,0,0.2);\n  border-radius: 14px;\n  padding: 1.5rem;\n  margin-bottom: 1.5rem;\n  text-align: center;\n  color: rgba(0,0,0,0.55);\n  transition: background 120ms ease, border-color 120ms ease;\n}\n"
    );
    html.push_str(
        ".dropzone.active {\n  background: rgba(0, 102, 204, 0.08);\n  border-color: rgba(0, 102, 204, 0.6);\n}\n"
    );
    html.push_str("</style></head><body><div class=\"container\"> ");
    html.push_str("<h1>Directory listing</h1>");
    html.push_str("<div class=\"path\">");
    html.push_str(&abs_dir.display().to_string());
    html.push_str("</div>");

    // Upload form
    let encoded_path = urlencoding::encode(request_url);
    let upload_url = format!("/upload?path={}", encoded_path);
    html.push_str(&format!(
        "<form action=\"{}\" method=\"post\" enctype=\"multipart/form-data\" style=\"margin-bottom: 1.25rem;\">",
        upload_url
    ));
    html.push_str("<label style=\"font-weight: 600; margin-right: 0.5rem;\">Upload file: <input type=\"file\" name=\"file\" multiple></label>");
    html.push_str("<button type=\"submit\" style=\"padding: 0.45rem 0.9rem; border-radius: 8px; border: 1px solid rgba(0,0,0,0.15); background: rgba(0,0,0,0.04); cursor: pointer;\">Upload</button>");
    html.push_str("</form>");

    html.push_str("<div id=\"dropZone\" class=\"dropzone\">Drag files here to upload</div>");

    html.push_str("<ul>");
    html.push_str(&items.join(""));
    html.push_str("</ul>");
    html.push_str(
        "<div class=\"note\">Right-click an item to download as an archive.</div>"
    );
    html.push_str("<div id=\"contextMenu\" class=\"context-menu\">");
    html.push_str("<button data-format=\"zip\">Download ZIP</button>");
    html.push_str("<button data-format=\"tar.gz\">Download tar.gz</button>");
    html.push_str("<button data-format=\"7z\">Download 7z</button>");
    html.push_str("</div>");
    html.push_str("</div><script>");
    html.push_str("(function() {");
    html.push_str("  const menu = document.getElementById('contextMenu');");
    html.push_str("  let currentPath = ''; ");
    html.push_str("  function hideMenu() { menu.style.display = 'none'; }");
    html.push_str(
        "  function showMenu(x, y) { menu.style.left = `${x}px`; menu.style.top = `${y}px`; menu.style.display = 'block'; }"
    );
    html.push_str("  document.addEventListener('click', () => hideMenu());");
    html.push_str("  document.addEventListener('contextmenu', (event) => {");
    html.push_str("    const target = event.target.closest('li[data-path]');");
    html.push_str("    if (!target) return;");
    html.push_str("    event.preventDefault();");
    html.push_str("    currentPath = target.getAttribute('data-path') || ''; ");
    html.push_str("    showMenu(event.pageX, event.pageY);");
    html.push_str("  });");
    html.push_str("  menu.addEventListener('click', (event) => {");
    html.push_str("    event.preventDefault();");
    html.push_str("    event.stopPropagation();");
    html.push_str("    const btn = event.target.closest('button');");
    html.push_str("    if (!btn) return;");
    html.push_str("    const format = btn.getAttribute('data-format');");
    html.push_str("    if (!format) return;");
    html.push_str("    const url = new URL('/download', window.location.origin);\n");
    html.push_str("    url.searchParams.set('path', currentPath);\n");
    html.push_str("    url.searchParams.set('format', format);\n");
    html.push_str("    window.location.href = url;\n");
    html.push_str("  });");

    // Drag-and-drop upload
    html.push_str("  const dropZone = document.getElementById('dropZone');");
    html.push_str(&format!(
        "  const uploadUrl = \"{}\";",
        upload_url
    ));
    html.push_str("  function setDropActive(active) { dropZone.classList.toggle('active', active); }");
    html.push_str("  document.addEventListener('dragover', (e) => { e.preventDefault(); setDropActive(true); });");
    html.push_str("  document.addEventListener('dragleave', (e) => { if (!e.relatedTarget || !dropZone.contains(e.relatedTarget)) setDropActive(false); });");
    html.push_str("  document.addEventListener('drop', async (e) => {");
    html.push_str("    e.preventDefault();");
    html.push_str("    setDropActive(false);");
    html.push_str("    const dt = e.dataTransfer;");
    html.push_str("    if (!dt || !dt.files || dt.files.length === 0) return;");
    html.push_str("    const form = new FormData();");
    html.push_str("    for (const file of dt.files) form.append('file', file);");
    html.push_str("    await fetch(uploadUrl, { method: 'POST', body: form });");
    html.push_str("    window.location.reload();");
    html.push_str("  });");

    html.push_str("})();");
    html.push_str("</script></body></html>");

    html
}
fn parse_query(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter_map(|pair| {
            let mut parts = pair.splitn(2, '=');
            let key = parts.next()?;
            let value = parts.next().unwrap_or("");
            let decoded = decode(value)
                .unwrap_or_else(|_| value.into())
                .into_owned();
            Some((key.to_string(), decoded))
        })
        .collect()
}

fn handle_download(stream: &mut TcpStream, base_dir: &Path, query: Option<&str>) {
    let params = parse_query(query.unwrap_or(""));
    let format = params
        .get("format")
        .map(|s| s.as_str())
        .unwrap_or("zip");
    let request_path = params
        .get("path")
        .map(|s| s.as_str())
        .unwrap_or("");

    let request_path = request_path.trim_start_matches('/');
    let candidate = base_dir.join(request_path);
    let canonical_base = fs::canonicalize(base_dir).unwrap_or_else(|_| base_dir.to_path_buf());
    let canonical_path = fs::canonicalize(&candidate).unwrap_or_else(|_| candidate.clone());

    if !canonical_path.starts_with(&canonical_base) {
        respond_with_status(stream, "403 Forbidden", "Forbidden", "text/plain");
        return;
    }

    if !canonical_path.is_dir() {
        respond_with_status(stream, "400 Bad Request", "Path must be a directory", "text/plain");
        return;
    }

    let name = canonical_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("archive");

    match format {
        "zip" => {
            let mut out_path = std::env::temp_dir();
            let stamp = std::time::SystemTime
                ::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            out_path.push(format!("ls_web_{}.zip", stamp));

            if create_zip_archive(&canonical_path, &out_path).is_ok() {
                let _ = respond_with_file_stream(
                    stream,
                    &out_path,
                    &format!("{}.zip", name),
                    "application/zip",
                );
                let _ = fs::remove_file(&out_path);
            } else {
                respond_with_status(
                    stream,
                    "500 Internal Server Error",
                    "Failed to create zip",
                    "text/plain",
                );
            }
        }
        "tar.gz" | "tgz" => {
            let mut out_path = std::env::temp_dir();
            let stamp = std::time::SystemTime
                ::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            out_path.push(format!("ls_web_{}.tar.gz", stamp));

            if create_targz_archive(&canonical_path, &out_path).is_ok() {
                let _ = respond_with_file_stream(
                    stream,
                    &out_path,
                    &format!("{}.tar.gz", name),
                    "application/gzip",
                );
                let _ = fs::remove_file(&out_path);
            } else {
                respond_with_status(
                    stream,
                    "500 Internal Server Error",
                    "Failed to create tar.gz",
                    "text/plain",
                );
            }
        }
        "7z" => {
            let mut out_path = std::env::temp_dir();
            let stamp = std::time::SystemTime
                ::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            out_path.push(format!("ls_web_{}.7z", stamp));

            if create_7z_archive(&canonical_path, &out_path).is_ok() {
                let _ = respond_with_file_stream(
                    stream,
                    &out_path,
                    &format!("{}.7z", name),
                    "application/x-7z-compressed",
                );
                let _ = fs::remove_file(&out_path);
            } else {
                respond_with_status(
                    stream,
                    "500 Internal Server Error",
                    "Failed to create 7z archive (requires 7z binary)",
                    "text/plain",
                );
            }
        }
        _ => {
            respond_with_status(stream, "400 Bad Request", "Unknown archive format", "text/plain");
        }
    }
}

fn handle_upload(
    stream: &mut TcpStream,
    base_dir: &Path,
    query: Option<&str>,
    body: &[u8],
    content_type: Option<&str>,
) {
    let params = parse_query(query.unwrap_or(""));
    let request_path = params.get("path").map(|s| s.as_str()).unwrap_or("");

    let request_path = request_path.trim_start_matches('/');
    let target_dir = base_dir.join(request_path);

    let canonical_base = fs::canonicalize(base_dir).unwrap_or_else(|_| base_dir.to_path_buf());
    let canonical_target = fs::canonicalize(&target_dir).unwrap_or_else(|_| target_dir.clone());

    if !canonical_target.starts_with(&canonical_base) {
        respond_with_status(stream, "403 Forbidden", "Forbidden", "text/plain");
        return;
    }

    if let Err(e) = fs::create_dir_all(&canonical_target) {
        respond_with_status(
            stream,
            "500 Internal Server Error",
            &format!("Failed to create upload directory: {e}"),
            "text/plain",
        );
        return;
    }

    let boundary = content_type
        .and_then(|ct| ct.split(';').find_map(|part| {
            let part = part.trim();
            if part.starts_with("boundary=") {
                Some(part.trim_start_matches("boundary=").trim_matches('"').to_string())
            } else {
                None
            }
        }));

    let boundary = if let Some(b) = boundary {
        b
    } else {
        respond_with_status(
            stream,
            "400 Bad Request",
            "Missing multipart boundary",
            "text/plain",
        );
        return;
    };

    let files = parse_multipart(body, &boundary);
    if files.is_empty() {
        respond_with_status(
            stream,
            "400 Bad Request",
            "No files uploaded",
            "text/plain",
        );
        return;
    }

    for (filename, bytes) in files {
        let safe_name = Path::new(&filename)
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("upload.bin");
        let dest = canonical_target.join(safe_name);
        if let Err(e) = fs::write(&dest, &bytes) {
            respond_with_status(
                stream,
                "500 Internal Server Error",
                &format!("Failed to write file: {e}"),
                "text/plain",
            );
            return;
        }
    }

    let redirect_to = if request_path.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", request_path)
    };

    respond_with_redirect(stream, &redirect_to);
}

fn respond_with_redirect(stream: &mut TcpStream, location: &str) {
    let response = format!(
        "HTTP/1.1 303 See Other\r\nLocation: {location}\r\nContent-Length: 0\r\n\r\n"
    );
    stream.write_all(response.as_bytes()).unwrap();
}

fn parse_multipart(body: &[u8], boundary: &str) -> Vec<(String, Vec<u8>)> {
    let boundary = format!("--{}", boundary);
    let boundary_bytes = boundary.as_bytes();
    let mut parts = Vec::new();
    let mut idx = 0;

    while let Some(start) = find_subslice(&body[idx..], boundary_bytes) {
        idx += start + boundary_bytes.len();

        // Check for end boundary
        if body.get(idx..idx + 2) == Some(b"--") {
            break;
        }

        // Skip CRLF if present
        if body.get(idx..idx + 2) == Some(b"\r\n") {
            idx += 2;
        }

        // Read headers
        let mut headers = HashMap::new();
        while let Some(pos) = find_subslice(&body[idx..], b"\r\n") {
            if pos == 0 {
                idx += 2;
                break;
            }
            let line = &body[idx..idx + pos];
            if let Ok(line_str) = std::str::from_utf8(line) {
                if let Some((k, v)) = line_str.split_once(':') {
                    headers.insert(k.to_lowercase(), v.trim().to_string());
                }
            }
            idx += pos + 2;
        }

        // Find next boundary
        if let Some(next) = find_subslice(&body[idx..], boundary_bytes) {
            let mut data = body[idx..idx + next].to_vec();
            if data.ends_with(b"\r\n") {
                data.truncate(data.len() - 2);
            }

            if let Some(disposition) = headers.get("content-disposition") {
                if let Some(filename) = disposition
                    .split(';')
                    .find_map(|part| {
                        let part = part.trim();
                        if part.starts_with("filename=") {
                            Some(
                                part.trim_start_matches("filename=")
                                    .trim_matches('"')
                                    .to_string(),
                            )
                        } else {
                            None
                        }
                    })
                {
                    parts.push((filename, data));
                }
            }

            idx += next;
            continue;
        }

        break;
    }

    parts
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }

    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn respond_with_file_stream(
    stream: &mut TcpStream,
    file_path: &Path,
    filename: &str,
    content_type: &str,
) -> std::io::Result<()> {
    let mut file = fs::File::open(file_path)?;
    let len = file.metadata()?.len();

    let status_line = "HTTP/1.1 200 OK";
    let content_disp = format!("attachment; filename=\"{}\"", filename);
    let response = format!(
        "{status_line}\r\nContent-Length: {len}\r\nContent-Type: {content_type}\r\nContent-Disposition: {content_disp}\r\n\r\n"
    );

    stream.write_all(response.as_bytes())?;
    std::io::copy(&mut file, stream)?;
    Ok(())
}

fn create_zip_archive(dir: &Path, out_path: &Path) -> std::io::Result<()> {
    let file = fs::File::create(out_path)?;
    let mut zip = ZipWriter::new(file);
    let options = FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    for entry in WalkDir::new(dir) {
        let entry = entry?;
        let path = entry.path();
        let name = path.strip_prefix(dir).unwrap();
        if name.as_os_str().is_empty() {
            continue;
        }

        let name_str = name.to_string_lossy();
        if entry.file_type().is_dir() {
            zip.add_directory(name_str.to_string(), options)?;
        } else {
            zip.start_file(name_str.to_string(), options)?;
            let mut f = fs::File::open(path)?;
            std::io::copy(&mut f, &mut zip)?;
        }
    }

    zip.finish()?;
    Ok(())
}

fn create_targz_archive(dir: &Path, out_path: &Path) -> std::io::Result<()> {
    let file = fs::File::create(out_path)?;
    let enc = GzEncoder::new(file, Compression::default());
    let mut tar = Builder::new(enc);
    tar.append_dir_all(".", dir)?;
    tar.finish()?;
    Ok(())
}

fn create_7z_archive(dir: &Path, out_path: &Path) -> std::io::Result<()> {
    let status = Command::new("7z")
        .arg("a")
        .arg("-t7z")
        .arg(out_path.to_string_lossy().as_ref())
        .arg(dir.to_string_lossy().as_ref())
        .status()?;

    if !status.success() {
        return Err(std::io::Error::new(std::io::ErrorKind::Other, "7z command failed"));
    }

    Ok(())
}
fn respond_with_status(stream: &mut TcpStream, status: &str, body: &str, content_type: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).unwrap();
}
