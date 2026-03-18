use std::{
    collections::HashMap,
    env,
    fs,
    io::{ prelude::*, BufRead, BufReader, Cursor },
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

    if method != "GET" {
        respond_with_status(
            &mut stream,
            "405 Method Not Allowed",
            "Method not allowed",
            "text/plain"
        );
        return;
    }

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
        let data = fs::read(&canonical_path).unwrap_or_default();
        let file_name = canonical_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file");
        let status_line = "HTTP/1.1 200 OK";
        let content_disp = format!("attachment; filename=\"{}\"", file_name);
        let response = format!(
            "{status_line}\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nContent-Disposition: {}\r\n\r\n",
            data.len(),
            content_disp
        );

        stream.write_all(response.as_bytes()).unwrap();
        stream.write_all(&data).unwrap();
        return;
    }

    respond_with_status(&mut stream, "404 Not Found", "Not found", "text/plain");
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
    html.push_str("</style></head><body><div class=\"container\"> ");
    html.push_str("<h1>Directory listing</h1>");
    html.push_str("<div class=\"path\">");
    html.push_str(&abs_dir.display().to_string());
    html.push_str("</div>");
    html.push_str("<ul>");
    html.push_str(&items.join(""));
    html.push_str("</ul>");
    html.push_str(
        "<div class=\"note\">Right-click an item to download the folder as an archive.</div>"
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
    html.push_str("    const btn = event.target.closest('button[data-format]');");
    html.push_str("    if (!btn) return;");
    html.push_str("    const format = btn.getAttribute('data-format');");
    html.push_str("    if (!format) return;");
    html.push_str(
        "    const url = `/download?path=${encodeURIComponent(currentPath)}&format=${encodeURIComponent(format)}`;"
    );
    html.push_str("    window.location.href = url;");
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
            if let Ok(bytes) = archive_to_zip(&canonical_path) {
                respond_with_bytes(stream, &bytes, &format!("{}.zip", name), "application/zip");
            } else {
                respond_with_status(
                    stream,
                    "500 Internal Server Error",
                    "Failed to create zip",
                    "text/plain"
                );
            }
        }
        "tar.gz" | "tgz" => {
            if let Ok(bytes) = archive_to_targz(&canonical_path) {
                respond_with_bytes(stream, &bytes, &format!("{}.tar.gz", name), "application/gzip");
            } else {
                respond_with_status(
                    stream,
                    "500 Internal Server Error",
                    "Failed to create tar.gz",
                    "text/plain"
                );
            }
        }
        "7z" => {
            match archive_to_7z(&canonical_path) {
                Ok(bytes) => {
                    respond_with_bytes(
                        stream,
                        &bytes,
                        &format!("{}.7z", name),
                        "application/x-7z-compressed"
                    );
                }
                Err(_) => {
                    respond_with_status(
                        stream,
                        "500 Internal Server Error",
                        "Failed to create 7z archive (requires 7z binary)",
                        "text/plain"
                    );
                }
            }
        }
        _ => {
            respond_with_status(stream, "400 Bad Request", "Unknown archive format", "text/plain");
        }
    }
}

fn respond_with_bytes(stream: &mut TcpStream, bytes: &[u8], filename: &str, content_type: &str) {
    let status_line = "HTTP/1.1 200 OK";
    let content_disp = format!("attachment; filename=\"{}\"", filename);
    let response = format!(
        "{status_line}\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nContent-Disposition: {content_disp}\r\n\r\n",
        bytes.len()
    );
    stream.write_all(response.as_bytes()).unwrap();
    stream.write_all(bytes).unwrap();
}

fn archive_to_zip(dir: &Path) -> std::io::Result<Vec<u8>> {
    let mut buffer = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut buffer);
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
    }

    Ok(buffer.into_inner())
}

fn archive_to_targz(dir: &Path) -> std::io::Result<Vec<u8>> {
    let mut buffer = Vec::new();
    {
        let enc = GzEncoder::new(&mut buffer, Compression::default());
        let mut tar = Builder::new(enc);
        tar.append_dir_all(".", dir)?;
        tar.finish()?;
    }
    Ok(buffer)
}

fn archive_to_7z(dir: &Path) -> std::io::Result<Vec<u8>> {
    let mut out_path = std::env::temp_dir();
    let stamp = std::time::SystemTime
        ::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    out_path.push(format!("ls_web_{}.7z", stamp));

    let status = Command::new("7z")
        .arg("a")
        .arg("-t7z")
        .arg(out_path.to_string_lossy().as_ref())
        .arg(dir.to_string_lossy().as_ref())
        .status()?;

    if !status.success() {
        return Err(std::io::Error::new(std::io::ErrorKind::Other, "7z command failed"));
    }

    let bytes = fs::read(&out_path)?;
    let _ = fs::remove_file(&out_path);
    Ok(bytes)
}
fn respond_with_status(stream: &mut TcpStream, status: &str, body: &str, content_type: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).unwrap();
}
