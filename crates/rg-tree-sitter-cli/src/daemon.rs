use rg_tree_sitter_core::{engine::SearchEngine, LanguageId, SemanticMatch};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

/// Request sent from CLI to daemon.
#[derive(Debug, Serialize, Deserialize)]
pub struct QueryRequest {
    pub command: String, // "define", "refs", or "filter"
    pub symbol: String,
    pub lang: LanguageId,
    pub dir: PathBuf,
}

/// Response from daemon to CLI.
#[derive(Debug, Serialize, Deserialize)]
pub struct QueryResponse {
    pub matches: Vec<SemanticMatch>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_size: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watch_enabled: Option<bool>,
}

pub async fn run_daemon(
    socket_path: &std::path::Path,
    dir: &std::path::Path,
    watch: bool,
) -> anyhow::Result<()> {
    // Remove old socket if it exists
    let _ = std::fs::remove_file(socket_path);

    let listener = UnixListener::bind(socket_path)?;
    println!("Daemon listening on {}", socket_path.display());
    println!("Project directory: {}", dir.display());

    let engine = Arc::new(SearchEngine::new(2048).with_watch(watch));
    let project_dir = dir.to_path_buf();

    // Start file watcher if requested
    if watch {
        let engine_clone = Arc::clone(&engine);
        let watch_dir = project_dir.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = run_watcher(&watch_dir, engine_clone) {
                eprintln!("Watcher error: {}", e);
            }
        });
    }

    loop {
        let (stream, _) = listener.accept().await?;
        let engine = Arc::clone(&engine);
        let project_dir = project_dir.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_client(stream, engine, project_dir).await {
                eprintln!("Client error: {}", e);
            }
        });
    }
}

async fn handle_client(
    stream: UnixStream,
    engine: Arc<SearchEngine>,
    project_dir: PathBuf,
) -> anyhow::Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut buf_reader = BufReader::new(reader);
    let mut line = String::new();
    buf_reader.read_line(&mut line).await?;

    let req: QueryRequest = serde_json::from_str(&line)?;
    let resp = match req.command.as_str() {
        "define" => {
            match engine.find_definitions(&req.symbol, &project_dir, req.lang) {
                Ok(matches) => QueryResponse { matches, cache_size: None, watch_enabled: None },
                Err(e) => {
                    eprintln!("Query error: {}", e);
                    QueryResponse { matches: vec![], cache_size: None, watch_enabled: None }
                }
            }
        }
        "refs" => {
            match engine.find_references(&req.symbol, &project_dir, req.lang) {
                Ok(matches) => QueryResponse { matches, cache_size: None, watch_enabled: None },
                Err(e) => {
                    eprintln!("Query error: {}", e);
                    QueryResponse { matches: vec![], cache_size: None, watch_enabled: None }
                }
            }
        }
        "filter" => {
            use std::io::Cursor;
            let input = Cursor::new(req.symbol);
            let matches = match rg_tree_sitter_core::parse_external_matches(input) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("Parse error: {}", e);
                    vec![]
                }
            };
            let mut filter = match rg_tree_sitter_core::AstFilter::new_with_cache(req.lang, Some(engine.cache())) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("Filter init error: {}", e);
                    return Ok(());
                }
            };
            QueryResponse {
                matches: filter.filter_definitions(&matches),
                cache_size: None,
                watch_enabled: None,
            }
        }
        "status" => QueryResponse {
            matches: vec![],
            cache_size: Some(engine.cache_len()),
            watch_enabled: Some(engine.watch_enabled()),
        },
        _ => {
            eprintln!("Unknown command: {}", req.command);
            QueryResponse { matches: vec![], cache_size: None, watch_enabled: None }
        }
    };

    let resp_json = serde_json::to_string(&resp)?;
    writer.write_all(resp_json.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.shutdown().await?;
    Ok(())
}

fn run_watcher(dir: &std::path::Path, engine: Arc<SearchEngine>) -> anyhow::Result<()> {
    use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
    use std::collections::HashSet;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    let (tx, rx) = channel::<Result<Event, notify::Error>>();
    let mut watcher: RecommendedWatcher = Watcher::new(
        move |res| {
            let _ = tx.send(res);
        },
        Config::default(),
    )?;

    watcher.watch(dir, RecursiveMode::Recursive)?;

    let mut pending: HashSet<PathBuf> = HashSet::new();
    let debounce = Duration::from_millis(100);

    loop {
        match rx.recv_timeout(debounce) {
            Ok(Ok(event)) => {
                for path in event.paths {
                    if should_ignore_watcher_event(&path) {
                        continue;
                    }
                    pending.insert(path);
                }
            }
            Ok(Err(e)) => {
                eprintln!("Watch error: {}", e);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // Flush pending paths after debounce interval
                if !pending.is_empty() {
                    for path in pending.drain() {
                        engine.mark_dirty(path);
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                eprintln!("Watch channel disconnected");
                break;
            }
        }
    }

    Ok(())
}

/// Filter out irrelevant watcher events.
fn should_ignore_watcher_event(path: &std::path::Path) -> bool {
    // Ignore directories
    if path.is_dir() {
        return true;
    }

    // Ignore common non-source directories
    let path_str = path.to_string_lossy();
    for ignored in &["/.", "/target/", "/node_modules/", "/build/", "/dist/", "/.git/"] {
        if path_str.contains(ignored) {
            return true;
        }
    }

    // Only care about source file extensions
    let is_source = matches!(
        path.extension().and_then(|s| s.to_str()),
        Some("c" | "h" | "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" | "py" | "rs" | "go" | "js" | "ts" | "java")
    );
    if !is_source {
        return true;
    }

    false
}

pub fn run_daemon_status(socket_path: &std::path::Path) -> anyhow::Result<()> {
    use std::io::{BufRead, Write};
    use std::os::unix::net::UnixStream;

    let mut stream = UnixStream::connect(socket_path)?;
    let req = QueryRequest {
        command: "status".to_string(),
        symbol: "".to_string(),
        lang: LanguageId::Cpp,
        dir: PathBuf::from("."),
    };
    let req_json = serde_json::to_string(&req)?;
    stream.write_all(req_json.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.shutdown(std::net::Shutdown::Write)?;

    let reader = std::io::BufReader::new(&stream);
    let resp_line = reader.lines().next().transpose()?.unwrap_or_default();
    let resp: QueryResponse = serde_json::from_str(&resp_line)?;

    println!("rg-tree-sitter daemon status");
    println!("----------------------------");
    if let Some(size) = resp.cache_size {
        println!("Cache entries: {}", size);
    }
    if let Some(watch) = resp.watch_enabled {
        println!("File watcher: {}", if watch { "enabled" } else { "disabled" });
    }
    Ok(())
}

pub fn run_daemon_stop(socket_path: &std::path::Path) -> anyhow::Result<()> {
    if std::fs::remove_file(socket_path).is_ok() {
        println!("Daemon socket removed: {}", socket_path.display());
    } else {
        println!("No daemon socket found at {}", socket_path.display());
    }
    Ok(())
}
